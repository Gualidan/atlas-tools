use anyhow::{Context, Result};
use common::functions::hash::hash;
use std::fs::File;

use crate::fetch::fetch::{FetchMode, fetch};
use crate::types::package::Package;
use crate::types::runtime_config::RuntimeConfig;

/// Fetches a package source and prints its SHA-256 checksum.
///
/// # Errors
///
/// Returns an error if fetching, opening, or hashing the source fails.
pub fn checksum_gen(package: &Package, runtime_config: &RuntimeConfig) -> Result<()> {
    let fetched = fetch(package, &FetchMode::Checksum, runtime_config)?;

    // Generate checksum and print it to the user
    let file = File::open(fetched).context("Failed to open fetched archive")?;
    let hasher = hash(&file).context("Failed to hash fetched archive")?;

    println!(
        "Checksum: {}, paste this into your recipe",
        hex::encode(hasher.as_ref())
    );

    Ok(())
}
