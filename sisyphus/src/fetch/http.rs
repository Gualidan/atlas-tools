use crate::types::{
    error::FetchError,
    fetcher::{FetchedSource, Fetcher},
};
use std::fs::File;
use tempfile::TempDir;

pub struct HttpFetcher<'a> {
    pub url: String,
    pub destination: &'a TempDir,
}

impl<'a> Fetcher<'a> for HttpFetcher<'a> {
    fn fetch(&self) -> Result<FetchedSource<'a>, FetchError> {
        let response = reqwest::blocking::get(&self.url)?;
        let response = response.error_for_status()?;

        let fname = response
            .url()
            .path_segments()
            .and_then(|segments| segments.last())
            .and_then(|name| if name.is_empty() { None } else { Some(name) })
            .unwrap_or("tmp.bin");
        let fname = self.destination.path().join(fname);

        let mut dest = File::create(&fname)?;
        let content = response.bytes()?;

        std::io::copy(&mut content.as_ref(), &mut dest)?;
        // ensure it's flushed/closed before reopening
        drop(dest);

        Ok(FetchedSource::Archive(fname))
    }
}
