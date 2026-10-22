use anyhow::{Context, Result};
use chrono::Utc;

use crate::types::fetcher::Fetcher;
use std::{
    fs::{self, File},
    path::PathBuf,
};

pub struct HttpFetcher {
    pub url: String,
    pub destination: PathBuf,
}

impl Fetcher for HttpFetcher {
    fn fetch(self: Box<Self>) -> Result<(PathBuf, String)> {
        let response =
            reqwest::blocking::get(&self.url).context("Failed to perform HTTP request")?;
        let response = response
            .error_for_status()
            .context("HTTP request returned an error status")?;

        let fname = response
            .url()
            .path_segments()
            .and_then(|segments| segments.last())
            .and_then(|name| if name.is_empty() { None } else { Some(name) })
            .unwrap_or("tmp.bin");
        let fname = self.destination.join(fname);

        fs::create_dir_all(self.destination).context("Failed to create destination directory")?;
        let mut dest = File::create(&fname).context("Failed to create destination file")?;
        let content = response
            .bytes()
            .context("Failed to read HTTP response body")?;

        std::io::copy(&mut content.as_ref(), &mut dest)
            .context("Failed to write downloaded content to disk")?;
        let fetched_at = Utc::now().to_rfc3339();
        // ensure it's flushed/closed before reopening
        drop(dest);

        Ok((fname, fetched_at))
    }
}
