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
    let conn = Connection::open(&runtime_config.db_path)?;
    conn.execute("BEGIN", [])?;

    conn.execute("CREATE TABLE IF NOT EXISTS sources (url TEXT PRIMARY KEY, sha256 TEXT, path TEXT NOT NULL, fetched_at INTEGER NOT NULL)", [])?;

    conn.execute(
        "INSERT OR REPLACE INTO sources (url, sha256, path, fetched_at) VALUES (?, ?, ?, ?)",
        params![url, sha256, path, fetched_at],
    )?;

    conn.execute("COMMIT", [])?;

    Ok(())
}
