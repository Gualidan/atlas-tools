use std::{
    fs::{self, File},
    io::{Cursor, Read},
    path::PathBuf,
};

use ring::signature::{ED25519, UnparsedPublicKey};
use tar::Archive;
use zstd::Decoder;

use crate::{
    functions::hash::hash,
    types::{error::VerifyError, metadata::Metadata, package::Package, settings::Settings},
};

pub fn sky_verify(
    file_name: &PathBuf,
    package: &Package,
    settings: &Settings,
) -> Result<Option<Metadata>, VerifyError> {
    if !file_name.exists() {
        return Ok(None);
    }

    let tar_gz = File::open(&file_name)?;
    let tar = Decoder::new(tar_gz)?;
    let mut archive = Archive::new(tar);

    let mut metadata_bytes: Vec<u8> = Vec::new();
    let mut payload_bytes: Vec<u8> = Vec::new();
    let mut files_bytes: Vec<u8> = Vec::new();
    let mut signature_bytes: Vec<u8> = Vec::new();

    let entries = archive.entries()?;
    for entry in entries {
        let mut entry = entry?;

        let path = entry.path()?;

        match path.file_name().and_then(|n| n.to_str()) {
            Some("metadata.yaml") => {
                let mut buffer = Vec::new();
                entry.read_to_end(&mut buffer)?;
                metadata_bytes = buffer;
            }
            Some("payload.tar.zst") => {
                let mut buffer = Vec::new();
                entry.read_to_end(&mut buffer)?;
                payload_bytes = buffer;
            }
            Some("files") => {
                let mut buffer = Vec::new();
                entry.read_to_end(&mut buffer)?;
                files_bytes = buffer;
            }
            Some("signature.ed25519") => {
                let mut buffer = Vec::new();
                entry.read_to_end(&mut buffer)?;
                signature_bytes = buffer;
            }
            _ => {}
        }
    }

    let metadata_hash = hash(Cursor::new(&metadata_bytes))?;
    let payload_hash = hash(Cursor::new(&payload_bytes))?;
    let files_hash = hash(Cursor::new(&files_bytes))?;

    let data = [
        metadata_hash.as_ref(),
        payload_hash.as_ref(),
        files_hash.as_ref(),
    ]
    .concat();

    let pub_key_file: Vec<u8> = fs::read(&settings.pub_key_path)?;

    if pub_key_file.len() != 32 {
        return Err(VerifyError::InvalidPublicKey);
    }
    let pub_key = UnparsedPublicKey::new(&ED25519, &pub_key_file);
    pub_key.verify(data.as_slice(), &signature_bytes)?;

    // Verify metadata.yaml
    let metadata: Metadata = serde_saphyr::from_slice(&metadata_bytes)?;

    if metadata.name != package.name {
        return Ok(None);
    }

    if metadata.version != package.version {
        return Ok(None);
    }

    if metadata.release != package.release {
        return Ok(None);
    }

    if metadata.architecture != package.architecture {
        return Ok(None);
    }

    Ok(Some(metadata))
}
