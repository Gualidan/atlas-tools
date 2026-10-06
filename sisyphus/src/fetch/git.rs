use crate::types::fetcher::{FetchedSource, Fetcher};
use chrono::Utc;
use common::types::error::FetchError;
use std::{path::PathBuf, process::Command};
use which::which;

pub struct GitFetcher {
    pub url: String,
    pub destination: PathBuf,
}

impl Fetcher for GitFetcher {
    fn fetch(self: Box<Self>) -> Result<(FetchedSource, String), FetchError> {
        which("git")?;

        let output = Command::new("git")
            .arg("clone")
            .arg(&self.url)
            .arg(&self.destination)
            .output()?;

        let fetched_at = Utc::now().to_rfc3339();
        if !output.status.success() {
            return Err(FetchError::CloneError(std::io::Error::new(
                std::io::ErrorKind::Other,
                String::from_utf8_lossy(&output.stderr).to_string(),
            )));
        }

        Ok((FetchedSource::Dir(self.destination), fetched_at))
    }
}
