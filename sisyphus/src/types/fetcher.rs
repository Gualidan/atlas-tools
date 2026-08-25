use tempfile::TempDir;

use crate::types::error::FetchError;

use std::path::PathBuf;

pub enum FetchedSource {
    Archive(PathBuf),
    Dir(TempDir),
}

pub trait Fetcher {
    fn fetch(self) -> Result<FetchedSource, FetchError>;
}
