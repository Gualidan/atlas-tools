use crate::types::package::Package;
use anyhow::{Context, Result, anyhow};
use common::functions::hash::hash;
use std::fs::File;
use std::path::PathBuf;

/// Verifies that a file matches the checksum declared by a package recipe.
///
/// # Errors
///
/// Returns an error if the file cannot be opened or hashed, or its checksum does
/// not match the recipe.
pub fn verify(file: &PathBuf, package: &Package) -> Result<()> {
    let computed = hash(File::open(file).context("Failed to open file for checksum verification")?)
        .context("Failed to hash file for checksum verification")?;

    let expected = &package.source.sha256;

    let computed = hex::encode(computed.as_ref());

    if &computed != expected {
        return Err(anyhow!(
            "Checksum mismatch: expected {expected}, got {computed}"
        ));
    }
    Ok(())
}
