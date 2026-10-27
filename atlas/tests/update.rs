mod fixtures;

use anyhow::Result;
use atlas::{
    cli::cli::UpdateArgs,
    install::install::install,
    types::runtime_config::RuntimeConfig,
    update::update::{update, update_one},
};
use common::types::package::PackageIdentity;
use fixtures::{
    ARCH, ConfigEnv, count, generate_keypair, runtime_config, seed_repo_db, tempdir,
    write_signed_sky_with_file,
};
use ring::signature::Ed25519KeyPair;
use rusqlite::{Connection, params};
use std::{fs, path::PathBuf};
use tempfile::{Builder, TempDir};

struct Installation {
    tmp: TempDir,
    config: RuntimeConfig,
    root: PathBuf,
    dir: PathBuf,
    keypair: Ed25519KeyPair,
    _env: ConfigEnv,
}

impl Installation {
    fn new() -> Result<Self> {
        let tmp = tempdir();
        let config = runtime_config(&tmp)?;
        let (keypair, public_key) = generate_keypair()?;
        let config_env = ConfigEnv::new(&tmp, &public_key)?;
        let sky = write_signed_sky_with_file(
            &tmp,
            &keypair,
            "old.sky",
            "foo",
            "1.0.0",
            1,
            b"old\n",
            vec![],
            "dir",
            "old.txt",
        )?;
        seed_repo_db(
            &config.repo_db,
            &[("foo", "1.0.0", 1, sky.to_str().unwrap())],
        );
        let root = tmp.path().join("root");
        install(&"foo".to_string(), &config, root.clone())?;
        let dir = root.join("foo-1.0.0-1");
        Ok(Self {
            tmp,
            config,
            root,
            dir,
            keypair,
            _env: config_env,
        })
    }

    fn publish(&self, version: &str, deps: &[&str]) -> Result<PathBuf> {
        let sky = write_signed_sky_with_file(
            &self.tmp,
            &self.keypair,
            "new.sky",
            "foo",
            version,
            7,
            b"new\n",
            deps.iter().map(|dep| dep.to_string()).collect(),
            "dir",
            "new.txt",
        )?;
        seed_repo_db(
            &self.config.repo_db,
            &[("foo", version, 7, sky.to_str().unwrap())],
        );
        Ok(sky)
    }

    fn update(&self) -> Result<()> {
        update(
            &self.config,
            UpdateArgs {
                package: Some("foo".into()),
            },
        )
    }

    fn conn(&self) -> Result<Connection> {
        Ok(Connection::open(&self.config.install_db)?)
    }

    fn assert_no_staging_dirs(&self) -> Result<()> {
        let entries: Vec<_> = fs::read_dir(&self.root)?.collect::<std::io::Result<_>>()?;
        assert_eq!(
            entries.len(),
            1,
            "only the original install directory should remain"
        );
        assert_eq!(entries[0].path(), self.dir);
        Ok(())
    }

