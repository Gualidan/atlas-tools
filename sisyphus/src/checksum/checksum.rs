use crate::types::package::Package;
use anyhow::{Context, Result};
use common::functions::hash::hash;
use ring::digest::Digest;
use std::fs::File;

use crate::fetch::fetch::fetch;
use crate::types::fetcher::FetchedSource;
use crate::types::runtime_config::RuntimeConfig;

pub enum ChecksumResult {
    Hashed(Digest),
}

pub fn checksum_gen(package: &Package, runtime_config: &RuntimeConfig) -> Result<ChecksumResult> {
    let fetched = fetch(package, false, runtime_config)?;

    // Generate checksum and print it to the user
    match fetched {
        FetchedSource::Archive(path) => {
            let file = File::open(path).context("Failed to open fetched archive")?;
            let hasher = hash(&file).context("Failed to hash fetched archive")?;

            println!(
                "Checksum: {}, paste this into your recipe",
                hasher
                    .as_ref()
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect::<String>()
            );
            Ok(ChecksumResult::Hashed(hasher))
        }
    }
}
