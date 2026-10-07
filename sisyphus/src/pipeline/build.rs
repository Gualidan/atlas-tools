use common::types::settings::load_settings;
use rusqlite::Connection;

use crate::{
    build::{build::build, dependencies::resolve},
    extract::extract::extract,
    fetch::fetch::fetch,
    package::package::package_gen,
    types::{error::BuildPipelineError, recipe::Recipe, runtime_config::RuntimeConfig},
};
use std::path::PathBuf;

pub fn run_build(
    recipe: &PathBuf,
    runtime_config: Option<&RuntimeConfig>,
    nested: bool,
) -> Result<(), BuildPipelineError> {
    let package = Recipe { path: &recipe }.parse()?;
    let binding = RuntimeConfig::build(recipe)?;
    let config = load_settings()?;
    let runtime_config = runtime_config.unwrap_or(&binding);

    // Abort if package already exists
    if runtime_config.db_path.exists() {
        let conn = Connection::open(&runtime_config.db_path)?;
        let mut stmt = conn.prepare("SELECT 1 FROM packages WHERE name = ? AND version = ? AND release = ? AND architecture = ? LIMIT 1")?;
        let package_exists: bool = stmt.query_row(
            &[
                &package.name,
                &package.version,
                &package.release.to_string(),
                &package.architecture.to_string(),
            ],
            |row| row.get(0),
        )?;

        let mut stmt = conn.prepare("SELECT sky_path FROM packages where name = ? AND version = ? AND release = ? AND architecture = ? LIMIT 1")?;
        let sky_path: String = stmt.query_row(
            &[
                &package.name,
                &package.version,
                &package.release.to_string(),
                &package.architecture.to_string(),
            ],
            |row| row.get(0),
        )?;

        if package_exists && PathBuf::from(sky_path).exists() {
            println!(
                "Package \"{}\" already exists, skipping build",
                package.name
            );
            return Ok(());
        }
    }

    // Phase 1: Fetch
    let fetched = fetch(&package, true, runtime_config)?;

    // Phase 2: Extract
    let (_destination, source_root) = extract(fetched)?;

    // Phase 3: Dependency resolution / Build
    let build_plan = if !nested {
        Some(resolve(runtime_config, &package, &config)?)
    } else {
        None
    };

    let pkgdir = build(runtime_config, &package, &source_root, &build_plan)?;

    // Phase 4: Package generation
    package_gen(
        recipe,
        &pkgdir.path().to_path_buf(),
        &package,
        &config.priv_key_path,
        runtime_config,
    )?;

    Ok(())
}
