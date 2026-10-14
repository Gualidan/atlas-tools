use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::types::runtime_config::RuntimeConfig;

pub fn insert_source(
    url: &String,
    sha256: &Option<String>,
    path: &String,
    fetched_at: &String,
    runtime_config: &RuntimeConfig,
) -> Result<()> {
    let mut conn = Connection::open(&runtime_config.cache_db_path)
        .context("Failed to open connection to the cache database")?;
    let transaction = conn
        .transaction()
        .context("Failed to start a cache database transaction")?;

    transaction.execute("CREATE TABLE IF NOT EXISTS sources (url TEXT PRIMARY KEY, sha256 TEXT, path TEXT NOT NULL, fetched_at TEXT NOT NULL)", []).context("Failed to create sources table in cache database")?;

    transaction
        .execute(
            "INSERT OR REPLACE INTO sources (url, sha256, path, fetched_at) VALUES (?, ?, ?, ?)",
            params![url, sha256, path, fetched_at],
        )
        .context("Failed to insert source into the cache database")?;

    transaction
        .commit()
        .context("Failed to commit changes to cache database")?;

    Ok(())
}
