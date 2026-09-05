use crate::types::error::FetchError;
use std::path::PathBuf;

pub enum FetchedSource {
    Archive(PathBuf),
    Dir(PathBuf),
}

pub trait Fetcher {
    fn fetch(self: Box<Self>) -> Result<FetchedSource, FetchError>;
}
