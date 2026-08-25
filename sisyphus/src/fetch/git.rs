use crate::types::{
    error::FetchError,
    fetcher::{FetchedSource, Fetcher},
};
use std::process::Command;
use tempfile::TempDir;
use which::which;

pub struct GitFetcher {
    pub url: String,
    pub destination: TempDir,
}

impl Fetcher for GitFetcher {
    fn fetch(self) -> Result<FetchedSource, FetchError> {
        which("git")?;

        let output = Command::new("git")
            .arg("clone")
            .arg(&self.url)
            .arg(&self.destination.path())
            .output()?;

        if !output.status.success() {
            return Err(FetchError::CloneError(std::io::Error::new(
                std::io::ErrorKind::Other,
                String::from_utf8_lossy(&output.stderr).to_string(),
            )));
        }

        Ok(FetchedSource::Dir(self.destination))
    }
}
