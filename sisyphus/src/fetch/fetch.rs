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

use rusqlite::{Connection, params};
use url::Url;

pub fn fetch(
    package: &Package,
    verify_checksum: bool,
    runtime_config: &RuntimeConfig,
) -> Result<(FetchedSource, PathBuf), FetchError> {
    let download_path = runtime_config.fetch_cache.clone();
    let parsed_url = Url::parse(&package.source.url)?;

    let conn = Connection::open(&runtime_config.db_path)?;
    let mut stmt = conn.prepare("SELECT path FROM sources WHERE name = ? AND sha256 = ?")?;
    let path: Option<String> = stmt
        .query_row(params![&package.name, &package.source.sha256], |row| {
            row.get(0)
        })?;

    if !path.is_none() {
        return Ok((
            FetchedSource::Archive(PathBuf::from(path.unwrap())),
            download_path,
        ));
    }

    let fetcher = if package.source.url.ends_with("git") {
        Box::new(GitFetcher {
            url: package.source.url.clone(),
            destination: download_path.clone(),
        }) as Box<dyn Fetcher>
    } else {
        Box::new(HttpFetcher {
            url: package.source.url.clone(),
            destination: download_path.clone(),
        }) as Box<dyn Fetcher>
    };
    let (fetched, fetched_at) = fetcher.fetch()?;
    if verify_checksum && !parsed_url.path().trim_end_matches("/").ends_with(".git") {
        match fetched {
            FetchedSource::Archive(ref path) => verify(&path, package)?,
            FetchedSource::Dir(_) => {}
        }
    }

    insert_source(
        &package.source.url,
        &package.source.sha256,
        &download_path.to_string_lossy().into_owned(),
        &fetched_at,
        runtime_config,
    )?;

    Ok((fetched, download_path))
}
