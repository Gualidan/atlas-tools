use std::{env::consts::ARCH, fs::File, path::PathBuf};

use anyhow::{Context as AnyhowContext, Result, anyhow};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    cli::confirmation::{Action, confirmation_prompt},
    install::db::ensure_install_schema,
    types::{contexts::RemoveContext, package_state::PackageState, runtime_config::RuntimeConfig},
    uninstall::{
        db::remove_uninstalled_package,
        resolve::{UninstallReport, packages_to_uninstall},
    },
};

/// Removes the packages resolved from `name` by [`packages_to_uninstall`],
/// deleting each directory before its install database records. Lookups are by
/// name without an architecture filter. A missing directory is tolerated.
///
/// When `prompt` is true, requests confirmation for the resolved list unless
/// `set_assume_yes` is enabled. Declining succeeds without removing packages.
/// Resolution and dependent checks run before confirmation.
///
/// Returns an error for an absent package or recorded dependent, and propagates
/// resolution, confirmation, database, and other filesystem errors.
/// A database failure after directory removal does not restore the files.
///
/// # Errors
///
/// Returns an error if the package is absent or has dependents, or if resolution,
/// confirmation, database updates, or filesystem removal fails.
pub fn uninstall(
    runtime_config: &RuntimeConfig,
    name: &String,
    prompt: bool,
) -> Result<UninstallReport> {
    let mut install_conn = if runtime_config.install_db.exists() {
        Connection::open(&runtime_config.install_db)
            .context("Failed to open connection to install database, perhaps it is missing?")?
    } else {
        File::create_new(&runtime_config.install_db)?;
        let conn = Connection::open(&runtime_config.install_db)
            .context("Failed to open connection to install database, perhaps it is missing?")?;
        ensure_install_schema(&conn)?;

        conn
    };

    let mut stmt = install_conn
        .prepare("SELECT name FROM installed_packages WHERE name = ?")
        .context("Failed to prepare database query")?;
    let Some(name): Option<String> = stmt.query_row(params![name], |row| row.get(0)).optional()?
    else {
        return Err(anyhow!("Package `{name}` not found"));
    };

    drop(stmt);

    let (to_uninstall, report) = packages_to_uninstall(&install_conn, name.clone())?;
    let packages = to_uninstall
        .iter()
        .map(|(pkg, _)| (pkg.name.as_str(), pkg.version.as_str()))
        .collect::<Vec<(&str, &str)>>();

    if !report.skipped.is_empty() {
        println!("Skipped packages:");
        for skipped in &report.skipped {
            println!("  - {} (required by {})", skipped.name, skipped.required_by);
        }
    }

    if prompt && !confirmation_prompt(packages, 4, &Action::Uninstall)? {
        return Ok(report);
    }

    for (package, install_dir) in to_uninstall {
        let state = PackageState::Installed;
        let ctx = &mut RemoveContext {
            package_id: &package,
            install_conn: &install_conn,
            install_dir: PathBuf::from(install_dir),
        };

        state.remove(ctx)?;

        remove_uninstalled_package(&mut install_conn, &package)?;
    }

    Ok(report)
}

/// Looks up an installed package's directory, version, and release for the host architecture.
///
/// # Errors
///
/// Returns an error if the install schema cannot be ensured or the package is absent.
pub fn look_up_installed_package(
    conn: &Connection,
    name: &String,
) -> Result<(String, String, u32)> {
    ensure_install_schema(conn)?;
    let mut stmt  = conn.prepare("SELECT install_dir, version, release FROM installed_packages WHERE name = ? AND architecture = ?")?;
    let Some((install_dir, version, release)) = stmt
        .query_row(params![name, ARCH], |row| {
            let install_dir: String = row.get(0)?;
            let version: String = row.get(1)?;
            let release: u32 = row.get(2)?;

            Ok((install_dir, version, release))
        })
        .optional()?
    else {
        return Err(anyhow!("Package `{name}` not found"));
    };

    Ok((install_dir, version, release))
}
