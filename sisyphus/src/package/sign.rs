use std::path::PathBuf;

use common::{functions::hash::hash, types::metadata::Metadata};

use crate::{signing::signing::sign, types::error::SigningError};

pub fn sign_artifacts(
    priv_key_path: &PathBuf,
    metadata: &Metadata,
    payload: &Vec<u8>,
    manifest: &String,
) -> Result<Vec<u8>, SigningError> {
    let metadata_string = serde_saphyr::to_string(metadata)?;

    let metadata_hash = hash(metadata_string.as_bytes())?;
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
