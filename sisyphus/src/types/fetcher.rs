use anyhow::Result;
use std::path::PathBuf;

pub trait Fetcher {
    fn fetch(self: Box<Self>) -> Result<(PathBuf, String)>;
}
