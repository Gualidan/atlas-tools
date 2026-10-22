use anyhow::Result;
use std::path::PathBuf;

#[derive(Debug)]
pub enum FetchedSource {
    Archive(PathBuf),
}

pub trait Fetcher {
    fn fetch(self: Box<Self>) -> Result<(FetchedSource, String)>;
}
