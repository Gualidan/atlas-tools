mod fixtures;

use anyhow::Result;
use atlas::{
    cli::cli::UpdateArgs, install::install::install, types::runtime_config::RuntimeConfig,
    update::update::update,
};
use fixtures::{
    ARCH, ConfigEnv, generate_keypair, runtime_config, seed_repo_db_with_deps, tempdir,
    write_signed_sky_with_file,
};
use ring::signature::Ed25519KeyPair;
use rusqlite::{Connection, params};
use std::{ffi::OsString, fs, path::PathBuf};
use tempfile::TempDir;

#[allow(dead_code)]
struct Installed {
    version: String,
    release: u32,
    sky_path: String,
    install_dir: PathBuf,
    installed_at: String,
    install_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpdateLogRow {
    id: i64,
    name: String,
    architecture: String,
    version: String,
    release: u32,
}

struct Workspace {
    tmp: TempDir,
    config: RuntimeConfig,
    root: PathBuf,
    keypair: Ed25519KeyPair,
    _env: ConfigEnv,
}

impl Workspace {
    fn new() -> Result<Self> {
        let tmp = tempdir();
        let config = runtime_config(&tmp)?;
        let (keypair, public_key) = generate_keypair()?;
        let env = ConfigEnv::new(&tmp, &public_key)?;
        let root = tmp.path().join("root");
        fs::create_dir(&root)?;

        Ok(Self {
            tmp,
            config,
            root,
            keypair,
            _env: env,
        })
    }

    fn publish(&self, name: &str, version: &str, release: u32, deps: &[&str]) -> Result<PathBuf> {
        let sky = write_signed_sky_with_file(
            &self.tmp,
            &self.keypair,
            &format!("{name}-{version}-{release}.sky"),
            name,
            version,
            release,
            format!("{name} {version}\n").as_bytes(),
            deps.iter().map(ToString::to_string).collect(),
            name,
            &format!("{version}.txt"),
        )?;
        seed_repo_db_with_deps(
            &self.config.repo_db,
            &[(name, version, release, sky.to_str().unwrap(), deps)],
        );

        Ok(sky)
    }

    fn install(&self, name: &str) -> Result<()> {
        install(&name.to_string(), &self.config, &self.root)
    }

    fn update(&self, name: &str) -> Result<()> {
        update(
            &self.config,
            UpdateArgs {
                package: Some(name.into()),
            },
        )
    }

    fn conn(&self) -> Result<Connection> {
        Ok(Connection::open(&self.config.install_db)?)
    }

    fn row(&self, name: &str) -> Result<Installed> {
        let conn = self.conn()?;
        let (version, release, sky_path, install_dir, installed_at, install_reason): (
            String,
            u32,
            String,
            String,
            String,
            String,
        ) = conn.query_row(
            "SELECT version, release, sky_path, install_dir, installed_at, install_reason
                 FROM installed_packages WHERE name = ? AND architecture = ?",
            params![name, ARCH.to_string()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )?;

        Ok(Installed {
            version,
            release,
            sky_path,
            install_dir: PathBuf::from(install_dir),
            installed_at,
            install_reason,
        })
    }

    fn set_metadata(&self, name: &str, installed_at: &str, reason: &str) -> Result<()> {
        self.conn()?.execute(
            "UPDATE installed_packages SET installed_at = ?, install_reason = ?
             WHERE name = ? AND architecture = ?",
            params![installed_at, reason, name, ARCH.to_string()],
        )?;

        Ok(())
    }

    fn deps(&self, name: &str) -> Result<Vec<String>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT dependency_name FROM package_dependencies
             WHERE package_name = ? AND architecture = ? ORDER BY dependency_name",
        )?;
        let deps = stmt
            .query_map(params![name, ARCH.to_string()], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;

        Ok(deps)
    }

    fn assert_installed(&self, name: &str, version: &str, release: u32) -> Result<Installed> {
        let row = self.row(name)?;
        assert_eq!(row.version, version, "{name} version");
        assert_eq!(row.release, release, "{name} release");

        let payload = row.install_dir.join(name);
        let mut files = fs::read_dir(&payload)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<std::io::Result<Vec<OsString>>>()?;
        files.sort();
        assert_eq!(
            files,
            vec![OsString::from(format!("{version}.txt"))],
            "{name} payload files"
        );
        assert_eq!(
            fs::read(payload.join(format!("{version}.txt")))?,
            format!("{name} {version}\n").as_bytes(),
            "{name} payload contents"
        );

        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT path FROM owned_files
             WHERE package_name = ? AND architecture = ? ORDER BY path",
        )?;
        let owned = stmt
            .query_map(params![name, ARCH.to_string()], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        assert_eq!(
            owned,
            vec![format!("{name}/{version}.txt")],
            "{name} owned files"
        );

        Ok(row)
    }

