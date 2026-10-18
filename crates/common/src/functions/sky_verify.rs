use std::{
    fs::{self, File},
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

use anyhow::{Context, Ok, Result, anyhow};
use ring::signature::{ED25519, UnparsedPublicKey};
use tar::Archive;
use zstd::Decoder;

use crate::{
    functions::hash::hash,
    types::{config::SisyphusConfig, metadata::Metadata, package::Package},
};

pub fn read_sky<P: AsRef<Path>>(path: P) -> Result<(Vec<u8>, Vec<u8>, String, Vec<u8>)> {
    let tar_gz = File::open(&path).context("Failed to open sky file")?;
    let tar = Decoder::new(tar_gz).context("Failed to decompress the sky file")?;
    let mut archive = Archive::new(tar);

    let mut metadata = Vec::new();
    let mut payload = Vec::new();
    let mut files = String::new();
    let mut signature = Vec::new();

    let entries = archive
        .entries()
        .context("Failed to read sky archive entries")?;
    for entry in entries {
        let mut entry = entry.context("Failed to read sky archive entry")?;

        let path = entry.path().context("Failed to derive entry path")?;

        match path.file_name().and_then(|n| n.to_str()) {
            Some("metadata.yaml") => {
                let mut buffer = Vec::new();
                entry
                    .read_to_end(&mut buffer)
                    .context("Failed to read metadata from sky file")?;
                metadata = buffer;
            }
            Some("payload.tar.zst") => {
                let mut buffer = Vec::new();
                entry
                    .read_to_end(&mut buffer)
                    .context("Failed to read payload from sky file")?;
                payload = buffer;
            }
            Some("files") => {
                let mut buffer = String::new();
                entry
                    .read_to_string(&mut buffer)
                    .context("Failed to read manifest from sky file")?;
                files = buffer;
            }
            Some("signature.ed25519") => {
                let mut buffer = Vec::new();
                entry
                    .read_to_end(&mut buffer)
                    .context("Failed to read signature from sky file")?;
                signature = buffer;
            }
            _ => {}
        }
    }

    Ok((metadata, payload, files, signature))
}

pub fn verify_signature(
    data: &[u8],
    settings: &SisyphusConfig,
    signature_bytes: Vec<u8>,
) -> Result<()> {
    let pub_key_file: Vec<u8> =
        fs::read(&settings.pub_key_path).context("Failed to read public key")?;

    if pub_key_file.len() != 32 {
        return Err(anyhow!("Invalid public key: expected 32 bytes"));
    }
    let pub_key = UnparsedPublicKey::new(&ED25519, &pub_key_file);
    pub_key
        .verify(data, &signature_bytes)
        .context("Failed to verify sky file signature")?;

    Ok(())
}

pub fn verify_metadata(metadata_bytes: &Vec<u8>, package: &Package) -> Result<Metadata> {
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

pub fn data_to_sign(
    metadata_bytes: &Vec<u8>,
    payload_bytes: Vec<u8>,
    files_bytes: &[u8],
) -> Result<Vec<u8>> {
    let metadata_hash = hash(Cursor::new(&metadata_bytes)).context("Failed to hash metadata")?;
    let payload_hash = hash(Cursor::new(&payload_bytes)).context("Failed to hash payload")?;
    let files_hash = hash(Cursor::new(&files_bytes)).context("Failed to hash manifest")?;

    let data = [
        metadata_hash.as_ref(),
        payload_hash.as_ref(),
        files_hash.as_ref(),
    ]
    .concat();

    Ok(data)
}
pub fn sky_verify(
    file_name: &PathBuf,
    package: &Package,
    settings: &SisyphusConfig,
) -> Result<Metadata> {
    let (metadata_bytes, payload_bytes, files_bytes, signature_bytes) =
        read_sky(file_name).context("Failed to read sky file")?;

    let data = data_to_sign(&metadata_bytes, payload_bytes, files_bytes.as_bytes())
        .context("Failed to produce the data to sign")?;

    verify_signature(&data, settings, signature_bytes).context("Failed to verify signature")?;

    let metadata =
        verify_metadata(&metadata_bytes, &package).context("Failed to verify metadata")?;

    Ok(metadata)
}