    fn assert_unchanged(&self) -> Result<()> {
        assert_eq!(fs::read(self.dir.join("dir/old.txt"))?, b"old\n");
        assert!(!self.dir.join("dir/new.txt").exists());
        let conn = self.conn()?;
        let record: (String, u32, String, String) = conn.query_row(
            "SELECT version, release, sky_path, install_dir FROM installed_packages WHERE name = 'foo'",
            [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        assert_eq!(
            record,
            (
                "1.0.0".into(),
                1,
                self.tmp.path().join("old.sky").to_str().unwrap().into(),
                self.dir.to_str().unwrap().into()
            )
        );
        let owned: String = conn.query_row(
            "SELECT path FROM owned_files WHERE package_name = 'foo'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(owned, "dir/old.txt");
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM package_dependencies WHERE package_name = 'foo'",
                &[]
            ),
            0
        );
        self.assert_no_staging_dirs()
    }
}

#[test]
fn upgrade_keeps_the_install_location_and_preserves_incoming_dependencies() -> Result<()> {
    let fixture = Installation::new()?;
    let conn = fixture.conn()?;
    conn.execute_batch(
        "INSERT INTO installed_packages (name, version, release, architecture, sky_path, install_dir, install_reason)
         VALUES ('consumer', '1.0.0', 1, 'x86_64', 'consumer.sky', '/unused', 'explicit');
         INSERT INTO package_dependencies VALUES ('consumer', 'x86_64', 'foo');
         UPDATE installed_packages SET installed_at = '2000-01-01', install_reason = 'dependency' WHERE name = 'foo';",
    )?;
    let sky = fixture.publish("2.0.0", &[])?;

    fixture.update()?;

    assert_eq!(fs::read(fixture.dir.join("dir/new.txt"))?, b"new\n");
    assert!(!fixture.dir.join("dir/old.txt").exists());
    let record: (String, u32, String, String, String, String) = conn.query_row(
        "SELECT version, release, sky_path, install_dir, installed_at, install_reason FROM installed_packages WHERE name = 'foo'",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    )?;
    assert_eq!(
        record,
        (
            "2.0.0".into(),
            7,
            sky.to_str().unwrap().into(),
            fixture.dir.to_str().unwrap().into(),
            "2000-01-01".into(),
            "dependency".into()
        )
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM package_dependencies WHERE package_name = 'consumer' AND dependency_name = 'foo'",
            &[]
        ),
        1
    );
    let owned: String = conn.query_row(
        "SELECT path FROM owned_files WHERE package_name = 'foo'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(owned, "dir/new.txt");
    fixture.assert_no_staging_dirs()
}

#[test]
fn update_one_commits_using_its_supplied_connections_and_staging_directory() -> Result<()> {
    let fixture = Installation::new()?;
    let sky = fixture.publish("2.0.0", &[])?;
    let repo = Connection::open(&fixture.config.repo_db)?;
    let mut conn = fixture.conn()?;
    let staging = Builder::new().tempdir_in(&fixture.root)?;
    let staging_path = staging.path().to_path_buf();

    update_one(
        PackageIdentity {
            name: "foo".to_string(),
            version: "2.0.0".to_string(),
            release: 7,
            architecture: ARCH,
        },
        &repo,
        &mut conn,
        sky,
        staging,
        fixture.dir.clone(),
    )?;

    assert_eq!(fs::read(fixture.dir.join("dir/new.txt"))?, b"new\n");
    assert!(
        !staging_path.exists(),
        "the old install must be cleaned up with the TempDir"
    );
    assert_eq!(
        conn.query_row(
            "SELECT version FROM installed_packages WHERE name = 'foo'",
            [],
            |row| row.get::<_, String>(0)
        )?,
        "2.0.0"
    );
    fixture.assert_no_staging_dirs()
}

#[test]
fn database_failure_after_swap_restores_the_previous_installation() -> Result<()> {
    let fixture = Installation::new()?;
    fixture.publish("2.0.0", &["missing-dep"])?;
    let err = fixture
        .update()
        .expect_err("missing dependency must prevent commit");
    assert!(
        format!("{err:#}").contains("Dependency `missing-dep` not found"),
        "{err:#}"
    );
    fixture.assert_unchanged()
}

#[test]
fn database_write_error_restores_files_and_all_ledger_changes() -> Result<()> {
    let fixture = Installation::new()?;
    fixture.publish("2.0.0", &[])?;
    fixture.conn()?.execute_batch(
        "CREATE TRIGGER reject_new_file BEFORE INSERT ON owned_files
         WHEN NEW.path = 'dir/new.txt'
         BEGIN SELECT RAISE(ABORT, 'injected ledger failure'); END;",
    )?;
    let err = fixture
        .update()
        .expect_err("database rejection must roll back the swap");
    assert!(
        format!("{err:#}").contains("injected ledger failure"),
        "{err:#}"
    );
    fixture.assert_unchanged()
}

#[test]
fn metadata_mismatch_rejects_the_update_before_swapping_files() -> Result<()> {
    let fixture = Installation::new()?;
    fixture.publish("2.0.0", &[])?;
    Connection::open(&fixture.config.repo_db)?
        .execute("UPDATE packages SET release = 8 WHERE name = 'foo'", [])?;
    let err = fixture
        .update()
        .expect_err("the archive must match the selected release");
    assert!(
        format!("{err:#}").contains("metadata does not match"),
        "{err:#}"
    );
    fixture.assert_unchanged()
}

#[test]
fn signed_payload_with_setuid_permissions_is_rejected_before_staging() -> Result<()> {
    let fixture = Installation::new()?;
    fixture.publish("2.0.0", &[])?;
    fixtures::write_signed_sky_with_raw_entry(
        &fixture.tmp,
        &fixture.keypair,
        "new.sky",
        "foo",
        "2.0.0",
        7,
        "dir/unsafe",
        0o4755,
        tar::EntryType::file(),
        b"unsafe",
    )?;
    let err = fixture
        .update()
        .expect_err("unsafe payload must be rejected");
    assert!(format!("{err:#}").contains("setuid bit set"), "{err:#}");
    fixture.assert_unchanged()
}

#[test]
fn untrusted_signature_preserves_the_previous_installation() -> Result<()> {
    let fixture = Installation::new()?;
    fixture.publish("2.0.0", &[])?;
    let (untrusted_key, _) = generate_keypair()?;
    write_signed_sky_with_file(
        &fixture.tmp,
        &untrusted_key,
        "new.sky",
        "foo",
        "2.0.0",
        7,
        b"untrusted",
        vec![],
        "dir",
        "new.txt",
    )?;
    let err = fixture
        .update()
        .expect_err("untrusted archive must be rejected");
    assert!(
        format!("{err:#}").contains("Failed to verify signature"),
        "{err:#}"
    );
    fixture.assert_unchanged()
}

#[test]
fn conflicting_file_is_rejected_before_changing_the_installation() -> Result<()> {
    let fixture = Installation::new()?;
    fixture.publish("2.0.0", &[])?;
    let conn = fixture.conn()?;
    conn.execute_batch(
        "INSERT INTO installed_packages (name, version, release, architecture, sky_path, install_dir, install_reason)
         VALUES ('neighbor', '1.0.0', 1, 'x86_64', 'neighbor.sky', '/unused', 'explicit');
         INSERT INTO owned_files VALUES ('dir/new.txt', 'neighbor', 'x86_64');",
    )?;
    let err = fixture
        .update()
        .expect_err("another package owns the new file");
    assert!(
        format!("{err:#}").contains("Another package neighbor already owns this path dir/new.txt"),
        "{err:#}"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM owned_files WHERE path = 'dir/new.txt' AND package_name = 'neighbor'",
            &[]
        ),
        1
    );
    fixture.assert_unchanged()
}

