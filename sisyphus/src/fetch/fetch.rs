use std::path::PathBuf;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    checksum::verify::verify,
    fetch::{db::insert_source, http::HttpFetcher},
    types::{fetcher::Fetcher, package::Package, runtime_config::RuntimeConfig},
};

pub enum FetchMode {
    Build,
    Checksum,
}

pub fn fetch(
    package: &Package,
    fetch_mode: FetchMode,
    runtime_config: &RuntimeConfig,
) -> Result<PathBuf> {
    let download_path = runtime_config.fetch_cache.clone();

    match fetch_mode {
        FetchMode::Build => {
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

                if let Some(path) = source_path {
                    let path = PathBuf::from(&path);
                    if path.exists() {
                        println!("Cache hit: {}", path.display());
                        verify(&path, package).context("Failed to verify cached source")?;
                        return Ok(path);
                    }
                }
            }

            let (fetched, fetched_at) = fetch_payload(
                package.source.url.to_string(),
                download_path.join(&package.name),
            )?;

            verify(&fetched, package).context("Failed to verify fetched source")?;

            insert_source(
                &package.source.url,
                &package.source.sha256,
                &fetched.to_string_lossy().into_owned(),
                &fetched_at,
                runtime_config,
            )
            .context("Failed to record source in the cache database")?;

            Ok(fetched)
        }
        FetchMode::Checksum => {
            let (fetched, _fetched_at) = fetch_payload(
                package.source.url.clone(),
                download_path.join(&package.name),
            )?;

            Ok(fetched)
        }
    }
}

fn fetch_payload(url: String, download_path: PathBuf) -> Result<(PathBuf, String)> {
    let fetcher = Box::new(HttpFetcher {
        url,
        destination: download_path,
    }) as Box<dyn Fetcher>;

    let (fetched, fetched_at) = fetcher.fetch().context("Failed to fetch source")?;

    Ok((fetched, fetched_at))
}
