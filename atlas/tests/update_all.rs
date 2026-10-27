mod fixtures;

use anyhow::Result;
use atlas::{
    cli::cli::UpdateArgs,
    install::{db::ensure_install_schema, install::install},
    types::runtime_config::RuntimeConfig,
    update::update::update,
};
use fixtures::{
    ARCH, ConfigEnv, count, generate_keypair, runtime_config, seed_repo_db, tempdir,
    write_signed_sky_with_file,
};
use ring::signature::Ed25519KeyPair;
use rusqlite::{Connection, params};
use std::{fs, path::PathBuf};
use tempfile::TempDir;

struct BulkUpdate {
    tmp: TempDir,
    config: RuntimeConfig,
    keypair: Ed25519KeyPair,
    _env: ConfigEnv,
}

impl BulkUpdate {
    fn new() -> Result<Self> {
        let tmp = tempdir();
        let config = runtime_config(&tmp)?;
        ensure_install_schema(&Connection::open(&config.install_db)?)?;
        seed_repo_db(&config.repo_db, &[]);
        let (keypair, public_key) = generate_keypair()?;
        let config_env = ConfigEnv::new(&tmp, &public_key)?;
        fs::create_dir(tmp.path().join("root"))?;
        Ok(Self {
            tmp,
            config,
            keypair,
            _env: config_env,
        })
    }

    fn conn(&self) -> Result<Connection> {
        Ok(Connection::open(&self.config.install_db)?)
    }

    fn repo(&self) -> Result<Connection> {
        Ok(Connection::open(&self.config.repo_db)?)
    }

    fn dir(&self, name: &str) -> PathBuf {
        self.tmp.path().join("root").join(format!("{name}-1.0.0-1"))
    }

    fn publish(&self, name: &str, version: &str, release: u32) -> Result<PathBuf> {
        let sky = write_signed_sky_with_file(
            &self.tmp,
            &self.keypair,
            &format!("{name}-{version}-{release}.sky"),
            name,
            version,
            release,
            format!("{name} {version}\n").as_bytes(),
            vec![],
            name,
            &format!("{version}.txt"),
        )?;
        seed_repo_db(
            &self.config.repo_db,
            &[(name, version, release, sky.to_str().unwrap())],
        );
        Ok(sky)
    }

    fn install(&self, name: &str) -> Result<()> {
        self.publish(name, "1.0.0", 1)?;
        install(
            &name.to_string(),
            &self.config,
            self.tmp.path().join("root"),
        )
    }

    fn update_all(&self) -> Result<()> {
        update(&self.config, UpdateArgs { package: None })
    }

