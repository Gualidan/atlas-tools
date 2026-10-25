use std::{env::consts::ARCH, path::PathBuf};

use anyhow::{Context as AnyhowContext, Result, anyhow};
use common::types::package::{Architectures, PackageIdentity};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    install::db::{ensure_install_schema, remove_uninstalled_package},
    types::{contexts::RemoveContext, package_state::PackageState, runtime_config::RuntimeConfig},
};

/// Removes the named package for the host architecture, then deletes its install
/// database records. A missing install directory is tolerated.
/// Returns an error if the package is absent or has dependents, and propagates
/// database and other filesystem errors. A database failure after directory
/// removal does not restore the files.
pub fn uninstall(runtime_config: &RuntimeConfig, name: &String) -> Result<()> {
    let mut install_conn = Connection::open(&runtime_config.install_db)
        .context("Failed to open connection to install database")?;
    let (install_dir, version, release) = look_up_installed_package(&install_conn, name)
        .context("Failed to look up installed package")?;

    let package_id = PackageIdentity {
        name: name.as_str(),
        version: version.as_str(),
        release,
        architecture: Architectures::try_from(ARCH.to_string())?,
    };

    let mut ctx = RemoveContext {
        package_id: &package_id,
        install_dir: PathBuf::from(install_dir),
        install_conn: &install_conn,
    };

    let state = PackageState::Installed;

    state.remove(&mut ctx)?;

    remove_uninstalled_package(&mut install_conn, &package_id)?;

    Ok(())
}

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
        return Err(anyhow!("Package {name} not found"));
    };

    Ok((install_dir, version, release))
}