    fn assert_not_installed(&self, name: &str) {
        let row = self.row(name);
        assert!(
            row.is_err(),
            "function should fail because the package is not installed"
        );
    }

    fn assert_no_staging_dirs(&self) -> Result<()> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT install_dir FROM installed_packages")?;
        let mut expected = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        expected.sort();

        let mut actual = fs::read_dir(&self.root)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        actual.sort();

        assert_eq!(
            actual, expected,
            "temporary staging directories must be removed"
        );

        Ok(())
    }

    fn get_update_log(&self) -> Result<Vec<UpdateLogRow>> {
        let conn = self.conn()?;

        let mut stmt = conn.prepare(
            "SELECT id, name, version, release
        FROM installed_packages_update_log
        ORDER BY id ASC",
        )?;

        let rows = stmt
            .query_map([], |row| {
                Ok(UpdateLogRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    architecture: row.get(2)?,
                    version: row.get(3)?,
                    release: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }
}

#[test]
fn updating_a_package_upgrades_its_direct_dependency_in_place() -> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish("lib", "1.0.0", 1, &[])?;
    workspace.publish("app", "1.0.0", 1, &["lib"])?;
    workspace.install("app")?;

    workspace.set_metadata("app", "2000-01-01", "explicit")?;
    workspace.set_metadata("lib", "2000-01-02", "dependency")?;

    let app_dir_before = workspace.row("app")?.install_dir;
    let lib_dir_before = workspace.row("lib")?.install_dir;

    workspace.publish("lib", "2.0.0", 7, &[])?;
    workspace.publish("app", "2.0.0", 7, &["lib"])?;

    workspace.update("app")?;

    let app = workspace.assert_installed("app", "2.0.0", 7)?;
    let lib = workspace.assert_installed("lib", "2.0.0", 7)?;

    assert_eq!(app.install_dir, app_dir_before, "app install directory");
    assert_eq!(lib.install_dir, lib_dir_before, "lib install directory");
    assert_eq!(app.installed_at, "2000-01-01", "app install timestamp");
    assert_eq!(lib.installed_at, "2000-01-02", "lib install timestamp");
    assert_eq!(app.install_reason, "explicit", "app install reason");
    assert_eq!(lib.install_reason, "dependency", "lib install reason");
    assert_eq!(workspace.deps("app")?, vec!["lib".to_string()]);
    assert_eq!(
        workspace.deps("lib")?,
        Vec::<String>::new(),
        "lib's deps should be empty"
    );
    workspace.assert_no_staging_dirs()?;

    Ok(())
}

#[test]
fn dependencies_are_resolved_recursively() -> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish("base", "1.0.0", 1, &[])?;
    workspace.publish("mid", "1.0.0", 1, &["base"])?;
    workspace.publish("app", "1.0.0", 1, &["mid"])?;

    workspace.install("app")?;

    workspace.set_metadata("app", "2000-01-01", "explicit")?;
    workspace.set_metadata("mid", "2000-01-01", "dependency")?;
    workspace.set_metadata("base", "2000-01-01", "dependency")?;

    workspace.publish("base", "2.0.0", 8, &[])?;
    workspace.publish("mid", "2.0.0", 8, &["base"])?;
    workspace.publish("app", "2.0.0", 8, &["mid"])?;

    workspace.update("app")?;

    workspace.assert_installed("app", "2.0.0", 8)?;
    workspace.assert_installed("mid", "2.0.0", 8)?;
    workspace.assert_installed("base", "2.0.0", 8)?;

    assert!(workspace.deps("app")?.contains(&"mid".to_string()));
    assert!(workspace.deps("mid")?.contains(&"base".to_string()));
    assert_eq!(
        workspace.deps("base")?,
        Vec::<String>::new(),
        "base's deps should be empty"
    );

    Ok(())
}

#[test]
fn packages_are_updated_in_order() -> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish("base", "1.0.0", 1, &[])?;
    workspace.publish("left", "1.0.0", 1, &["base"])?;
    workspace.publish("right", "1.0.0", 1, &["base"])?;
    workspace.publish("app", "1.0.0", 1, &["left", "right"])?;

    workspace.install("app")?;

    workspace.set_metadata("app", "2000-01-01", "explicit")?;
    workspace.set_metadata("left", "2000-01-01", "dependency")?;
    workspace.set_metadata("right", "2000-01-01", "dependency")?;
    workspace.set_metadata("base", "2000-01-01", "dependency")?;

    workspace.publish("base", "2.0.0", 8, &[])?;
    workspace.publish("left", "2.0.0", 8, &["base"])?;
    workspace.publish("right", "2.0.0", 8, &["base"])?;
    workspace.publish("app", "2.0.0", 8, &["left", "right"])?;

