use std::{collections::HashSet, env::consts::ARCH, fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use common::{
    functions::sky_verify::read_sky,
    types::{
        config::load_config,
        package::{Architectures, PackageIdentity},
    },
};
use rusqlite::{Connection, params};
use rustix::fs::{CWD, RenameFlags, renameat_with};

use tempfile::TempDir;

use crate::{
    cli::{
        cli::UpdateArgs,
        confirmation::{Action, confirmation_prompt},
    },
    types::{
        guards::SwapBackGuard,
        package_state::{PackageState, ensure_no_symlink_components},
        runtime_config::RuntimeConfig,
    },
    update::{
        db::{clear_update_log, update_installed_package},
        resolve::{Resolution, resolve_all, resolve_group, resolve_one},
    },
    verify::parse_metadata_lines::parse_metadata_lines,
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
#[allow(clippy::too_many_lines)]
pub fn update(config: &RuntimeConfig, args: &UpdateArgs) -> Result<()> {
    match (args.package.as_deref(), args.group.as_deref()) {
        (Some(package), None) => {
            let mut install_db_conn = Connection::open(&config.install_db)
                .context("Failed to connect to install database")?;

            let repo_db_conn = Connection::open(&config.repo_db)
                .context("Failed to open connection to repository database")?;

            clear_update_log(&mut install_db_conn).context("Failed to clear update log")?;

            let to_update = resolve_one(&install_db_conn, &repo_db_conn, package)?;

            if to_update.is_empty() {
                println!("Package `{package}` is already up to date");
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

                let tempdir = crate::install::install::staging_directory(&install_dir)?;

                update_one(
                    &package_id,
                    &package.groups,
                    &repo_db_conn,
                    &mut install_db_conn,
                    &PathBuf::from(package.sky_path),
                    tempdir,
                    &install_dir,
                )?;
            }

            Ok(())
        }

        (None, None) => {
            let mut install_conn =
                Connection::open(&config.install_db).context("Failed to open install database")?;
            let repo_conn =
                Connection::open(&config.repo_db).context("Failed to open repo database")?;

            let Resolution {
                candidates: to_update,
                error: deferred_error,
            } = resolve_all(&install_conn, &repo_conn)?;

            if let Some(error) = deferred_error {
                return Err(error);
            }

            if to_update.is_empty() {
                println!("All packages are up to date");
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

                let tempdir = crate::install::install::staging_directory(&install_dir)?;

                update_one(
                    &package_id,
                    &package.groups,
                    &repo_conn,
                    &mut install_conn,
                    &PathBuf::from(package.sky_path),
                    tempdir,
                    &install_dir,
                )?;
            }

            Ok(())
        }
        (Some(_), Some(_)) => {
            bail!("Specify either a package or group to update, not both");
        }
        (None, Some(group)) => {
            let repo_conn =
                Connection::open(&config.repo_db).context("Failed to open repository database")?;
            let mut install_conn =
                Connection::open(&config.install_db).context("Failed to open install database")?;
            clear_update_log(&mut install_conn).context("Failed to clear update log")?;
            let Resolution {
                candidates: to_update,
                error: deferred_error,
            } = resolve_group(&repo_conn, &install_conn, group)
                .context("Failed to resolve group")?;

            if let Some(error) = deferred_error {
                return Err(error);
            }

            let packages = to_update
                .iter()
                .map(|package| (package.name.as_str(), package.version.as_str()))
                .collect::<Vec<_>>();
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

                let tempdir = crate::install::install::staging_directory(&install_dir)?;

                update_one(
                    &package_id,
                    &package.groups,
                    &repo_conn,
                    &mut install_conn,
                    &PathBuf::from(package.sky_path),
                    tempdir,
                    &install_dir,
                )?;
            }

            Ok(())
        }
    }
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
    updated_groups: &Vec<String>,
    repo_conn: &Connection,
    install_conn: &mut Connection,
    sky_path: &PathBuf,
    tempdir: TempDir,
    install_dir: &PathBuf,
) -> Result<()> {
    let config = load_config().context("Failed to load configuration")?;

    let (_, _, manifest, _) = read_sky(sky_path).context("Failed to read sky file")?;
    let new_paths: HashSet<PathBuf> = parse_metadata_lines(&manifest)
        .context("Failed to parse manifest lines")?
        .into_iter()
        .map(|line| line.path)
        .collect();
    let mut owned_stmt = install_conn
        .prepare("SELECT path FROM owned_files WHERE package_name = ? AND architecture = ?")?;
    let old_paths: Vec<String> = owned_stmt
        .query_map(
            params![package_id.name, package_id.architecture.to_string()],
            |row| row.get(0),
        )?
        .collect::<rusqlite::Result<_>>()?;
    drop(owned_stmt);

    let state = PackageState::NotInstalled {
        package_id: package_id.clone(),
        install_conn,
        sky_path,
        tempdir: &tempdir,
        config: &config,
        repo_conn,
        install_dir,
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
    let staging_root = guard
        .staging_dir_path()
        .context("Rollback staging directory is unavailable")?
        .to_path_buf();
    for old_path in old_paths {
        let relative = PathBuf::from(old_path);
        if new_paths.contains(&relative) {
            continue;
        }

        ensure_no_symlink_components(install_dir, &relative, true)?;
        let install_path = install_dir.join(&relative);
        let metadata = match fs::symlink_metadata(&install_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).context("Failed to inspect obsolete owned path"),
        };
        if !metadata.is_file() && !metadata.file_type().is_symlink() {
            continue;
        }

        let staged_path = staging_root.join(&relative);
        fs::create_dir_all(
            staged_path
                .parent()
                .context("Failed to get obsolete path's staging parent")?,
        )
        .context("Failed to create obsolete path's staging parent")?;
        guard.record_old_file(relative);
        renameat_with(CWD, install_path, CWD, staged_path, RenameFlags::NOREPLACE)
            .context("Failed to stage obsolete owned path")?;
    }

    update_installed_package(
        install_conn,
        repo_conn,
        &metadata,
        &package_id.clone(),
        updated_groups,
        &sky_path.to_string_lossy(),
        &install_dir.to_string_lossy(),
        &created_dirs,
    )?;

    guard.disarm();
    Ok(())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
