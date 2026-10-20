use anyhow::{Context as AnyhowContext, Result, anyhow};
use common::types::package::{Architectures, Source};
use std::{env::consts::ARCH, path::PathBuf};

use common::types::{config::load_config, package::Package};
use rusqlite::{Connection, OptionalExtension, params};

use tempfile::Builder;

use crate::{
    cli::cli::InstallArgs,
    install::db::{ensure_install_schema, insert_installed_package},
    types::{
        context::Context,
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

    let package = Package {
        schema: 1,
        name,
        version,
        release,
        architecture: Architectures::try_from(architecture)?,
        source: Source {
            url: String::new(),
            sha256: None,
        },
        makedeps: vec![],
        deps: vec![],
        prepare: None,
        build: None,
        check: None,
        package: String::new(),
    };
    let settings = load_config().context("Failed to load settings")?;

    let temp_dir = Builder::new()
        .prefix("sisyphus_install_tmp_dir_")
        .tempdir_in(PathBuf::from("/var/tmp"))
        .context("Failed to create temporary directory")?;

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
        package: &package,
        sky_path: PathBuf::from(sky_path),
        sky_repo: runtime_config.sky_repo.to_owned(),
        install_dir: install_dir.to_owned(),
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

    insert_installed_package(
        runtime_config,
        ctx.metadata.context("Metadata should not be empty.")?,
        &package,
        install_dir.to_string_lossy().to_string(),
    )?;

    Ok(())
}
