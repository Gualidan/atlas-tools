use anyhow::{Context, Result};
use compress_tools::{Ownership, uncompress_archive};

use std::fs::File;
use std::path::PathBuf;
use tempfile::{Builder, TempDir};

use crate::extract::source_root::find_source_root;

/// Unpacks a fetched archive into a temporary directory and locates its source root.
///
/// # Errors
///
/// Returns an error if the temporary directory cannot be created, the archive
/// cannot be opened or extracted, or the source root cannot be determined.
pub fn extract(fetched: PathBuf) -> Result<(TempDir, PathBuf)> {
    let destination = Builder::new()
        .prefix("sisyphus_extract_")
        .tempdir()
        .context("Failed to create extraction directory")?;

    let reader = File::open(fetched).context("Failed to open fetched archive")?;

    uncompress_archive(reader, destination.path(), Ownership::Ignore)
        .context("Failed to uncompress archive")?;
    let source_root = find_source_root(destination.path())?;
    Ok((destination, source_root))
}
