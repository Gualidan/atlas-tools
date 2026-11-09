use anyhow::{Context as AnyhowContext, Result, anyhow, bail};
use common::types::{
    config::SisyphusConfig,
    package::{Architectures, PackageIdentity},
};
use std::{env::consts::ARCH, fs::create_dir_all, path::PathBuf};

use common::types::config::load_config;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use tempfile::{Builder, TempDir};

use crate::{
    cli::confirmation::{Action, confirmation_prompt},
    install::{
        db::{InstallReason, ensure_install_schema, insert_installed_package},
        resolve::resolve_install_order,
    },
    types::{guards::SwapBackGuard, package_state::PackageState, runtime_config::RuntimeConfig},
};

/// Installs the named package and its repository-declared dependencies for the
/// host architecture under `install_dir/<name>-<version>-<release>`.
/// Already installed dependencies are skipped; the requested package is reinstalled.
///
/// Propagates resolution, configuration, verification, filesystem, and database
/// errors. Packages successfully installed before a later failure remain installed.
///
/// # Errors
///
/// Returns an error if repository resolution, confirmation, package verification,
/// filesystem operations, or database updates fail.
#[allow(clippy::too_many_lines)]
pub fn install(
    package_name: Option<&str>,
    group: Option<&str>,
    runtime_config: &RuntimeConfig,
    install_dir: &PathBuf,
) -> Result<()> {
    match (package_name, group) {
        (Some(_), Some(_)) => {
            bail!("Specify a package or group name to install, not both.")
        }
        (None, None) => {
            bail!("Specify a package or group name to install.")
        }
        (None, Some(group)) => {
            install_group(group, runtime_config)?;
            Ok(())
        }
        (Some(package_name), None) => {
            let repo_conn = Connection::open(&runtime_config.repo_db)
                .context("Failed to open connection to the repository database")?;
            let mut stmt = repo_conn
                .prepare("SELECT name FROM packages WHERE name = ? AND architecture = ?")
                .context("Failed to prepare database query")?;
            let Some(name): Option<String> = stmt
                .query_row(params![package_name, ARCH], |row| row.get(0))
                .optional()?
            else {
                return Err(anyhow!("Package {package_name} not found"));
            };

            let architecture = Architectures::try_from(ARCH.to_string())?;
            let config = load_config().context("Failed to load settings")?;

            let to_install = {
                let install_conn = if runtime_config.install_db.exists() {
                    Connection::open_with_flags(
                        &runtime_config.install_db,
                        OpenFlags::SQLITE_OPEN_READ_ONLY,
                    )
                    .context("Failed to open connection to install database")?
                } else {
                    let conn = Connection::open_in_memory()
                        .context("Failed to open temporary install database")?;
                    ensure_install_schema(&conn)
                        .context("Failed to create the install database schema")?;
                    conn
                };
                resolve_install_order(&repo_conn, &install_conn, name.clone(), architecture)?
            };
            let packages = to_install
                .iter()
                .map(|pkg| (pkg.name.as_str(), pkg.version.as_str()))
                .collect::<Vec<(&str, &str)>>();

            let ans = confirmation_prompt(packages, 4, &Action::Install)?;

            if !ans {
                return Ok(());
            }

            create_dir_all(install_dir).context("Failed to create install_dir's parents")?;

            if let Some(parent) = runtime_config.install_db.parent() {
                std::fs::create_dir_all(parent)
                    .context("Failed to create the install database directory")?;
            }

            let install_conn = Connection::open(&runtime_config.install_db)
                .context("Failed to open connection to install database")?;
            install_conn
                .execute("PRAGMA foreign_keys = ON", [])
                .context("Failed to enable foreign key enforcement")?;
            ensure_install_schema(&install_conn)
                .context("Failed to create the install database schema")?;

            let mut stmt = install_conn
                .prepare("SELECT name FROM installed_packages WHERE name = ? AND architecture = ?")
                .context("Failed to prepare database query")?;

            for package in to_install {
                let is_root = package.name == name;

                if !is_root {
                    let installed = stmt
                        .query_row(
                            params![package.name, package.architecture.to_string()],
                            |row| row.get::<_, String>(0),
                        )
                        .optional()?
                        .is_some();

                    if installed {
                        continue;
                    }
                }

                let install_reason = if is_root {
                    InstallReason::Explicit
                } else {
                    InstallReason::Dependency
                };

                let package_id = PackageIdentity {
                    name: package.name,
                    version: package.version,
                    release: package.release,
                    architecture: package.architecture,
                };

                let tempdir = staging_directory(install_dir)?;

                install_one(
                    runtime_config,
                    install_reason,
                    &package_id,
                    &package.groups,
                    &PathBuf::from(&package.sky_path),
                    install_dir,
                    tempdir,
                    &config,
                    &repo_conn,
                    &install_conn,
                )?;
            }

            Ok(())
        }
    }
}

/// Verifies and stages one package, exchanges it into `install_dir`, and records
/// it in the database selected by `runtime_config` with `install_reason`.
/// `conn` supplies repository identities; `install_conn` supplies file ownership
/// records for conflict checks.
///
/// `tempdir` must share a filesystem with `install_dir`. Database recording failures
/// trigger a best-effort swap back, removing a newly created destination.
/// Verification, staging, filesystem, and database errors propagate.
///
/// # Errors
///
/// Returns an error if package verification, staging, exchange, or database
/// recording fails.
///
/// # Panics
///
/// Panics if `sky_path` is not valid UTF-8 when recording the installation.
#[allow(clippy::too_many_arguments)]
pub fn install_one(
    runtime_config: &RuntimeConfig,
    install_reason: InstallReason,
    package_id: &PackageIdentity,
    groups: &Vec<String>,
    sky_path: &PathBuf,
    install_dir: &PathBuf,
    tempdir: TempDir,
    config: &SisyphusConfig,
    conn: &Connection,
    install_conn: &Connection,
) -> Result<()> {
    let state = PackageState::NotInstalled {
        package_id: package_id.clone(),
        sky_path,
        install_dir,
        tempdir: &tempdir,
        config,
        repo_conn: conn,
        install_conn,
    };
    let state = state.verify()?;
    let state = state.stage()?;
    let (_, metadata, _, entries) = state.commit()?;

    let created_dirs = entries
        .iter()
        .filter_map(|entry| match entry {
            crate::types::guards::GuardedEntry::CreatedDir(path) => Some(path.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut guard = SwapBackGuard::new(install_dir, tempdir, entries);

    insert_installed_package(
        runtime_config,
        &metadata,
        package_id,
        groups,
        install_dir.to_string_lossy().as_ref(),
        sky_path.to_str().unwrap(),
        install_reason,
        &created_dirs,
    )?;

    guard.disarm();
    Ok(())
}

/// Queries the repository for packages in the given group and installs them.
///
/// # Errors
///
/// Returns an error if the group cannot be resolved or one of its packages
/// cannot be installed.
pub fn install_group(_group: &str, _config: &RuntimeConfig) -> Result<()> {
    todo!("Implement this and `resolve_group`");
}

pub(crate) fn staging_directory(install_dir: &std::path::Path) -> Result<TempDir> {
    let location = install_dir.join("var/lib/atlas/tmp");
    crate::types::package_state::ensure_no_symlink_components(
        install_dir,
        std::path::Path::new("var/lib/atlas/tmp"),
        false,
    )?;
    create_dir_all(&location).context("Failed to create staging directory")?;
    Builder::new()
        .prefix("sisyphus_install_tmp_dir_")
        .tempdir_in(location)
        .context("Failed to create temporary directory")
}
