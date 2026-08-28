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

    let fetched = match package
        .sources
        .url
        .as_str()
        .get(package.sources.url.as_str().len().saturating_sub(3)..)
        .unwrap_or("")
    {
        "git" => GitFetcher {
            url: package.sources.url.clone(),
            destination: download_path.clone(),
        }
        .fetch()?,
        _ => {
            let fetcher = HttpFetcher {
                url: package.sources.url.clone(),
                destination: download_path.clone(),
            }
            .fetch()?;
            if verify_checksum {
                match fetcher {
                    FetchedSource::Archive(ref path) => verify(&path, package)?,
                    FetchedSource::Dir(_) => {}
                }
            }

            fetcher
        }
    };

    Ok((fetched, download_dir))
}
