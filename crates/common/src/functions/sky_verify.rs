use std::{
    fs::{self, File},
    io::{Cursor, Read},
    path::PathBuf,
};

use anyhow::{Context, Result, anyhow};
use ring::signature::{ED25519, UnparsedPublicKey};
use tar::Archive;
use zstd::Decoder;

use crate::{
    functions::hash::hash,
    types::{config::SisyphusConfig, metadata::Metadata, package::Package},
};

pub fn sky_verify(
    file_name: &PathBuf,
    package: &Package,
    settings: &SisyphusConfig,
) -> Result<Metadata> {
    let tar_gz = File::open(&file_name).context("Failed to open sky file")?;
    let tar = Decoder::new(tar_gz).context("Failed to decompress the sky file")?;
    let mut archive = Archive::new(tar);

    let mut metadata_bytes: Vec<u8> = Vec::new();
    let mut payload_bytes: Vec<u8> = Vec::new();
    let mut files_bytes: Vec<u8> = Vec::new();
    let mut signature_bytes: Vec<u8> = Vec::new();

    let entries = archive.entries().context("Failed to read sky archive entries")?;
    for entry in entries {
        let mut entry = entry.context("Failed to read sky archive entry")?;

        let path = entry.path().context("Failed to derive entry path")?;

        match path.file_name().and_then(|n| n.to_str()) {
            Some("metadata.yaml") => {
                let mut buffer = Vec::new();
                entry
                    .read_to_end(&mut buffer)
                    .context("Failed to read metadata from sky file")?;
                metadata_bytes = buffer;
            }
            Some("payload.tar.zst") => {
                let mut buffer = Vec::new();
                entry
                    .read_to_end(&mut buffer)
                    .context("Failed to read payload from sky file")?;
                payload_bytes = buffer;
            }
            Some("files") => {
                let mut buffer = Vec::new();
                entry
                    .read_to_end(&mut buffer)
                    .context("Failed to read manifest from sky file")?;
                files_bytes = buffer;
            }
            Some("signature.ed25519") => {
                let mut buffer = Vec::new();
                entry
                    .read_to_end(&mut buffer)
                    .context("Failed to read signature from sky file")?;
                signature_bytes = buffer;
            }
            _ => {}
        }
    }

    let metadata_hash = hash(Cursor::new(&metadata_bytes)).context("Failed to hash metadata")?;
    let payload_hash = hash(Cursor::new(&payload_bytes)).context("Failed to hash payload")?;
    let files_hash = hash(Cursor::new(&files_bytes)).context("Failed to hash manifest")?;

    let data = [
        metadata_hash.as_ref(),
        payload_hash.as_ref(),
        files_hash.as_ref(),
    ]
    .concat();

    let pub_key_file: Vec<u8> =
        fs::read(&settings.pub_key_path).context("Failed to read public key")?;

    if pub_key_file.len() != 32 {
        return Err(anyhow!("Invalid public key: expected 32 bytes"));
    }
    let pub_key = UnparsedPublicKey::new(&ED25519, &pub_key_file);
    pub_key
        .verify(data.as_slice(), &signature_bytes)
        .context("Failed to verify sky file signature")?;

    let metadata: Metadata =
        serde_saphyr::from_slice(&metadata_bytes).context("Failed to deserialize metadata")?;

    if metadata.name != package.name
        || metadata.version != package.version
        || metadata.release != package.release
        || metadata.architecture != package.architecture
    {
        return Err(anyhow!(
            "Verification failed: metadata does not match the expected package"
        ));
    }

    Ok(metadata)
}
