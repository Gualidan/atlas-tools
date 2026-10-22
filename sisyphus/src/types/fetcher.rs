use anyhow::Result;
use std::path::PathBuf;

/// Kept for future use when multiple source types will be supported
pub trait Fetcher {
    fn fetch(self: Box<Self>) -> Result<(PathBuf, String)>;
}