    workspace.update("app")?;

    let log = workspace.get_update_log()?;

    assert_eq!(log.len(), 4);

    let names: Vec<_> = log.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names.iter().filter(|&&name| name == "base").count(), 1);
    assert!(names.contains(&"base"));
    assert!(names.contains(&"left"));
    assert!(names.contains(&"right"));
    assert!(names.contains(&"app"));

    let base_index = names.iter().position(|&name| name == "base").unwrap();
    let left_index = names.iter().position(|&name| name == "left").unwrap();
    let right_index = names.iter().position(|&name| name == "right").unwrap();
    let app_index = names.iter().position(|&name| name == "app").unwrap();

    assert!(base_index < left_index);
    assert!(base_index < right_index);
    assert!(app_index > base_index);
    assert!(app_index > left_index);
    assert!(app_index > right_index);

    workspace.assert_installed("app", "2.0.0", 8)?;
    workspace.assert_installed("left", "2.0.0", 8)?;
    workspace.assert_installed("right", "2.0.0", 8)?;
    workspace.assert_installed("base", "2.0.0", 8)?;

    assert!(workspace.deps("app")?.contains(&"mid".to_string()));
    assert!(workspace.deps("mid")?.contains(&"base".to_string()));
    assert_eq!(
        workspace.deps("base")?,
        Vec::<String>::new(),
        "base's deps should be empty"
    );

    Ok(())
}

#[test]
fn already_up_to_date_is_a_no_op() -> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish("app", "1.0.0", 1, &[])?;

    Ok(())
}

#[test]
fn installed_package_declares_a_new_dependency_and_updating_fails_and_tells_to_reinstall()
-> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish("app", "1.0.0", 1, &[])?;

    workspace.install("app")?;

    workspace.set_metadata("app", "2000-01-01", "explicit")?;

    workspace.publish("base", "1.0.0", 1, &[])?;
    workspace.publish("app", "2.0.0", 2, &["base"])?;

    let err = workspace.update("app").expect_err(
        "updating should error because a new not yet installed dep was declared in the new version",
    );

    assert_eq!(
        err.to_string(),
        "New `app` version declares `base` as a depency but it is not yet installed, try reinstalling `app`"
    );

    workspace.assert_installed("app", "1.0.0", 1)?;
    workspace.assert_no_staging_dirs()?;
    workspace.assert_not_installed("base");

    Ok(())
}

#[test]
fn missing_transitive_dep_errors() -> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish("mid", "1.0.0", 1, &[])?;
    workspace.publish("app", "1.0.0", 1, &["mid"])?;

    workspace.install("app")?;

    workspace.set_metadata("app", "2000-01-01", "explicit")?;
    workspace.set_metadata("mid", "2000-01-01", "dependency")?;

    workspace.publish("base", "1.0.0", 1, &[])?;
    workspace.publish("mid", "2.0.0", 2, &["base"])?;
    workspace.publish("app", "2.0.0", 2, &["mid"])?;

    let err = workspace.update("app").expect_err("updating should error because a new not yet installed dependency was added to one of the new packages");

    assert_eq!(
        err.to_string(),
        "New version of `mid` (dependdency of package `app`) has declared a new depency `base`, try reinstalling app"
    );

    workspace.assert_installed("app", "1.0.0", 1)?;
    workspace.assert_no_staging_dirs()?;
    workspace.assert_not_installed("base");

    Ok(())
}

#[test]
fn dep_absent_from_the_repository_fails() -> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish("base", "1.0.0", 1, &[])?;
    workspace.publish("app", "1.0.0", 1, &["base"])?;

    workspace.install("app")?;

    workspace.set_metadata("app", "2000-01-01", "explicit")?;
    workspace.set_metadata("base", "2000-01-01", "dependency")?;

    let conn = workspace.conn()?;

    conn.execute("DELETE FROM packages WHERE name = ?", params!["base"])?;

    workspace.publish("base", "2.0.0", 2, &[])?;
    workspace.publish("app", "2.0.0", 2, &["base"])?;

    let err = workspace
        .update("app")
        .expect_err("update should error because the repository rows for base no longer exist");

    assert_eq!(
        err.to_string(),
        "Dependency of `app` `base` not found in repository"
    );

    workspace.assert_installed("app", "1.0.0", 1)?;
    workspace.assert_installed("base", "1.0.0", 1)?;
    workspace.assert_no_staging_dirs()?;

    Ok(())
}

#[test]
fn dependencies_are_not_downgraded() -> Result<()> {
    let workspace = Workspace::new()?;

    Ok(())
}
