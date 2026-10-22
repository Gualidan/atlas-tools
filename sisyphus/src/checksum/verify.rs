use crate::types::package::Package;
use anyhow::{Context, Result, anyhow};
use common::functions::hash::hash;
use std::fs::File;
use std::path::PathBuf;

pub fn verify(file: &PathBuf, package: &Package) -> Result<()> {
    let computed = hash(File::open(file).context("Failed to open file for checksum verification")?)
        .context("Failed to hash file for checksum verification")?;

    let expected = &package.source.sha256;

    let computed = computed
        .as_ref()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();

    if &computed != expected {
        return Err(anyhow!(
            "Checksum mismatch: expected {expected}, got {computed}"
        ));
    }
    Ok(())
}
