use std::path::PathBuf;

use crate::{
    checksum::verify::verify,
    fetch::{db::insert_source, git::GitFetcher, http::HttpFetcher},
    types::{
        fetcher::{FetchedSource, Fetcher},
        runtime_config::RuntimeConfig,
    },
};
use common::types::{error::FetchError, package::Package};

use rusqlite::{Connection, OptionalExtension, params};

pub fn fetch(
    package: &Package,
    verify_checksum: bool,
    runtime_config: &RuntimeConfig,
) -> Result<FetchedSource, FetchError> {
    let download_path = runtime_config.fetch_cache.clone();

    if runtime_config.cache_db_path.exists() {
        let conn = Connection::open(&runtime_config.cache_db_path)?;
        let mut stmt = if let Some(_sha) = &package.source.sha256 {
            conn.prepare("SELECT path FROM sources WHERE url = ? AND sha256 = ?")?
        } else {
            conn.prepare("SELECT path FROM sources WHERE url = ? AND sha256 IS NULL")?
        };

        let source_path: Option<String> = match &package.source.sha256 {
            Some(sha) => stmt.query_row(params![&package.source.url, sha], |row| row.get(0)),
            None => stmt.query_row(params![&package.source.url], |row| row.get(0)),
        }
        .optional()?;

        if let Some(path) = source_path
            && PathBuf::from(&path).exists()
        {
            println!("Cache hit: {}", path);
            if PathBuf::from(&path).is_dir() {
                return Ok(FetchedSource::Dir(PathBuf::from(path)));
            }

            verify(&PathBuf::from(&path), package)?;
            return Ok(FetchedSource::Archive(PathBuf::from(path)));
        }
    }

    let fetcher = if package.source.url.ends_with("git") {
        Box::new(GitFetcher {
            url: package.source.url.clone(),
            destination: download_path.join(&package.name),
        }) as Box<dyn Fetcher>
    } else {
        Box::new(HttpFetcher {
            url: package.source.url.clone(),
            destination: download_path.join(&package.name),
        }) as Box<dyn Fetcher>
    };

    let (fetched, fetched_at) = fetcher.fetch()?;
    match fetched {
        FetchedSource::Archive(ref path) => {
            if verify_checksum {
                verify(path, package)?;
            }
            insert_source(
                &package.source.url,
                &package.source.sha256,
                &path.to_string_lossy().into_owned(),
                &fetched_at,
                runtime_config,
            )?;
        }
        FetchedSource::Dir(ref path) => insert_source(
            &package.source.url,
            &package.source.sha256,
            &path.to_string_lossy().into_owned(),
            &fetched_at,
            runtime_config,
        )?,
    }

    Ok(fetched)
}
