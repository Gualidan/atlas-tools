mod fixtures;

use anyhow::Result;
use atlas::{
    install::db::ensure_install_schema,
    types::{contexts::RemoveContext, package_state::PackageState},
};
use common::types::{config::SisyphusConfig, package::PackageIdentity};
use fixtures::{ARCH, generate_keypair, metadata_for, seed_repo_db, tempdir, write_signed_sky};
use rusqlite::{Connection, params};
use std::{fs, path::PathBuf};
use tempfile::{Builder, TempDir};

struct StateFixture {
    tmp: TempDir,
    sky: PathBuf,
    install_dir: PathBuf,
    repo: Connection,
    installed: Connection,
    config: SisyphusConfig,
}

impl StateFixture {
    fn new() -> Result<Self> {
        let tmp = tempdir();
        let (keypair, public_key) = generate_keypair()?;
        let sky = write_signed_sky(
            &tmp,
            &keypair,
            "foo.sky",
            "foo",
            "2.0.0",
            3,
            b"new\n",
            vec![],
        )?;
        let repo_path = tmp.path().join("repo.db");
        seed_repo_db(&repo_path, &[("foo", "2.0.0", 3, sky.to_str().unwrap())]);
        let repo = Connection::open(repo_path)?;
        let installed = Connection::open_in_memory()?;
        ensure_install_schema(&installed)?;
        installed.execute_batch("PRAGMA foreign_keys = ON")?;
        let public_key_path = tmp.path().join("public.key");
        fs::write(&public_key_path, public_key)?;
        let config = SisyphusConfig {
            pub_key_path: public_key_path,
            priv_key_path: tmp.path().join("unused.key"),
        };
        let install_dir = tmp.path().join("install");
        Ok(Self {
            tmp,
            sky,
            install_dir,
            repo,
            installed,
            config,
        })
    }

    fn identity(&self) -> PackageIdentity<'_> {
        PackageIdentity {
            name: "foo",
            version: "2.0.0",
            release: 3,
            architecture: ARCH,
        }
    }

    fn staging(&self) -> Result<TempDir> {
        Ok(Builder::new().tempdir_in(self.tmp.path())?)
    }

    fn initial(&self, tempdir: TempDir) -> PackageState<'_> {
        PackageState::NotInstalled {
            package_id: self.identity(),
            install_conn: &self.installed,
            tempdir,
            sky_path: &self.sky,
            config: &self.config,
            repo_conn: &self.repo,
            install_dir: &self.install_dir,
        }
    }

    fn states(&self) -> Result<Vec<(&str, PackageState<'_>)>> {
        Ok(vec![
            ("not installed", self.initial(self.staging()?)),
            ("verified", self.initial(self.staging()?).verify()?),
            (
                "staged",
                PackageState::Staged {
                    tempdir: self.staging()?,
                    metadata: metadata_for("foo", "2.0.0", 3, vec![]),
                    install_dir: &self.install_dir,
                },
            ),
            ("installed", PackageState::Installed),
        ])
    }

    fn claim(&self, package: &str, architecture: &str, path: &str) -> Result<()> {
        self.installed.execute("INSERT INTO installed_packages (name, version, release, architecture, sky_path, install_dir, install_reason) VALUES (?, '1.0.0', 1, ?, 'old.sky', '/unused', 'explicit')", params![package, architecture])?;
        self.installed.execute(
            "INSERT INTO owned_files VALUES (?, ?, ?)",
            params![path, package, architecture],
        )?;
        Ok(())
    }
}

#[test]
fn verify_stage_and_commit_transfer_metadata_and_keep_the_old_tree_alive() -> Result<()> {
    let fixture = StateFixture::new()?;
    fs::create_dir(&fixture.install_dir)?;
    fs::write(fixture.install_dir.join("old.txt"), b"old\n")?;
    let staging = fixture.staging()?;
    let staging_path = staging.path().to_path_buf();

    let state = fixture.initial(staging).verify()?;
    assert!(!fixture.install_dir.join("dir/file.txt").exists());
    assert!(fs::read_dir(&staging_path)?.next().is_none());
    let state = state.stage()?;
    assert_eq!(fs::read(staging_path.join("dir/file.txt"))?, b"new\n");
    assert_eq!(fs::read(fixture.install_dir.join("old.txt"))?, b"old\n");

    let (state, metadata, previous_install) = state.commit()?;
    assert!(matches!(state, PackageState::Installed));
    assert_eq!(
        (
            metadata.name.as_str(),
            metadata.version.as_str(),
            metadata.release
        ),
        ("foo", "2.0.0", 3)
    );
    assert_eq!(previous_install.path(), staging_path);
    assert_eq!(fs::read(previous_install.path().join("old.txt"))?, b"old\n");
    assert_eq!(
        fs::read(fixture.install_dir.join("dir/file.txt"))?,
        b"new\n"
    );
    assert!(!fixture.install_dir.join("old.txt").exists());
    drop(previous_install);
    assert!(!staging_path.exists());
    assert!(fixture.install_dir.join("dir/file.txt").exists());
    Ok(())
}

#[test]
fn commit_creates_a_missing_install_directory() -> Result<()> {
    let fixture = StateFixture::new()?;
    let (_, _, previous_install) = fixture
        .initial(fixture.staging()?)
        .verify()?
        .stage()?
        .commit()?;
    assert_eq!(
        fs::read(fixture.install_dir.join("dir/file.txt"))?,
        b"new\n"
    );
    assert!(fs::read_dir(previous_install.path())?.next().is_none());
    Ok(())
}