#[test]
fn equal_version_is_a_no_op_even_when_the_new_archive_is_unavailable() -> Result<()> {
    let fixture = Installation::new()?;
    let sky = fixture.publish("1.0.0", &[])?;
    fs::remove_file(sky)?;
    fixture.update()?;
    fixture.assert_unchanged()
}

#[test]
fn downgrade_is_rejected_without_changing_files_or_records() -> Result<()> {
    let fixture = Installation::new()?;
    fixture.publish("0.9.0", &[])?;
    let err = fixture.update().expect_err("downgrade must be rejected");
    assert!(
        format!("{err:#}").contains("Not allowed to downgrade"),
        "{err:#}"
    );
    fixture.assert_unchanged()
}

#[test]
fn invalid_repository_version_leaves_the_installation_untouched() -> Result<()> {
    let fixture = Installation::new()?;
    fixture.publish("not-semver", &[])?;
    let err = fixture
        .update()
        .expect_err("invalid semver must be rejected");
    assert!(
        format!("{err:#}").contains("Failed to parse version number"),
        "{err:#}"
    );
    fixture.assert_unchanged()
}

#[test]
fn missing_repository_package_leaves_the_installation_untouched() -> Result<()> {
    let fixture = Installation::new()?;
    Connection::open(&fixture.config.repo_db)?
        .execute("DELETE FROM packages WHERE name = 'foo'", [])?;
    let err = fixture
        .update()
        .expect_err("missing repository package must be rejected");
    assert!(
        format!("{err:#}").contains("Package foo not found"),
        "{err:#}"
    );
    fixture.assert_unchanged()
}

#[test]
fn repository_package_for_another_architecture_is_not_a_candidate() -> Result<()> {
    let fixture = Installation::new()?;
    fixture.publish("2.0.0", &[])?;
    Connection::open(&fixture.config.repo_db)?.execute(
        "UPDATE packages SET architecture = 'arm' WHERE name = 'foo'",
        [],
    )?;
    let err = fixture.update().expect_err("architecture must match");
    assert!(
        format!("{err:#}").contains("Package foo not found"),
        "{err:#}"
    );
    fixture.assert_unchanged()
}

#[test]
fn updating_a_package_that_is_not_installed_reports_not_found() -> Result<()> {
    let fixture = Installation::new()?;
    let err = update(
        &fixture.config,
        UpdateArgs {
            package: Some("unknown".into()),
        },
    )
    .expect_err("unknown package must be rejected");
    assert!(
        format!("{err:#}").contains("Package unknown not found in installed packages"),
        "{err:#}"
    );
    fixture.assert_unchanged()
}

#[test]
fn semantic_versions_are_compared_numerically() -> Result<()> {
    let fixture = Installation::new()?;
    fixture.conn()?.execute(
        "UPDATE installed_packages SET version = '1.9.0' WHERE name = 'foo'",
        [],
    )?;
    fixture.publish("1.10.0", &[])?;
    fixture.update()?;
    assert_eq!(
        fixture.conn()?.query_row(
            "SELECT version FROM installed_packages WHERE name = ?",
            params!["foo"],
            |row| row.get::<_, String>(0)
        )?,
        "1.10.0"
    );
    assert_eq!(fs::read(fixture.dir.join("dir/new.txt"))?, b"new\n");
    fixture.assert_no_staging_dirs()
}

#[test]
fn single_update_does_not_resolve_repository_install_dependencies() -> Result<()> {
    let fixture = Installation::new()?;
    fixture.publish("2.0.0", &[])?;
    Connection::open(&fixture.config.repo_db)?.execute(
        "UPDATE packages SET deps = 'invalid: [yaml' WHERE name = 'foo'",
        [],
    )?;

    fixture.update()?;

    assert_eq!(fs::read(fixture.dir.join("dir/new.txt"))?, b"new\n");
    assert!(!fixture.dir.join("dir/old.txt").exists());
    let conn = fixture.conn()?;
    let (version, dir): (String, String) = conn.query_row(
        "SELECT version, install_dir FROM installed_packages WHERE name = 'foo'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(version, "2.0.0");
    assert_eq!(dir, fixture.dir.to_str().unwrap());
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM installed_packages", &[]),
        1
    );
    fixture.assert_no_staging_dirs()
}
