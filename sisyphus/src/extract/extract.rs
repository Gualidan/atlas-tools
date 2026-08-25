use crate::{
    extract::source_root::find_source_root,
    types::{error::ExtractError, fetcher::FetchedSource},
};
use compress_tools::{Ownership, uncompress_archive};
use copy_dir::copy_dir;
use std::fs::File;
use std::path::PathBuf;
use tempfile::{Builder, TempDir};

pub fn extract(fetched: FetchedSource) -> Result<(TempDir, PathBuf), ExtractError> {
    let destination = Builder::new().prefix("sisyphus_extract_").tempdir()?;
    match fetched {
        FetchedSource::Archive(path) => {
            let reader = File::open(path)?;

            uncompress_archive(reader, destination.path(), Ownership::Ignore)?;
            let source_root = find_source_root(&destination.path().to_path_buf())?;
            Ok((destination, source_root))
        }
        FetchedSource::Dir(dir) => {
            copy_dir(dir, destination.path())?;
            let source_root = find_source_root(&destination.path().to_path_buf())?;
            Ok((destination, source_root))
        }
    }
}
