use std::{fs::File, path::PathBuf};

use tar::{Builder, Header};
use zstd::Encoder;

use crate::types::{error::PackageError, package::Package};

pub fn sky_gen(
    metadata: &String,
    payload: &Vec<u8>,
    manifest: &String,
    signature: &Vec<u8>,
    package: &Package,
    output_path: PathBuf,
) -> Result<PathBuf, PackageError> {
    if !output_path.exists() {
        std::fs::create_dir_all(&output_path)?;
    }
    let output_path = output_path.join(PathBuf::from(format!(
        "{}-{}-{}-{}.sky",
        package.name, package.version, package.release, package.architecture
    )));

    let sky_file = File::create(&output_path)?;

    let mut archive = Builder::new(Encoder::new(sky_file, 3)?);

    let mut metadata_header = Header::new_gnu();
    metadata_header.set_mode(0o644);
    metadata_header.set_size(metadata.len() as u64);
    archive.append_data(&mut metadata_header, "metadata.yaml", metadata.as_bytes())?;

    let mut payload_header = Header::new_gnu();
    payload_header.set_mode(0o644);
    payload_header.set_size(payload.len() as u64);
    archive.append_data(&mut payload_header, "payload.tar.zst", payload.as_slice())?;

    let mut manifest_header = Header::new_gnu();
    manifest_header.set_mode(0o644);
    manifest_header.set_size(manifest.len() as u64);
    archive.append_data(&mut manifest_header, "files", manifest.as_bytes())?;

    let mut signature_header = Header::new_gnu();
    signature_header.set_mode(0o644);
    signature_header.set_size(signature.len() as u64);
    archive.append_data(
        &mut signature_header,
        "signature.ed25519",
        signature.as_slice(),
    )?;

    let archive = archive.into_inner()?.finish()?;

    drop(archive);

    Ok(output_path)
}
