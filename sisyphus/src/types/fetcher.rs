use common::types::error::FetchError;
use std::path::PathBuf;

#[derive(Debug)]
pub enum FetchedSource {
    Archive(PathBuf),
    Dir(PathBuf),
}

pub trait Fetcher {
    fn fetch(self: Box<Self>) -> Result<(FetchedSource, String), FetchError>;
}
