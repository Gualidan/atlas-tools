use anyhow::{Context as AnyhowContext, Result, anyhow};
use common::types::package::{Architectures, PackageIdentity};
use std::{env::consts::ARCH, fs::create_dir_all, path::PathBuf};

use common::types::config::load_config;
use rusqlite::{Connection, OptionalExtension, params};

use tempfile::Builder;

use crate::{
    cli::cli::InstallArgs,
    install::{
        db::{ensure_install_schema, insert_installed_package},
        resolve::resolve_install_order,
    },
    types::{
        context::Context, guards::SwapBackGuard, package_state::PackageState,
        runtime_config::RuntimeConfig,
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

    let mut stmt = conn
        .prepare("SELECT name FROM packages WHERE name = ? AND architecture = ?")
        .context("Failed to prepare database query")?;
    let Some(name): Option<String> = stmt
        .query_row(params![&args.package, ARCH], |row| row.get(0))
        .optional()?
    else {
        return Err(anyhow!("Package {} not found", &args.package.unwrap()));
    };

    let architecture = Architectures::try_from(ARCH.to_string())?;
    let settings = load_config().context("Failed to load settings")?;

    let to_install = resolve_install_order(&conn, name.clone(), architecture)?;

    create_dir_all(&install_dir).context("Failed to create install_dir's parents")?;

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
        };

        let install_dir = install_dir.join(format!(
            "{}-{}-{}",
            &package.name, &package.version, package.release
        ));
        let install_dir_existed = install_dir.exists();

        let package_id = PackageIdentity {
            name: package.name,
            version: package.version,
            release: package.release,
            architecture: package.architecture,
        };

        let temp_dir = Builder::new()
            .prefix("sisyphus_install_tmp_dir_")
            .tempdir_in(
                install_dir
                    .parent()
                    .context("Failed to get install dir's parent")?,
            )
            .context("Failed to create temporary directory")?;

        let staging_dir = temp_dir.path().to_path_buf();

        let mut ctx = Context {
            package_id: &package_id,
            sky_path: PathBuf::from(&package.sky_path),
            sky_repo: runtime_config.sky_repo.clone(),
            install_dir: install_dir.clone(),
            temp_dir: Some(temp_dir),
            settings: &settings,
            repo_conn: &conn,
            install_conn: &install_conn,
        };

        install_one(&mut ctx, runtime_config, install_dir_existed, &staging_dir)?;
    }

    Ok(())
}

pub fn install_one(
    ctx: &mut Context,
    runtime_config: &RuntimeConfig,
    install_dir_existed: bool,
    staging_dir: &PathBuf,
) -> Result<()> {
    let state = PackageState::NotInstalled;

    let state = state.verify(ctx)?;
    let state = state.stage(ctx)?;
    let (_state, metadata) = state.commit(ctx)?;
    let mut guard = SwapBackGuard::new(
        PathBuf::from(&ctx.install_dir),
        staging_dir.to_path_buf(),
        install_dir_existed,
    );

    insert_installed_package(
        runtime_config,
        &metadata,
        &ctx.package_id,
        ctx.install_dir.to_string_lossy().to_string(),
        &ctx.sky_path.to_str().unwrap(),
    )?;

    guard.disarm();
    Ok(())
}
