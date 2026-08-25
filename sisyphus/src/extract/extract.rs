use std::fs::File;

use compress_tools::{Ownership, uncompress_archive};
use copy_dir::copy_dir;
use tempfile::{Builder, TempDir};

use crate::types::{error::ExtractError, fetcher::FetchedSource};

pub fn extract(fetched: FetchedSource) -> Result<TempDir, ExtractError> {
    let destination = Builder::new().prefix("sisyphus_extract_").tempdir()?;
    match fetched {
        FetchedSource::Archive(path) => {
            let reader = File::open(path)?;

            uncompress_archive(reader, destination.path(), Ownership::Ignore)?;
            Ok(destination)
        }
        FetchedSource::Dir(dir) => {
            copy_dir(dir, destination.path())?;
            Ok(destination)
        }
    }
}
