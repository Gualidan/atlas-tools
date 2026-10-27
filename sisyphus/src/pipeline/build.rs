use anyhow::{Context, Result};
use common::types::config::load_config;
use rusqlite::{Connection, OptionalExtension};

use crate::{
    build::{
        build::{build, build_package},
        dependencies::resolve,
    },
    extract::extract::extract,
    fetch::fetch::{FetchMode, fetch},
    package::package::package_gen,
    types::{recipe::Recipe, runtime_config::RuntimeConfig},
};
use std::path::PathBuf;

pub fn run_build(recipe: &PathBuf, runtime_config: Option<&RuntimeConfig>) -> Result<()> {
    let package = Recipe { path: recipe }.parse()?;
    let binding = RuntimeConfig::build(recipe)?;
    let config = load_config().context("Failed to load config")?;
    let runtime_config = runtime_config.unwrap_or(&binding);

    // Abort if package already exists
    if runtime_config.repo_db_path.exists() {
        let conn = Connection::open(&runtime_config.repo_db_path)
            .context("Failed to open connection to repository database")?;
        let mut stmt = conn.prepare("SELECT 1 FROM packages WHERE name = ? AND version = ? AND release = ? AND architecture = ? LIMIT 1").context("Failed to prepare database query")?;
        let package_exists: Option<bool> = stmt
            .query_row(
                [
                    &package.name,
                    &package.version,
                    &package.release.to_string(),
                    &package.architecture.to_string(),
                ],
                |row| row.get(0),
            )
            .optional()
            .context("Failed to query database row")?;

        if package_exists.is_some() {
            let mut stmt = conn.prepare("SELECT sky_path FROM packages where name = ? AND version = ? AND release = ? AND architecture = ? LIMIT 1").context("Failed to prepare database query")?;
            let sky_path: String = stmt
                .query_row(
                    [
                        &package.name,
                        &package.version,
                        &package.release.to_string(),
                        &package.architecture.to_string(),
                    ],
                    |row| row.get(0),
                )
                .context("Failed to query database row")?;

            if PathBuf::from(sky_path).exists() {
                println!(
                    "Package \"{}\" already exists, skipping build",
                    package.name
                );
                return Ok(());
            }
        }
    }

    // Phase 1: Fetch
    let fetched =
        fetch(&package, FetchMode::Build, runtime_config).context("Failed to fetch package")?;

    // Phase 2: Extract
    let (_destination, source_root) = extract(fetched).context("Failed to extract package")?;

    // Phase 3: Dependency resolution / Build
    let build_plan = resolve(runtime_config, &package, &config)
        .context("Failed to resolve package's dependencies")?;

    let pkgdir = build(runtime_config, &package, &source_root, &build_plan)
        .context("Failed to build package")?;

    // Phase 4: Package generation
    package_gen(
        recipe,
        &pkgdir.path().to_path_buf(),
        &package,
        &config.priv_key_path,
        runtime_config,
    )
    .context("Failed to generate sky file")?;

    Ok(())
}

pub fn run_planned_build(
    recipe: &PathBuf,
    runtime_config: &RuntimeConfig,
    artifacts: &[PathBuf],
) -> Result<PathBuf> {
    let package = Recipe { path: recipe }.parse()?;
    let config = load_config().context("Failed to load config")?;

    // Phase 1: Fetch
    let fetched =
        fetch(&package, FetchMode::Build, runtime_config).context("Failed to fetch package")?;

    // Phase 2: Extract
    let (_destination, source_root) = extract(fetched).context("Failed to extract package")?;

    // Phase 3: Build
    let pkgdir = build_package(&package, &source_root, artifacts)
        .with_context(|| format!("Failed to build package `{}`", package.name))?;

    // Phase 4: Package generation
    package_gen(
        recipe,
        &pkgdir.path().to_path_buf(),
        &package,
        &config.priv_key_path,
        runtime_config,
    )
    .context("Failed to generate sky file")
}
