use crate::{
    fetch::{git::GitFetcher, http::HttpFetcher},
    types::{
        error::FetchError,
        fetcher::{FetchedSource, Fetcher},
        package::Package,
    },
};
use tempfile::Builder;

pub fn fetch(package: Package) -> Result<FetchedSource, FetchError> {
    let download_dir = Builder::new().prefix("sisyphus_download_").tempdir()?;
    match package.download_method.as_str() {
        "git" => GitFetcher {
            url: package.url.clone(),
            destination: download_dir,
        }
        .fetch(),
        _ => HttpFetcher {
            url: package.url.clone(),
            destination: download_dir,
        }
        .fetch(),
    }
}
