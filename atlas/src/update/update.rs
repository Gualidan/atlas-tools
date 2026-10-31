use std::{env::consts::ARCH, path::PathBuf};

use anyhow::{Context, Result};
use common::types::{
    config::load_config,
    package::{Architectures, PackageIdentity},
};
use rusqlite::Connection;
use rustix::path::Arg;

use tempfile::{Builder, TempDir};

use crate::{
    cli::{
        cli::UpdateArgs,
        confirmation::{Action, confirmation_prompt},
    },
    types::{guards::SwapBackGuard, package_state::PackageState, runtime_config::RuntimeConfig},
    update::{
        db::{clear_update_log, update_installed_package},
        resolve::{Resolution, resolve_update_order},
    },
};

/// Upgrades `args.package` for the host architecture, or every installed package
/// with an available upgrade when it is `None`. Repository lookups always use
/// the host architecture, even for installed rows with another architecture.
///
/// Replaces each package in its recorded directory when the repository's semantic
/// version is newer. Equal versions succeed without updating, even if release
/// numbers differ; downgrades fail. An empty update set succeeds without asking.
///
/// Prompts before applying the resolved updates unless `set_assume_yes` is
/// enabled. Declining succeeds without applying updates. In bulk mode, the
/// first per-package resolution error is returned before any prompt, and no
/// update is applied.
///
/// Propagates missing-package, version parsing, confirmation, database,
/// configuration, verification, staging, and filesystem errors. Stops at the
/// first update failure; earlier successful updates remain applied.
///
/// # Errors
///
/// Returns an error if update resolution, configuration, confirmation, archive
/// verification, filesystem operations, or database updates fail.
pub fn update(runtime_config: &RuntimeConfig, args: &UpdateArgs) -> Result<()> {
    let mut install_db_conn = Connection::open(&runtime_config.install_db)
        .context("Failed to connect to install database")?;

    let repo_db_conn = Connection::open(&runtime_config.repo_db)
        .context("Failed to open connection to repository database")?;

    clear_update_log(&mut install_db_conn).context("Failed to clear update log")?;

    let Resolution {
        candidates: to_update,
        error: deferred_error,
    } = resolve_update_order(&install_db_conn, &repo_db_conn, args.package.as_deref())?;

    if let Some(error) = deferred_error {
        return Err(error);
    }

    if to_update.is_empty() {
        match args.package.as_deref() {
            Some(package) => println!("Package {package} is already up to date"),
            None => println!("All packages are up to date"),
        }
        return Ok(());
    }

    let packages = to_update
        .iter()
        .map(|row| (row.name.as_str(), row.version.as_str()))
        .collect::<Vec<(&str, &str)>>();

    if !confirmation_prompt(packages, 4, &Action::Update)? {
        return Ok(());
    }

    for package in to_update {
        let package_id = PackageIdentity {
            name: package.name,
            version: package.version,
            release: package.release,
            architecture: Architectures::try_from(ARCH.to_string())?,
        };

        let install_dir = PathBuf::from(package.install_dir);

        let tempdir = Builder::new()
            .prefix("sisyphus_install_tmp_dir_")
            .tempdir_in(
                install_dir
                    .parent()
                    .context("Failed to get install dir's parent")?,
            )
            .context("Failed to create temporary directory")?;

        update_one(
            &package_id,
            &repo_db_conn,
            &mut install_db_conn,
            &PathBuf::from(package.sky_path),
            tempdir,
            &install_dir,
        )?;
    }

    Ok(())
}

/// Verifies and stages the supplied package, exchanges it into `install_dir`,
/// and updates the existing install records. The caller selects the version;
/// this function does not compare it with the installed version.
///
/// `install_dir` is expected to exist, and `tempdir` must share its filesystem.
/// Recording failures trigger a best-effort swap back. Configuration,
/// verification, staging, filesystem, and database errors propagate.
///
/// # Errors
///
/// Returns an error if configuration loading, package verification, staging,
/// filesystem exchange, or database recording fails.
pub fn update_one(
    package_id: &PackageIdentity,
    repo_conn: &Connection,
    install_conn: &mut Connection,
    sky_path: &PathBuf,
    tempdir: TempDir,
    install_dir: &PathBuf,
) -> Result<()> {
    let config = load_config().context("Failed to load configuration")?;

    let state = PackageState::NotInstalled {
        package_id: package_id.clone(),
        install_conn,
        sky_path,
        tempdir,
        config: &config,
        repo_conn,
        install_dir,
    };

    let state = state.verify()?;
    let state = state.stage()?;
    let (_, metadata, staging_dir) = state.commit()?;
    let mut guard = SwapBackGuard::new(install_dir, &staging_dir, true);

    update_installed_package(
        install_conn,
        repo_conn,
        &metadata,
        &package_id.clone(),
        &sky_path.to_string_lossy(),
        &install_dir.to_string_lossy(),
    )?;

    guard.disarm();
    Ok(())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
