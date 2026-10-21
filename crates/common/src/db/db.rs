use anyhow::{Context, Result};
use rusqlite::Connection;

pub fn create_packages_table(conn: &mut Connection) -> Result<()> {
    let transaction = conn
        .transaction()
        .context("Failed to construct transaction from database connection")?;

    transaction.execute("CREATE TABLE IF NOT EXISTS packages (name TEXT NOT NULL, version TEXT NOT NULL, release INTEGER NOT NULL, architecture TEXT NOT NULL, sky_path TEXT PRIMARY KEY, recipe_sha256 TEXT NOT NULL, built_at TEXT NOT NULL, builder TEXT NOT NULL, deps TEXT NOT NULL, makedeps TEXT NOT NULL,
           UNIQUE (name, architecture))", []).context("Failed to create packages table in repository database")?;

    transaction
        .commit()
        .context("Failed to commit database transaction")?;

    Ok(())
}
