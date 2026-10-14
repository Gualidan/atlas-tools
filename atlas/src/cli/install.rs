use anyhow::{Context as AnyhowContext, Result};
use common::types::package::{Architectures, Source};
use std::{env::consts::ARCH, path::PathBuf};

use common::types::{config::load_config, package::Package};
use rusqlite::{Connection, OptionalExtension, params};

use tempfile::Builder;

use crate::{
    cli::cli::InstallArgs,
    types::{
        context::Context,
        runtime_config::RuntimeConfig,
        states::{NotInstalled, State},
    },
};

pub fn install(
    args: &InstallArgs,
    runtime_config: &RuntimeConfig,
    install_dir: PathBuf,
) -> Result<()> {
    let conn = Connection::open(&runtime_config.repo_db)
        .context("Failed to open connection to the repository database")?;

    let mut stmt = conn.prepare("SELECT name, version, release, architecture, sky_path FROM packages WHERE name = ? AND architecture = ?").context("Failed to prepare database query")?;
    let (name, version, release, architecture, mut sky_path): (
        String,
        String,
        u32,
        String,
        String,
    ) = stmt
        .query_row(params![&args.package, ARCH], |row| {
            let name: String = row.get(0)?;
            let version: String = row.get(1)?;
            let release: u32 = row.get(2)?;
            let architecture: String = row.get(3)?;
            let sky_path: String = row.get(4)?;

            Ok((name, version, release, architecture, sky_path))
        })
        .optional()?
        .with_context(|| format!("Package {} not found", args.package))?;
    if let Some(path) = &args.sky_path {
        sky_path = path.to_string_lossy().to_string();
    }

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
        .tempdir()
        .context("Failed to create temporary directory")?;

    let mut ctx = Context {
        package: &package,
        sky_path: PathBuf::from(sky_path),
        sky_repo: runtime_config.sky_repo.to_owned(),
        install_dir,
        temp_dir: temp_dir,
        settings: &settings,
        dependencies: vec![],
        conn: &conn,
    };

    let state = Box::new(NotInstalled) as Box<dyn State>;

    let state = state.verify(&mut ctx)?;
    let state = state.stage(&mut ctx)?;
    state.commit(&mut ctx)?;

    Ok(())
}
