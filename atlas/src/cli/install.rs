use common::types::package::{Architectures, Source};
use std::{env::consts::ARCH, path::PathBuf};

use common::types::{package::Package, settings::load_settings};
use rusqlite::{Connection, OptionalExtension, params};

use tempfile::Builder;

use crate::{
    cli::cli::InstallArgs,
    types::{context::Context, error::Cli, runtime_config::RuntimeConfig},
};

pub fn install(
    args: &InstallArgs,
    runtime_config: &RuntimeConfig,
    install_dir: PathBuf,
) -> Result<(), Cli> {
    let conn = Connection::open(&runtime_config.repo_db)?;

    let mut stmt = conn.prepare("SELECT name, version, release, architecture, sky_path FROM packages WHERE name = ? AND architecture = ?")?;
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
        .ok_or_else(|| Cli::PackageNotFound(args.package.to_owned()))?;
    if let Some(path) = &args.sky_path {
        sky_path = path.to_string_lossy().to_string();
    }

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
    let settings = load_settings()?;

    std::fs::create_dir_all(&install_dir)?;
    let temp_dir = Builder::new()
        .prefix("sisyphus_install_tmp_dir_")
        .tempdir()?;

    let context = Context {
        package: &package,
        sky_path: PathBuf::from(sky_path),
        sky_repo: runtime_config.sky_repo.to_owned(),
        install_dir,
        temp_dir: temp_dir,
        settings: &settings,
        dependencies: vec![],
        conn: &conn,
    };

    Ok(())
}
