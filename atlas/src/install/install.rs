use anyhow::{Context as AnyhowContext, Result, anyhow};
use common::types::package::{Architectures, PackageIdentity};
use std::{env::consts::ARCH, fs::create_dir_all, path::PathBuf};

use common::types::config::load_config;
use rusqlite::{Connection, OptionalExtension, params};

use tempfile::Builder;

use crate::{
    cli::cli::InstallArgs,
    install::db::{ensure_install_schema, insert_installed_package},
    types::{
        context::Context,
        guards::SwapBackGuard,
        runtime_config::RuntimeConfig,
        states::{NotInstalled, State},
    },
};

pub fn install(
    args: InstallArgs,
    runtime_config: &RuntimeConfig,
    install_dir: PathBuf,
) -> Result<()> {
    if args.package.is_none() {
        return Err(anyhow!("You must provide a package name to install"));
    }

    let conn = Connection::open(&runtime_config.repo_db)
        .context("Failed to open connection to the repository database")?;

    let mut stmt = conn.prepare("SELECT name, version, release, architecture, sky_path FROM packages WHERE name = ? AND architecture = ?").context("Failed to prepare database query")?;
    let (name, version, release, architecture, sky_path): (String, String, u32, String, String) =
        stmt.query_row(params![&args.package, ARCH], |row| {
            let name: String = row.get(0)?;
            let version: String = row.get(1)?;
            let release: u32 = row.get(2)?;
            let architecture: String = row.get(3)?;
            let sky_path: String = row.get(4)?;

            Ok((name, version, release, architecture, sky_path))
        })
        .optional()?
        .with_context(|| format!("Package {} not found", args.package.unwrap()))?;

    let install_dir = install_dir.join(format!("{}-{}-{}", &name, &version, release));
    let install_dir_existed = install_dir.exists();

    let package_id = PackageIdentity {
        name,
        version,
        release,
        architecture: Architectures::try_from(architecture)?,
    };
    let settings = load_config().context("Failed to load settings")?;

    create_dir_all(
        &install_dir
            .parent()
            .context("Failed to get install dir's parent")?,
    )
    .context("Failed to create install_dir and its parents")?;
    let temp_dir = Builder::new()
        .prefix("sisyphus_install_tmp_dir_")
        .tempdir_in(
            install_dir
                .parent()
                .context("Failed to get install dir's parent")?,
        )
        .context("Failed to create temporary directory")?;
    let staging_dir = temp_dir.path().to_path_buf();

    if let Some(parent) = runtime_config.install_db.parent() {
        std::fs::create_dir_all(parent)
            .context("Failed to create the install database directory")?;
    }
    let install_conn = Connection::open(&runtime_config.install_db)
        .context("Failed to open connection to install database")?;
    install_conn
        .execute("PRAGMA foreign_keys = ON", [])
        .context("Failed to enable foreign key enforcement")?;
    ensure_install_schema(&install_conn).context("Failed to create the install database schema")?;

    let mut ctx = Context {
        package_id: &package_id,
        sky_path: PathBuf::from(sky_path),
        sky_repo: runtime_config.sky_repo.clone(),
        install_dir: install_dir.clone(),
        temp_dir: Some(temp_dir),
        settings: &settings,
        dependencies: vec![],
        repo_conn: &conn,
        install_conn: &install_conn,
        metadata: None,
        manifest_paths: None,
    };

    let state = Box::new(NotInstalled) as Box<dyn State>;

    let state = state.verify(&mut ctx)?;
    let state = state.stage(&mut ctx)?;
    state.commit(&mut ctx)?;

    let mut guard = SwapBackGuard::new(install_dir.clone(), staging_dir, install_dir_existed);

    insert_installed_package(
        runtime_config,
        ctx.metadata.context("Metadata should not be empty.")?,
        &package_id,
        install_dir.to_string_lossy().to_string(),
    )?;

    guard.disarm();
    Ok(())
}
