use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use url::Url;

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
    /// Downloads an HTTP(S) source into `destination`, returning its path and
    /// the completion time as an RFC 3339 UTC timestamp. Uses the final response
    /// URL's filename, or `tmp.bin` if absent, replacing any existing file there.
    ///
    /// Rejects other schemes and input URL paths ending in `.git`. URL, request,
    /// HTTP error status, response body, and filesystem errors propagate; a
    /// failure after file creation can leave an empty or partially written file.
    fn fetch(self: Box<Self>) -> Result<(PathBuf, String)> {
        let url = Url::parse(&self.url).context("Failed to parse url")?;

        match url.scheme() {
            "http" | "https" => {}
            scheme => {
                return Err(anyhow!(
                    "Invalid url scheme {scheme} only http and https are supported"
                ));
            }
        }

        if url.path().ends_with(".git") {
            return Err(anyhow!(
                "Git sources are not supported in v1 — build from a release tarball; devel packages are planned for v2"
            ));
        }

        let response = reqwest::blocking::get(url).context("Failed to perform HTTP request")?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fetcher(url: &str) -> HttpFetcher {
        HttpFetcher {
            url: url.to_string(),
            destination: PathBuf::from("/tmp"),
        }
    }

    #[test]
    fn rejects_git_sources() {
        let err = Box::new(fetcher("https://example.com/repo.git"))
            .fetch()
            .unwrap_err();
        assert!(
            err.to_string().contains("Git sources are not supported"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_file_scheme() {
        let err = Box::new(fetcher("file:///tmp/pkg.tar.gz"))
            .fetch()
            .unwrap_err();
        assert!(
            err.to_string().contains("Invalid url scheme"),
            "unexpected error: {err}"
        );
    }
}
