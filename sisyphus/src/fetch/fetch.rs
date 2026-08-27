use crate::{
    fetch::{git::GitFetcher, http::HttpFetcher},
    types::{
        error::FetchError,
        fetcher::{FetchedSource, Fetcher},
        package::Package,
    },
};
use tempfile::{Builder, TempDir};

pub fn fetch(package: &Package) -> Result<(FetchedSource, TempDir), FetchError> {
    let download_dir = Builder::new().prefix("sisyphus_download_").tempdir()?;
    let download_path = download_dir.path().to_path_buf();

    let fetched = match package.download_method.as_str() {
        "git" => GitFetcher {
            url: package.url.clone(),
            destination: download_path.clone(),
        }
        .fetch()?,
        _ => HttpFetcher {
            url: package.url.clone(),
            destination: download_path.clone(),
        }
        .fetch()?,
    };

    Ok((fetched, download_dir))
}