#[test]
fn failed_verification_cleans_up_its_owned_staging_directory() -> Result<()> {
    let fixture = StateFixture::new()?;
    fixture
        .repo
        .execute("UPDATE packages SET release = 4", [])?;
    let staging = fixture.staging()?;
    let path = staging.path().to_path_buf();
    assert!(fixture.initial(staging).verify().is_err());
    assert!(!path.exists());
    assert!(!fixture.install_dir.exists());
    Ok(())
}

#[test]
fn staging_allows_a_packages_own_file_during_an_upgrade() -> Result<()> {
    let fixture = StateFixture::new()?;
    fixture.claim("foo", "x86_64", "dir/file.txt")?;
    assert!(matches!(
        fixture.initial(fixture.staging()?).verify()?.stage()?,
        PackageState::Staged { .. }
    ));
    assert!(
        !fixture.install_dir.exists(),
        "staging must not commit the tree"
    );
    Ok(())
}

#[test]
fn staging_ignores_other_architectures_and_shared_directories() -> Result<()> {
    let fixture = StateFixture::new()?;
    fixture.claim("neighbor", "arm", "dir/file.txt")?;
    fixture.claim("neighbor", "x86_64", "dir")?;
    assert!(
        fixture
            .initial(fixture.staging()?)
            .verify()?
            .stage()
            .is_ok()
    );
    Ok(())
}

#[test]
fn staging_rejects_another_owner_and_cleans_up_without_touching_the_install() -> Result<()> {
    let fixture = StateFixture::new()?;
    fixture.claim("neighbor", "x86_64", "dir/file.txt")?;
    fs::create_dir(&fixture.install_dir)?;
    fs::write(fixture.install_dir.join("old.txt"), b"old\n")?;
    let staging = fixture.staging()?;
    let staging_path = staging.path().to_path_buf();
    let err = fixture
        .initial(staging)
        .verify()?
        .stage()
        .err()
        .expect("conflict must be rejected");
    assert!(
        format!("{err:#}").contains("Another package neighbor already owns this path dir/file.txt"),
        "{err:#}"
    );
    assert!(!staging_path.exists());
    assert_eq!(fs::read(fixture.install_dir.join("old.txt"))?, b"old\n");
    assert!(!fixture.install_dir.join("dir").exists());
    Ok(())
}

#[test]
fn commit_failure_drops_staging_without_replacing_an_existing_file() -> Result<()> {
    let fixture = StateFixture::new()?;
    fs::write(&fixture.install_dir, b"not a directory")?;
    let staging = fixture.staging()?;
    let path = staging.path().to_path_buf();
    let state = fixture.initial(staging).verify()?.stage()?;
    let err = state
        .commit()
        .err()
        .expect("a regular file cannot be an install directory");
    assert!(format!("{err:#}").contains("Failed to create the install directory"));
    assert!(!path.exists());
    assert_eq!(fs::read(&fixture.install_dir)?, b"not a directory");
    Ok(())
}

#[test]
fn invalid_verify_transitions_are_rejected() -> Result<()> {
    let fixture = StateFixture::new()?;
    for (name, state) in fixture.states()? {
        if name == "not installed" {
            continue;
        }
        let err = state.verify().err().expect("invalid verify transition");
        assert!(
            err.to_string().contains("not allowed to verify"),
            "{name}: {err:#}"
        );
    }
    Ok(())
}

#[test]
fn invalid_stage_transitions_are_rejected() -> Result<()> {
    let fixture = StateFixture::new()?;
    for (name, state) in fixture.states()? {
        if name == "verified" {
            continue;
        }
        let err = state.stage().err().expect("invalid stage transition");
        assert!(
            err.to_string().contains("not allowed to stage"),
            "{name}: {err:#}"
        );
    }
    Ok(())
}

#[test]
fn invalid_commit_transitions_are_rejected() -> Result<()> {
    let fixture = StateFixture::new()?;
    for (name, state) in fixture.states()? {
        if name == "staged" {
            continue;
        }
        let err = state.commit().err().expect("invalid commit transition");
        assert!(
            err.to_string().contains("not allowed to commit"),
            "{name}: {err:#}"
        );
    }
    assert!(!fixture.install_dir.exists());
    Ok(())
}

#[test]
fn invalid_remove_transitions_leave_existing_files_alone() -> Result<()> {
    let fixture = StateFixture::new()?;
    fs::create_dir(&fixture.install_dir)?;
    fs::write(fixture.install_dir.join("old.txt"), b"old\n")?;
    let identity = fixture.identity();
    let mut ctx = RemoveContext {
        package_id: &identity,
        install_conn: &fixture.installed,
        install_dir: fixture.install_dir.clone(),
    };
    for (name, state) in fixture.states()? {
        if name == "installed" {
            continue;
        }
        let err = state
            .remove(&mut ctx)
            .expect_err("invalid remove transition");
        assert!(
            err.to_string().contains("not allowed to remove"),
            "{name}: {err:#}"
        );
        assert_eq!(fs::read(fixture.install_dir.join("old.txt"))?, b"old\n");
    }
    Ok(())
}
