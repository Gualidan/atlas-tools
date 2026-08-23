use crate::types::error::FetchError;

pub enum FetchedSource<'a> {
    Archive(std::path::PathBuf),
    Dir(&'a tempfile::TempDir),
}

pub trait Fetcher<'a> {
    fn fetch(&self) -> Result<FetchedSource<'a>, FetchError>;
}
