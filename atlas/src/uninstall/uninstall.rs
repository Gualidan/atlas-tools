use std::{env::consts::ARCH, path::PathBuf};

use anyhow::{Context as AnyhowContext, Result, anyhow};
use common::types::{
    config::SisyphusConfig,
    package::{Architectures, PackageIdentity},
};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    cli::cli::UninstallArgs,
    install::db::ensure_install_schema,
    types::{
        context::Context,
        runtime_config::RuntimeConfig,
        states::{Installed, State},
    },
};

pub fn uninstall(
    runtime_config: &RuntimeConfig,
    args: UninstallArgs,
    settings: &SisyphusConfig,
) -> Result<()> {
    let repo_conn = Connection::open(&runtime_config.repo_db)
        .context("Failed to open connection to repository database")?;
    let install_conn = Connection::open(&runtime_config.install_db)
        .context("Failed to open connection to install database")?;
    let (install_dir, version, release, sky_path) =
        look_up_installed_package(&install_conn, &args.package)
            .context("Failed to look up installed package")?;

    let package_id = PackageIdentity {
        name: args.package,
        version,
        release,
        architecture: Architectures::try_from(ARCH.to_string())?,
    };

    let mut ctx = Context {
        package_id: &package_id,
        sky_path: PathBuf::from(sky_path),
        sky_repo: runtime_config.sky_repo.clone(),
        install_dir: PathBuf::from(install_dir),
        temp_dir: None,
        settings,
        dependencies: vec![],
        repo_conn: &repo_conn,
        install_conn: &install_conn,
        metadata: None,
        manifest_paths: None,
    };

    let state = Box::new(Installed) as Box<dyn State>;
    state.remove(&mut ctx)?;

    Ok(())
}

pub fn look_up_installed_package(
    conn: &Connection,
    name: &String,
) -> Result<(String, String, u32, String)> {
    ensure_install_schema(conn)?;
    let mut stmt  = conn.prepare("SELECT install_dir, version, release, sky_path FROM installed_packages WHERE name = ? AND architecture = ?")?;
    let Some((install_dir, version, release, sky_path)) = stmt
        .query_row(params![name, ARCH], |row| {
            let install_dir: String = row.get(0)?;
            let version: String = row.get(1)?;
            let release: u32 = row.get(2)?;
            let sky_path: String = row.get(3)?;

            Ok((install_dir, version, release, sky_path))
        })
        .optional()?
    else {
        return Err(anyhow!("Package {name} not found"));
    };

    Ok((install_dir, version, release, sky_path))
}
