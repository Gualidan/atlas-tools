use anyhow::{Context, Result};
use std::path::PathBuf;

use tar::{Builder, Header};
use zstd::Encoder;

use crate::{
    package::db::insert_package,
    types::{guard::SkyGuard, package::Package, runtime_config::RuntimeConfig},
};
use common::types::metadata::Metadata;

/// Writes a signed package archive and records it in the repository database.
///
/// # Errors
///
/// Returns an error if the output directory, archive, zstd stream, or repository
/// database cannot be written.
pub fn sky_gen(
    metadata: &Metadata,
    payload: &Vec<u8>,
    manifest: &String,
    signature: &Vec<u8>,
    package: &Package,
    output_path: &PathBuf,
    runtime_config: &RuntimeConfig,
) -> Result<PathBuf> {
    if !output_path.exists() {
        std::fs::create_dir_all(output_path).context("Failed to create sky output directory")?;
    }
    let output_path = output_path.join(PathBuf::from(format!(
        "{}-{}-{}-{}.sky",
        package.name, package.version, package.release, package.architecture
    )));

    let (sky_file, temp_path) = tempfile::Builder::new()
        .prefix(".sky-")
        .suffix(".tmp")
        .tempfile_in(
            output_path
                .parent()
                .context("Sky output path has no parent directory")?,
        )
        .context("Failed to create temporary sky file")?
        .keep()
        .context("Failed to retain temporary sky file")?;
    let mut guard = SkyGuard::new(&temp_path);

    let metadata_string =
        serde_saphyr::to_string(metadata).context("Failed to serialize metadata")?;

    let mut archive = Builder::new(
        Encoder::new(sky_file, 3).context("Failed to create zstd encoder for sky file")?,
    );

    let mut metadata_header = Header::new_gnu();
    metadata_header.set_mode(0o644);
    metadata_header.set_size(metadata_string.len() as u64);
    archive
        .append_data(
            &mut metadata_header,
            "metadata.yaml",
            metadata_string.as_bytes(),
        )
        .context("Failed to append metadata to sky archive")?;

    let mut payload_header = Header::new_gnu();
    payload_header.set_mode(0o644);
    payload_header.set_size(payload.len() as u64);
    archive
        .append_data(&mut payload_header, "payload.tar.zst", payload.as_slice())
        .context("Failed to append payload to sky archive")?;

    let mut manifest_header = Header::new_gnu();
    manifest_header.set_mode(0o644);
    manifest_header.set_size(manifest.len() as u64);
    archive
        .append_data(&mut manifest_header, "files", manifest.as_bytes())
        .context("Failed to append manifest to sky archive")?;

    let mut signature_header = Header::new_gnu();
    signature_header.set_mode(0o644);
    signature_header.set_size(signature.len() as u64);
    archive
        .append_data(
            &mut signature_header,
            "signature.ed25519",
            signature.as_slice(),
        )
        .context("Failed to append signature to sky archive")?;

    let archive = archive
        .into_inner()
        .context("Failed to finish sky archive")?
        .finish()
        .context("Failed to finish zstd stream")?;

    drop(archive);

    insert_package(
        runtime_config,
        package,
        &output_path.as_os_str().to_string_lossy(),
        metadata,
    )?;

    std::fs::rename(&temp_path, &output_path).context("Failed to publish sky file")?;

    guard.disarm();
    drop(guard);

    Ok(output_path)
}
