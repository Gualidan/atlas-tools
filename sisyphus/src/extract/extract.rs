use anyhow::{Context, Result};
use compress_tools::{Ownership, uncompress_archive};
use std::fs::File;
use std::path::PathBuf;
use tempfile::{Builder, TempDir};

use crate::{
    extract::source_root::find_source_root,
    types::fetcher::FetchedSource,
};

pub fn extract(fetched: FetchedSource) -> Result<(TempDir, PathBuf)> {
    let destination = Builder::new()
        .prefix("sisyphus_extract_")
        .tempdir()
        .context("Failed to create extraction directory")?;
    match fetched {
        FetchedSource::Archive(path) => {
            let reader = File::open(path).context("Failed to open fetched archive")?;

            uncompress_archive(reader, destination.path(), Ownership::Ignore)
                .context("Failed to uncompress archive")?;
            let source_root = find_source_root(&destination.path().to_path_buf())?;
            Ok((destination, source_root))
        }
        FetchedSource::Dir(_dir) => {
            let source_root = find_source_root(&destination.path().to_path_buf())?;
            Ok((destination, source_root))
        }
    }
}
