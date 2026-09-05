use crate::{
    checksum::verify::verify,
    fetch::{git::GitFetcher, http::HttpFetcher},
    types::{
        error::FetchError,
        fetcher::{FetchedSource, Fetcher},
        package::Package,
    },
};
use tempfile::{Builder, TempDir};

pub fn fetch(
    package: &Package,
    verify_checksum: bool,
) -> Result<(FetchedSource, TempDir), FetchError> {
    let download_dir = Builder::new().prefix("sisyphus_download_").tempdir()?;
    let download_path = download_dir.path().to_path_buf();

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
    let fetched = fetcher.fetch()?;
    if verify_checksum {
        match fetched {
            FetchedSource::Archive(ref path) => verify(&path, package)?,
            FetchedSource::Dir(_) => {}
        }
    }

    Ok((fetched, download_dir))
}
