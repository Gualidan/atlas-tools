use common::types::error::FetchError;
use rusqlite::{Connection, params};

use crate::types::runtime_config::RuntimeConfig;

pub fn insert_source(
    url: &String,
    sha256: &Option<String>,
    path: &String,
    fetched_at: &String,
    runtime_config: &RuntimeConfig,
) -> Result<(), FetchError> {
    let mut conn = Connection::open(&runtime_config.cache_db_path)?;
    let transaction = conn.transaction()?;

    transaction.execute("CREATE TABLE IF NOT EXISTS sources (url TEXT PRIMARY KEY, sha256 TEXT, path TEXT NOT NULL, fetched_at TEXT NOT NULL)", [])?;

    transaction.execute(
        "INSERT OR REPLACE INTO sources (url, sha256, path, fetched_at) VALUES (?, ?, ?, ?)",
        params![url, sha256, path, fetched_at],
    )?;

    transaction.commit()?;

    Ok(())
}