    fn assert_installed(&self, name: &str, version: &str, release: u32) -> Result<()> {
        let record: (String, u32, String, String) = self.conn()?.query_row(
            "SELECT version, release, sky_path, install_dir FROM installed_packages
             WHERE name = ? AND architecture = ?",
            params![name, ARCH.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        assert_eq!(
            record,
            (
                version.into(),
                release,
                self.tmp
                    .path()
                    .join(format!("{name}-{version}-{release}.sky"))
                    .to_str()
                    .unwrap()
                    .into(),
                self.dir(name).to_str().unwrap().into(),
            ),
            "installed record for {name}"
        );
        let file = format!("{name}/{version}.txt");
        assert_eq!(
            fs::read(self.dir(name).join(&file))?,
            format!("{name} {version}\n").as_bytes(),
        );
        let mut files: Vec<_> = fs::read_dir(self.dir(name).join(name))?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<std::io::Result<_>>()?;
        files.sort();
        assert_eq!(
            files,
            vec![std::ffi::OsString::from(format!("{version}.txt"))]
        );
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare("SELECT path FROM owned_files WHERE package_name = ? ORDER BY path")?;
        let owned = stmt
            .query_map([name], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        assert_eq!(owned, vec![file]);
        Ok(())
    }

    fn assert_no_staging_dirs(&self, names: &[&str]) -> Result<()> {
        let mut actual: Vec<_> = fs::read_dir(self.tmp.path().join("root"))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<_>>()?;
        let mut expected: Vec<_> = names.iter().map(|name| self.dir(name)).collect();
        actual.sort();
        expected.sort();
        assert_eq!(
            actual, expected,
            "temporary staging directories must be removed"
        );
        Ok(())
    }
}

#[test]
fn an_empty_installed_list_succeeds_without_installing_repository_packages() -> Result<()> {
    let fixture = BulkUpdate::new()?;
    fixture.publish("uninstalled", "2.0.0", 7)?;

    fixture.update_all()?;

    assert_eq!(
        count(
            &fixture.conn()?,
            "SELECT COUNT(*) FROM installed_packages",
            &[]
        ),
        0
    );
    fixture.assert_no_staging_dirs(&[])
}

#[test]
fn bulk_update_upgrades_multiple_packages_and_continues_past_an_equal_version() -> Result<()> {
    let fixture = BulkUpdate::new()?;
    // Reverse insertion order also exercises collecting the installed rows before writes.
    for name in ["zeta", "middle", "alpha"] {
        fixture.install(name)?;
    }
    fixture.publish("alpha", "2.0.0", 7)?;
    fixture.publish("zeta", "3.0.0", 9)?;
    let skipped_archive = fixture.publish("middle", "1.0.0", 99)?;
    fs::remove_file(skipped_archive)?;
    fixture.publish("uninstalled", "2.0.0", 7)?;
    fixture.conn()?.execute_batch(
        "UPDATE installed_packages SET installed_at = '2000-01-01', install_reason = 'dependency'
         WHERE name = 'alpha';
         INSERT INTO package_dependencies VALUES ('middle', 'x86_64', 'alpha');",
    )?;

    fixture.update_all()?;

    fixture.assert_installed("alpha", "2.0.0", 7)?;
    fixture.assert_installed("middle", "1.0.0", 1)?;
    fixture.assert_installed("zeta", "3.0.0", 9)?;
    let conn = fixture.conn()?;
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM installed_packages", &[]),
        3
    );
    let retained: (String, String) = conn.query_row(
        "SELECT installed_at, install_reason FROM installed_packages WHERE name = 'alpha'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(retained, ("2000-01-01".into(), "dependency".into()));
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM package_dependencies WHERE package_name = 'middle' AND dependency_name = 'alpha'",
            &[]
        ),
        1
    );
    fixture.assert_no_staging_dirs(&["alpha", "middle", "zeta"])
}

#[test]
fn an_equal_version_ignores_release_changes_without_reading_the_archive() -> Result<()> {
    let fixture = BulkUpdate::new()?;
    fixture.install("foo")?;
    fs::remove_file(fixture.publish("foo", "1.0.0", 99)?)?;

    fixture.update_all()?;

    fixture.assert_installed("foo", "1.0.0", 1)?;
    fixture.assert_no_staging_dirs(&["foo"])
}

#[test]
fn bulk_versions_are_compared_numerically() -> Result<()> {
    let fixture = BulkUpdate::new()?;
    fixture.install("foo")?;
    fixture.publish("foo", "1.9.0", 7)?;
    fixture.update_all()?;
    fixture.assert_installed("foo", "1.9.0", 7)?;
    fixture.publish("foo", "1.10.0", 8)?;

    fixture.update_all()?;

    fixture.assert_installed("foo", "1.10.0", 8)?;
    fixture.assert_no_staging_dirs(&["foo"])
}

#[test]
fn a_missing_bulk_repository_package_aborts_before_applying_earlier_candidates() -> Result<()> {
    let fixture = BulkUpdate::new()?;
    for name in ["zeta", "missing", "alpha"] {
        fixture.install(name)?;
        fixture.publish(name, "2.0.0", 7)?;
    }
    fixture
        .repo()?
        .execute("DELETE FROM packages WHERE name = 'missing'", [])?;

    let err = fixture
        .update_all()
        .expect_err("a missing repository package must stop the loop");

    assert_eq!(err.to_string(), "Package missing not found");
    fixture.assert_installed("alpha", "1.0.0", 1)?;
    fixture.assert_installed("missing", "1.0.0", 1)?;
    fixture.assert_installed("zeta", "1.0.0", 1)?;
    fixture.assert_no_staging_dirs(&["alpha", "missing", "zeta"])
}

#[test]
fn a_downgrade_stops_before_touching_later_packages() -> Result<()> {
    let fixture = BulkUpdate::new()?;
    fixture.install("alpha")?;
    fixture.install("zeta")?;
    fixture.publish("alpha", "0.9.0", 7)?;
    fixture.publish("zeta", "2.0.0", 7)?;

    let err = fixture
        .update_all()
        .expect_err("downgrades must be rejected");

    assert!(
        err.to_string().contains("Not allowed to downgrade"),
        "{err:#}"
    );
    fixture.assert_installed("alpha", "1.0.0", 1)?;
    fixture.assert_installed("zeta", "1.0.0", 1)?;
    fixture.assert_no_staging_dirs(&["alpha", "zeta"])
}

#[test]
fn invalid_repository_versions_leave_the_installation_untouched() -> Result<()> {
    let fixture = BulkUpdate::new()?;
    fixture.install("foo")?;
    fixture
        .repo()?
        .execute("UPDATE packages SET version = 'not-semver'", [])?;

    let err = fixture
        .update_all()
        .expect_err("invalid repository semver must fail");

    assert_eq!(err.to_string(), "Failed to parse version");
    fixture.assert_installed("foo", "1.0.0", 1)?;
    fixture.assert_no_staging_dirs(&["foo"])
}

#[test]
fn invalid_installed_versions_are_reported_without_attempting_an_upgrade() -> Result<()> {
    let fixture = BulkUpdate::new()?;
    fixture.install("foo")?;
    fixture.publish("foo", "2.0.0", 7)?;
    fixture
        .conn()?
        .execute("UPDATE installed_packages SET version = 'not-semver'", [])?;

    let err = fixture
        .update_all()
        .expect_err("invalid installed semver must fail");

    assert_eq!(err.to_string(), "Failed to parse version");
    let version: String = fixture.conn()?.query_row(
        "SELECT version FROM installed_packages WHERE name = 'foo'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(version, "not-semver");
    assert_eq!(
        fs::read(fixture.dir("foo").join("foo/1.0.0.txt"))?,
        b"foo 1.0.0\n"
    );
    assert!(!fixture.dir("foo").join("foo/2.0.0.txt").exists());
    fixture.assert_no_staging_dirs(&["foo"])
}

#[test]
fn bulk_lookup_uses_the_host_architecture_even_for_a_foreign_installed_row() -> Result<()> {
    let fixture = BulkUpdate::new()?;
    fixture.install("foo")?;
    fixture
        .conn()?
        .execute("UPDATE installed_packages SET architecture = 'arm'", [])?;
    // The host repository version is equal, so the nonexistent archive is never opened.
    fs::remove_file(fixture.publish("foo", "1.0.0", 99)?)?;

    fixture.update_all()?;

    let record: (String, String, u32) = fixture.conn()?.query_row(
        "SELECT architecture, version, release FROM installed_packages WHERE name = 'foo'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(record, ("arm".into(), "1.0.0".into(), 1));
    assert_eq!(
        fs::read(fixture.dir("foo").join("foo/1.0.0.txt"))?,
        b"foo 1.0.0\n"
    );
    fixture.assert_no_staging_dirs(&["foo"])
}

#[test]
fn a_repository_package_for_another_architecture_is_not_a_bulk_candidate() -> Result<()> {
    let fixture = BulkUpdate::new()?;
    fixture.install("foo")?;
    fixture.publish("foo", "2.0.0", 7)?;
    fixture
        .repo()?
        .execute("UPDATE packages SET architecture = 'arm'", [])?;

    let err = fixture
        .update_all()
        .expect_err("the host architecture must be available");

    assert_eq!(err.to_string(), "Package foo not found");
    fixture.assert_installed("foo", "1.0.0", 1)?;
    fixture.assert_no_staging_dirs(&["foo"])
}

#[test]
fn a_database_failure_rolls_back_only_the_failing_package_and_stops_the_loop() -> Result<()> {
    let fixture = BulkUpdate::new()?;
    for name in ["zeta", "middle", "alpha"] {
        fixture.install(name)?;
        fixture.publish(name, "2.0.0", 7)?;
    }
    fixture.conn()?.execute_batch(
        "CREATE TRIGGER reject_middle BEFORE INSERT ON owned_files
         WHEN NEW.path = 'middle/2.0.0.txt'
         BEGIN SELECT RAISE(ABORT, 'injected bulk ledger failure'); END;",
    )?;

    let err = fixture
        .update_all()
        .expect_err("a ledger failure must stop the loop");

    assert!(
        format!("{err:#}").contains("injected bulk ledger failure"),
        "{err:#}"
    );
    fixture.assert_installed("alpha", "2.0.0", 7)?;
    fixture.assert_installed("middle", "1.0.0", 1)?;
    fixture.assert_installed("zeta", "1.0.0", 1)?;
    fixture.assert_no_staging_dirs(&["alpha", "middle", "zeta"])
}

#[test]
fn an_untrusted_archive_stops_the_loop_without_modifying_any_package() -> Result<()> {
    let fixture = BulkUpdate::new()?;
    for name in ["alpha", "zeta"] {
        fixture.install(name)?;
        fixture.publish(name, "2.0.0", 7)?;
    }
    let (untrusted, _) = generate_keypair()?;
    write_signed_sky_with_file(
        &fixture.tmp,
        &untrusted,
        "alpha-2.0.0-7.sky",
        "alpha",
        "2.0.0",
        7,
        b"untrusted",
        vec![],
        "alpha",
        "2.0.0.txt",
    )?;

    let err = fixture
        .update_all()
        .expect_err("untrusted updates must fail verification");

    assert!(
        format!("{err:#}").contains("Failed to verify signature"),
        "{err:#}"
    );
    fixture.assert_installed("alpha", "1.0.0", 1)?;
    fixture.assert_installed("zeta", "1.0.0", 1)?;
    fixture.assert_no_staging_dirs(&["alpha", "zeta"])
}

#[test]
fn a_stable_release_upgrades_an_installed_prerelease() -> Result<()> {
    let fixture = BulkUpdate::new()?;
    fixture.install("foo")?;
    fixture.publish("foo", "2.0.0-rc.1", 7)?;
    fixture.update_all()?;
    fixture.assert_installed("foo", "2.0.0-rc.1", 7)?;
    fixture.publish("foo", "2.0.0", 8)?;

    fixture.update_all()?;

    fixture.assert_installed("foo", "2.0.0", 8)?;
    fixture.assert_no_staging_dirs(&["foo"])
}

#[test]
fn a_missing_install_table_reports_a_database_error() -> Result<()> {
    let fixture = BulkUpdate::new()?;
    fixture
        .conn()?
        .execute("DROP TABLE installed_packages", [])?;

    let err = fixture
        .update_all()
        .expect_err("a missing schema must be reported");

    assert_eq!(err.to_string(), "Failed to prepare database query");
    assert!(
        format!("{err:#}").contains("no such table: installed_packages"),
        "{err:#}"
    );
    fixture.assert_no_staging_dirs(&[])
}

#[test]
fn a_missing_repository_table_reports_an_error_even_with_no_installed_packages() -> Result<()> {
    let fixture = BulkUpdate::new()?;
    fixture.repo()?.execute("DROP TABLE packages", [])?;

    let err = fixture
        .update_all()
        .expect_err("the repository schema must be valid");

    assert_eq!(err.to_string(), "Failed to prepare database statement");
    assert!(
        format!("{err:#}").contains("no such table: packages"),
        "{err:#}"
    );
    fixture.assert_no_staging_dirs(&[])
}

#[test]
fn resolution_errors_are_returned_before_any_upgrade_is_applied() -> Result<()> {
    for failure in ["downgrade", "invalid-version", "invalid-release"] {
        let fixture = BulkUpdate::new()?;
        for name in ["zeta", "middle", "alpha"] {
            fixture.install(name)?;
            fixture.publish(name, "2.0.0", 7)?;
        }
        let (sql, expected) = match failure {
            "downgrade" => (
                "UPDATE packages SET version = '0.9.0' WHERE name = 'middle'",
                "Not allowed to downgrade middle",
            ),
            "invalid-version" => (
                "UPDATE packages SET version = 'invalid' WHERE name = 'middle'",
                "Failed to parse version",
            ),
            "invalid-release" => (
                "UPDATE packages SET release = -1 WHERE name = 'middle'",
                "out of range",
            ),
            _ => unreachable!(),
        };
        fixture.repo()?.execute(sql, [])?;

        let err = fixture
            .update_all()
            .expect_err("resolution failure must be reported");

        assert!(format!("{err:#}").contains(expected), "{failure}: {err:#}");
        fixture.assert_installed("alpha", "1.0.0", 1)?;
        fixture.assert_installed("middle", "1.0.0", 1)?;
        fixture.assert_installed("zeta", "1.0.0", 1)?;
        fixture.assert_no_staging_dirs(&["alpha", "middle", "zeta"])?;
    }
    Ok(())
}
