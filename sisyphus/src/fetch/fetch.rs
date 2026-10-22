use std::path::PathBuf;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    checksum::verify::verify,
    fetch::{db::insert_source, http::HttpFetcher},
    types::{
        fetcher::{FetchedSource, Fetcher},
        package::Package,
        runtime_config::RuntimeConfig,
    },
};

pub fn fetch(
    package: &Package,
    verify_checksum: bool,
    runtime_config: &RuntimeConfig,
) -> Result<FetchedSource> {
    let download_path = runtime_config.fetch_cache.clone();

    if runtime_config.cache_db_path.exists() {
        let conn = Connection::open(&runtime_config.cache_db_path)
            .context("Failed to open connection to the cache database")?;
        let mut stmt = conn
            .prepare("SELECT path FROM sources WHERE url = ? AND sha256 = ?")
            .context("Failed to prepare cache lookup query")?;

        let source_path: Option<String> = stmt
            .query_row(
                params![&package.source.url, &package.source.sha256],
                |row| row.get(0),
            )
            .optional()
            .context("Failed to query the cache database")?;

        if let Some(path) = source_path
            && PathBuf::from(&path).exists()
        {
            println!("Cache hit: {}", path);

            verify(&PathBuf::from(&path), package).context("Failed to verify cached source")?;
            return Ok(FetchedSource::Archive(PathBuf::from(path)));
        }
    }

    let fetcher = Box::new(HttpFetcher {
        url: package.source.url.clone(),
        destination: download_path.join(&package.name),
    }) as Box<dyn Fetcher>;

    let (fetched, fetched_at) = fetcher.fetch().context("Failed to fetch source")?;
    match fetched {
        FetchedSource::Archive(ref path) => {
            if verify_checksum {
                verify(path, package).context("Failed to verify fetched source")?;
            }
            insert_source(
                &package.source.url,
                &package.source.sha256,
                &path.to_string_lossy().into_owned(),
                &fetched_at,
                runtime_config,
            )
            .context("Failed to record source in the cache database")?;
        }
    }

    Ok(fetched)
}
