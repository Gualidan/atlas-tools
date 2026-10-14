use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use std::{path::PathBuf, process::Command};
use which::which;

use crate::types::fetcher::{FetchedSource, Fetcher};

pub struct GitFetcher {
    pub url: String,
    pub destination: PathBuf,
}

impl Fetcher for GitFetcher {
    fn fetch(self: Box<Self>) -> Result<(FetchedSource, String)> {
        which("git").context("Git not found")?;

        let output = Command::new("git")
            .arg("clone")
            .arg(&self.url)
            .arg(&self.destination)
            .output()
            .context("Failed to run git clone")?;

        let fetched_at = Utc::now().to_rfc3339();
        if !output.status.success() {
            return Err(anyhow!(
                "Failed to clone git repository: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        Ok((FetchedSource::Dir(self.destination), fetched_at))
    }
}
