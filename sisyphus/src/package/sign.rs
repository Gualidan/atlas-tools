use std::path::PathBuf;

use crate::{checksum::hash::hash, signing::signing::sign, types::error::SigningError};

pub fn sign_artifacts(
    priv_key_path: PathBuf,
    metadata: String,
    payload: Vec<u8>,
    manifest: String,
) -> Result<Vec<u8>, SigningError> {
    let metadata_hash = hash(metadata.as_bytes())?;
    let payload_hash = hash(payload.as_slice())?;
    let manifest_hash = hash(manifest.as_bytes())?;

    let data = [
        metadata_hash.as_ref(),
        payload_hash.as_ref(),
        manifest_hash.as_ref(),
    ]
    .concat();
    let signature = sign(&priv_key_path, data.as_slice())?;

    Ok(signature)
}
