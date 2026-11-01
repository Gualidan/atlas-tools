use anyhow::{Context, Result};
use std::path::PathBuf;

use common::{functions::hash::hash, types::metadata::Metadata};

use crate::signing::signing::sign;

/// Signs the hashes of package metadata, payload, and file manifest.
///
/// # Errors
///
/// Returns an error if metadata cannot be serialized or hashed, or if the
/// private key cannot be read or used to create a signature.
pub fn sign_artifacts(
    priv_key_path: &PathBuf,
    metadata: &Metadata,
    payload: &Vec<u8>,
    manifest: &String,
) -> Result<Vec<u8>> {
    let metadata_string =
        serde_saphyr::to_string(metadata).context("Failed to serialize metadata")?;

    let metadata_hash = hash(metadata_string.as_bytes()).context("Failed to hash metadata")?;
    let payload_hash = hash(payload.as_slice()).context("Failed to hash payload")?;
    let manifest_hash = hash(manifest.as_bytes()).context("Failed to hash manifest")?;

    let data = [
        metadata_hash.as_ref(),
        payload_hash.as_ref(),
        manifest_hash.as_ref(),
    ]
    .concat();
    let signature = sign(priv_key_path, data.as_slice()).context("Failed to sign artifacts")?;

    Ok(signature)
}
