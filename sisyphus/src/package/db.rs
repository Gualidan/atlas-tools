use anyhow::{Context, Result};
use common::types::{metadata::Metadata, package::Package};
use rusqlite::{Connection, params};

use crate::types::runtime_config::RuntimeConfig;

pub fn insert_package(
    runtime_config: &RuntimeConfig,
    package: &Package,
    sky_path: &str,
    metadata: &Metadata,
) -> Result<()> {
    let db_path = &runtime_config.repo_db_path;

    let mut conn =
        Connection::open(db_path).context("Failed to open connection to repository database")?;
    let transaction = conn.transaction()?;

    transaction.execute("CREATE TABLE IF NOT EXISTS packages (name TEXT NOT NULL, version TEXT NOT NULL, release INTEGER NOT NULL, architecture TEXT NOT NULL, sky_path TEXT PRIMARY KEY, recipe_sha256 TEXT NOT NULL, built_at TEXT NOT NULL, builder TEXT NOT NULL, deps TEXT NOT NULL, makedeps TEXT NOT NULL)", []).context("Failed to create packages table in repository database")?;

    transaction.execute("INSERT OR REPLACE INTO packages (name, version, release, architecture, sky_path, recipe_sha256, built_at, builder, deps, makedeps) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", params![package.name, package.version, package.release, package.architecture.to_string(), sky_path, metadata.build.recipe_sha256, metadata.build.built_at, metadata.build.builder, serde_saphyr::to_string(&package.deps).context("Failed to serialize deps to YAMl")?, serde_saphyr::to_string(&package.makedeps).context("Failed to serialize makedeps to YAMl")?]).context("Failed to insert package into the repository database")?;

    transaction
        .commit()
        .context("Failed to commit changes to database")?;

    Ok(())
}
