use anyhow::Result;
use std::path::PathBuf;

/// Kept for future use when multiple source types will be supported
pub trait Fetcher {
    /// Fetches a source and returns its local path and fetch timestamp.
    ///
    /// # Errors
    ///
    /// Returns an error if the source cannot be fetched or written locally.
    fn fetch(self: Box<Self>) -> Result<(PathBuf, String)>;
}
