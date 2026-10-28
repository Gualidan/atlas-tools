mod fixtures;

use std::{env::consts::ARCH, fs, io, path::PathBuf};

use anyhow::Result;
use atlas::{
    install::db::{InstallReason, ensure_install_schema},
    types::runtime_config::RuntimeConfig,
    uninstall::uninstall::uninstall,
};
use fixtures::{count, runtime_config, tempdir};
use rusqlite::{Connection, params};
use tempfile::TempDir;

struct Installation {
    tmp: TempDir,
    config: RuntimeConfig,
    conn: Connection,
}

impl Installation {
    fn new() -> Result<Self> {
        let tmp = tempdir();
        let config = runtime_config(&tmp)?;
        let conn = Connection::open(&config.install_db)?;
        ensure_install_schema(&conn)?;
        conn.execute_batch("PRAGMA foreign_keys = ON")?;
        Ok(Self { tmp, config, conn })
    }

    fn seed(&self, name: &str, install_reason: InstallReason) -> Result<PathBuf> {
        let dir = self.tmp.path().join(name);
        fs::create_dir(&dir)?;
        fs::write(dir.join("sentinel"), name)?;
        self.conn.execute(
            "INSERT INTO installed_packages
             (name, version, release, architecture, sky_path, install_dir, install_reason)
             VALUES (?, '1.0.0', 1, ?, 'unused.sky', ?, ?)",
            params![
                name,
                ARCH.to_string(),
                dir.to_str().unwrap(),
                install_reason.to_string()
            ],
        )?;
        self.conn.execute(
            "INSERT INTO owned_files VALUES (?, ?, ?)",
            params![format!("{name}/sentinel"), name, ARCH.to_string()],
        )?;
        Ok(dir)
    }

    fn assert_present(&self, name: &str) -> Result<()> {
        assert_eq!(
            fs::read(self.tmp.path().join(name).join("sentinel"))?,
            name.as_bytes()
        );
        assert_eq!(
            count(
                &self.conn,
                "SELECT COUNT(*) FROM installed_packages WHERE name = ?",
                &[&name]
            ),
            1
        );
        assert_eq!(
            count(
                &self.conn,
                "SELECT COUNT(*) FROM owned_files WHERE package_name = ?",
                &[&name]
            ),
            1
        );
        Ok(())
    }
}

#[test]
fn a_missing_install_directory_still_removes_the_ledger_and_preserves_neighbors() -> Result<()> {
    let fixture = Installation::new()?;
    let app_dir = fixture.seed("app", InstallReason::Explicit)?;
    fixture.seed("neighbor", InstallReason::Explicit)?;
    fs::remove_dir_all(&app_dir)?;

    uninstall(&fixture.config, &"app".into(), false)?;

    assert!(!app_dir.exists());
    assert_eq!(
        count(
            &fixture.conn,
            "SELECT COUNT(*) FROM installed_packages WHERE name = 'app'",
            &[]
        ),
        0
    );
    assert_eq!(
        count(
            &fixture.conn,
            "SELECT COUNT(*) FROM owned_files WHERE package_name = 'app'",
            &[]
        ),
        0
    );
    fixture.assert_present("neighbor")
}

#[test]
fn uninstall_removes_the_complete_dependency_chain_and_its_ledger_entries() -> Result<()> {
    let fixture = Installation::new()?;
    for name in ["app", "middle", "leaf", "neighbor"] {
        if name == "leaf" || name == "middle" {
            fixture.seed(name, InstallReason::Dependency)?;
        } else {
            fixture.seed(name, InstallReason::Explicit)?;
        }
    }
    for (name, dep) in [("app", "middle"), ("middle", "leaf")] {
        fixture.conn.execute(
            "INSERT INTO package_dependencies VALUES (?, ?, ?)",
            params![name, ARCH.to_string(), dep],
        )?;
    }

    uninstall(&fixture.config, &"app".into(), false)?;

    for name in ["app", "middle", "leaf"] {
        assert!(
            !fixture.tmp.path().join(name).exists(),
            "{name} must be removed"
        );
    }
    assert_eq!(
        count(
            &fixture.conn,
            "SELECT COUNT(*) FROM installed_packages",
            &[]
        ),
        1
    );
    assert_eq!(
        count(&fixture.conn, "SELECT COUNT(*) FROM owned_files", &[]),
        1
    );
    assert_eq!(
        count(
            &fixture.conn,
            "SELECT COUNT(*) FROM package_dependencies",
            &[]
        ),
        0
    );
    fixture.assert_present("neighbor")
}

#[test]
fn a_filesystem_error_preserves_the_install_record_and_ownership() -> Result<()> {
    let fixture = Installation::new()?;
    let dir = fixture.seed("app", InstallReason::Explicit)?;
    fs::remove_dir_all(&dir)?;
    fs::write(&dir, b"not a directory")?;
    let before = fs::read(&fixture.config.install_db)?;

    let err = uninstall(&fixture.config, &"app".into(), false)
        .expect_err("a regular file cannot be removed as a directory");

    let cause = err
        .downcast_ref::<io::Error>()
        .unwrap_or_else(|| panic!("must propagate the filesystem error, got {err:#}"));
    assert_ne!(cause.kind(), io::ErrorKind::NotFound);
    assert_eq!(fs::read(&dir)?, b"not a directory");
    assert_eq!(fs::read(&fixture.config.install_db)?, before);
    Ok(())
}

#[test]
fn dependents_are_rejected_before_confirmation_or_any_mutation() -> Result<()> {
    let fixture = Installation::new()?;
    fixture.seed("app", InstallReason::Explicit)?;
    fixture.seed("dependent", InstallReason::Explicit)?;
    fixture.conn.execute(
        "INSERT INTO package_dependencies VALUES ('dependent', ?, 'app')",
        [ARCH.to_string()],
    )?;
    let before = fs::read(&fixture.config.install_db)?;

    // A fresh test binary has assume_yes disabled. The dependency failure must
    // occur before the prompt even when confirmation was requested.
    let err = uninstall(&fixture.config, &"app".into(), true)
        .expect_err("dependent must block removal before confirmation");

    assert!(
        err.to_string().contains("dependent depends on it"),
        "{err:#}"
    );
    assert_eq!(fs::read(&fixture.config.install_db)?, before);
    fixture.assert_present("app")?;
    fixture.assert_present("dependent")
}

#[test]
fn an_absent_package_is_reported_by_name_without_changing_other_packages() -> Result<()> {
    let fixture = Installation::new()?;
    fixture.seed("neighbor", InstallReason::Explicit)?;
    let before = fs::read(&fixture.config.install_db)?;

    let err = uninstall(&fixture.config, &"missing".into(), false)
        .expect_err("an absent package must be reported");

    assert_eq!(err.to_string(), "Package `missing` not found");
    assert_eq!(fs::read(&fixture.config.install_db)?, before);
    fixture.assert_present("neighbor")
}
