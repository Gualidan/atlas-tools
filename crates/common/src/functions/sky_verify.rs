use std::{
    fs::{self, File},
    io::{BufReader, Cursor, Read},
    path::{Path, PathBuf},
};

use anyhow::{Context, Ok, Result, anyhow};
use ring::signature::{ED25519, UnparsedPublicKey};
use tar::Archive;
use zstd::Decoder;

use crate::{
    functions::hash::hash,
    types::{config::SisyphusConfig, metadata::Metadata, package::PackageIdentity},
};

/// Opens a sky file as a Zstandard-decoded tar reader without validating entries.
/// Returns file-open and decoder initialization errors; errors encountered while
/// reading archive contents are deferred to the returned reader.
///
/// # Errors
///
/// Returns an error if the file cannot be opened or the Zstandard decoder
/// cannot be initialized.
pub fn get_sky_archive<P: AsRef<Path>>(path: &P) -> Result<Archive<Decoder<'_, BufReader<File>>>> {
    let tar_gz = File::open(path).context("Failed to open sky file")?;
    let tar = Decoder::new(tar_gz).context("Failed to decompress the sky file")?;
    let archive = Archive::new(tar);

    Ok(archive)
}

pub type SkyContents = (Vec<u8>, Vec<u8>, String, Vec<u8>);

/// Reads the metadata, payload, manifest, and signature entries from a sky archive.
///
/// # Errors
///
/// Returns an error if the archive cannot be opened, decompressed, or read.
pub fn read_sky<P: AsRef<Path>>(path: P) -> Result<SkyContents> {
    let mut metadata = Vec::new();
    let mut payload = Vec::new();
    let mut files = String::new();
    let mut signature = Vec::new();

    let mut archive = get_sky_archive(&path)?;
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

/// Verifies the Ed25519 signature against the configured public key.
///
/// # Errors
///
/// Returns an error if the key cannot be read, has an invalid size, or the
/// signature does not verify.
pub fn verify_signature(
    data: &[u8],
    settings: &SisyphusConfig,
    signature_bytes: &[u8],
) -> Result<()> {
    let pub_key_file: Vec<u8> =
        fs::read(&settings.pub_key_path).context("Failed to read public key")?;

    if pub_key_file.len() != 32 {
        return Err(anyhow!("Invalid public key: expected 32 bytes"));
    }
    let pub_key = UnparsedPublicKey::new(&ED25519, &pub_key_file);
    pub_key
        .verify(data, signature_bytes)
        .context("Failed to verify sky file signature")?;

    Ok(())
}

/// Deserializes YAML metadata and returns it if its name, version, release, and
/// architecture match `package`. Returns deserialization or identity mismatch
/// errors; this check does not authenticate the metadata's signature.
///
/// # Errors
///
/// Returns an error if metadata cannot be deserialized or does not match the
/// expected package identity.
pub fn verify_metadata(metadata_bytes: &[u8], package: &PackageIdentity) -> Result<Metadata> {
    let metadata: Metadata =
        serde_saphyr::from_slice(metadata_bytes).context("Failed to deserialize metadata")?;

    if metadata.name != package.name
        || metadata.version != package.version
        || metadata.release != package.release
        || metadata.architecture != package.architecture
    {
        let actual = PackageIdentity {
            name: metadata.name,
            version: metadata.version,
            release: metadata.release,
            architecture: metadata.architecture,
        };
        return Err(anyhow!(
            "Verification failed: metadata does not match the expected package: metadata: {actual:?}, expected: {package:?}"
        ));
    }

    Ok(metadata)
}

/// Produces the concatenated hashes used as the signed data for an archive.
///
/// # Errors
///
/// Returns an error if hashing any input fails.
pub fn data_to_sign(
    metadata_bytes: &[u8],
    payload_bytes: &[u8],
    files_bytes: &[u8],
) -> Result<Vec<u8>> {
    let metadata_hash = hash(Cursor::new(metadata_bytes)).context("Failed to hash metadata")?;
    let payload_hash = hash(Cursor::new(payload_bytes)).context("Failed to hash payload")?;
    let files_hash = hash(Cursor::new(files_bytes)).context("Failed to hash manifest")?;

    let data = [
        metadata_hash.as_ref(),
        payload_hash.as_ref(),
        files_hash.as_ref(),
    ]
    .concat();

    Ok(data)
}
/// Returns metadata after verifying the archive's Ed25519 signature using
/// `settings.pub_key_path` and matching the expected package identity.
/// Propagates archive reading, public-key, signature, and metadata errors.
/// Does not validate payload paths or manifest contents.
///
/// # Errors
///
/// Returns an error if archive reading, signature verification, or package
/// metadata validation fails.
pub fn sky_verify(
    file_name: &PathBuf,
    package: &PackageIdentity,
    settings: &SisyphusConfig,
) -> Result<Metadata> {
    let (metadata_bytes, payload_bytes, files_bytes, signature_bytes) =
        read_sky(file_name).context("Failed to read sky file")?;

    let data = data_to_sign(&metadata_bytes, &payload_bytes, files_bytes.as_bytes())
        .context("Failed to produce the data to sign")?;

    verify_signature(&data, settings, &signature_bytes).context("Failed to verify signature")?;

    let metadata =
        verify_metadata(&metadata_bytes, package).context("Failed to verify metadata")?;

    Ok(metadata)
}
