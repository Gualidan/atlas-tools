use std::{fs::File, path};

use compress_tools::{Ownership, uncompress_archive};
use tempfile::TempDir;

use crate::types::{error::ExtractError, fetcher::FetchedSource};

pub fn extract(fetched: FetchedSource, destination: TempDir) -> Result<TempDir, ExtractError> {
    match fetched {
        FetchedSource::Archive(path) => {
            let reader = File::open(path)?;

            uncompress_archive(reader, destination.path(), Ownership::Ignore)?;
            Ok(destination)
        }
        FetchedSource::Dir(dir) => Ok(dir),
    }
}
