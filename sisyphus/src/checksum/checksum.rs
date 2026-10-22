use anyhow::{Context, Result};
use common::functions::hash::hash;
use std::fs::File;

use crate::fetch::fetch::fetch;
use crate::types::package::Package;
use crate::types::runtime_config::RuntimeConfig;

pub fn checksum_gen(package: &Package, runtime_config: &RuntimeConfig) -> Result<()> {
    let fetched = fetch(package, false, runtime_config)?;

    // Generate checksum and print it to the user
    let file = File::open(fetched).context("Failed to open fetched archive")?;
    let hasher = hash(&file).context("Failed to hash fetched archive")?;

    println!(
        "Checksum: {}, paste this into your recipe",
        hasher
            .as_ref()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>()
    );

    Ok(())
}
