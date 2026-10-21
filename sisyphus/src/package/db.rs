use anyhow::{Context, Result};
use common::{db::db::create_packages_table, types::metadata::Metadata};
use rusqlite::{Connection, params};

use crate::types::{package::Package, runtime_config::RuntimeConfig};

pub fn insert_package(
    runtime_config: &RuntimeConfig,
    package: &Package,
    sky_path: &str,
    metadata: &Metadata,
) -> Result<()> {
    let db_path = &runtime_config.repo_db_path;

    let mut conn =
        Connection::open(db_path).context("Failed to open connection to repository database")?;

    create_packages_table(&mut conn)?;

    // Republishing a package is a delete-then-reseed: the repository holds one
    // row per (name, architecture), regardless of version or sky path.
    let transaction = conn.transaction()?;
    transaction
        .execute(
            "DELETE FROM packages WHERE name = ? AND architecture = ?",
            params![package.name, package.architecture.to_string()],
        )
        .context("Failed to remove previously published package")?;

    transaction.execute("INSERT INTO packages (name, version, release, architecture, sky_path, recipe_sha256, built_at, builder, deps, makedeps) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", params![package.name, package.version, package.release, package.architecture.to_string(), sky_path, metadata.build.recipe_sha256, metadata.build.built_at, metadata.build.builder, serde_saphyr::to_string(&package.deps).context("Failed to serialize deps to YAMl")?, serde_saphyr::to_string(&package.makedeps).context("Failed to serialize makedeps to YAMl")?]).context("Failed to insert package into the repository database")?;

    transaction
        .commit()
        .context("Failed to commit changes to database")?;

    Ok(())
}
