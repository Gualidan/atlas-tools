use std::{
    collections::HashSet,
    env::consts::ARCH,
    fs::{self, File, read_link},
    os::unix::fs::MetadataExt,
    path::PathBuf,
};

use anyhow::{Context, Result, anyhow};
use common::{
    functions::{
        hash::hash,
        parse_metadata_lines::parse_metadata_lines,
        sky_verify::{data_to_sign, read_sky, verify_metadata, verify_signature},
        unpack_payload::unpack_payload,
        validate_archive::validate_archive_paths,
        validate_payload::validate_payload_paths,
    },
    types::{
        config::SisyphusConfig,
        manifest::FileType,
        package::{Architectures, PackageIdentity},
    },
};
use rusqlite::{Connection, OptionalExtension, params};
use tempfile::Builder;
use walkdir::WalkDir;

use crate::{cli::cli::VerifyArgs, types::runtime_config::RuntimeConfig};

pub fn verify(
    args: VerifyArgs,
    runtime_config: &RuntimeConfig,
    settings: SisyphusConfig,
) -> Result<()> {
    if args.package.is_none() && args.sky_path.is_none() {
        return Err(anyhow!(
            "Provide a package name or a sky file's path to verify"
        ));
    }
    if args.package.is_some() && args.sky_path.is_some() {
        return Err(anyhow!(
            "Either pass a package name or a sky file's path to verify, not both."
        ));
    }

    let mut package = None;
    let mut sky_path = String::new();

    if let Some(ref name) = args.package {
        let conn = Connection::open(&runtime_config.repo_db)
            .context("Failed to open connection to the repository database")?;
        let mut stmt = conn.prepare("SELECT name, version, release, architecture, sky_path FROM packages WHERE name = ? AND architecture = ?").context("Failed to prepare database query")?;
        let Some((version, release, architecture, s)) = stmt
            .query_row(params![name, ARCH], |row| {
                let version: String = row.get(1)?;
                let release: u32 = row.get(2)?;
                let architecture: String = row.get(3)?;
                let sky_path: String = row.get(4)?;
                Ok((version, release, architecture, sky_path))
            })
            .optional()?
        else {
            return Err(anyhow!("Package {name} not found"));
        };

        package = Some(PackageIdentity {
            name: name.clone(),
            version,
            release,
            architecture: Architectures::try_from(architecture)?,
        });

        sky_path = s;
    }

    if let Some(ref sp) = args.sky_path {
        sky_path = sp.to_string_lossy().to_string();
    }

    validate_archive_paths(&sky_path).context("Failed to validate archive's paths")?;

    let (metadata_bytes, payload_bytes, files, signature_bytes) =
        read_sky(&PathBuf::from(&sky_path)).context("Failed to read sky file")?;

    match (args.package.as_ref(), args.sky_path.as_deref()) {
        (Some(_), None) => {
            let data = data_to_sign(&metadata_bytes, payload_bytes, files.as_bytes())
                .context("Failed to produce the data to sign")?;

            verify_metadata(&metadata_bytes, &package.unwrap())
                .context("Failed to verify metadata")?;
            verify_signature(&data, &settings, signature_bytes)
                .context("Failed to verify signature")?;
        }

        (None, Some(_)) => {
            let data = data_to_sign(&metadata_bytes, payload_bytes, files.as_bytes())
                .context("Failed to produce the data to sign")?;
            verify_signature(&data, &settings, signature_bytes)
                .context("Failed to verify signature")?;
        }

        _ => {}
    }

    let manifest_lines = parse_metadata_lines(files).context("Failed to parse manifest's lines")?;
    let destination = Builder::new()
        .tempdir()
        .context("Failed to create temporary directory")?;

    validate_payload_paths(&sky_path).context("Failed to validate payload paths")?;

    unpack_payload(&PathBuf::from(&sky_path), &destination).context("Failed to unpack payload")?;
    let root = destination.path();

    let mut payload_paths: HashSet<PathBuf> = HashSet::new();

    for entry in WalkDir::new(root).into_iter().skip(1) {
        let entry = entry.context("Failed to get entry")?;
        let path = entry
            .path()
            .strip_prefix(root)
            .context("Failed to strip root prefix from path")?;

        payload_paths.insert(path.to_path_buf());
    }

    let mut manifest_paths: HashSet<PathBuf> = HashSet::new();

    for line in &manifest_lines {
        manifest_paths.insert(line.path.clone());
    }

    for path in &manifest_paths {
        if !payload_paths.contains(path) {
            return Err(anyhow!(
                "manifest references path {} not in payload",
                path.display()
            ));
        }
    }

    for line in manifest_lines {
        let path = line.path;

        let h = line.target_or_hash.as_str();
        match (line.file_type, h) {
            (FileType::Dir, "-") => {
                let path = root.join(path);
                let metadata = fs::metadata(path).context("Failed to get metadata")?;
                let expected_mode = line.mode;
                let computed_mode = metadata.mode() & 0o7777;

                if expected_mode != computed_mode {
                    return Err(anyhow!(
                        "Invalid record, mode mismatch expected {expected_mode:04o} got {computed_mode:04o}"
                    ));
                }
                if !metadata.is_dir() {
                    return Err(anyhow!(
                        "Invalid record, the file type should be a directory but it is not."
                    ));
                }
            }
            (FileType::Dir, _) => {
                return Err(anyhow!(
                    "Invalid record `{}` has non-sentinel hash",
                    path.display()
                ));
            }
            (FileType::File, h) if h != "-" => {
                let path = root.join(path);

                let expected_mode = line.mode;
                let computed_mode = fs::metadata(&path)
                    .context("Failed to get metadata")?
                    .mode()
                    & 0o7777;

                let computed = hex::encode(
                    hash(File::open(path).context("Failed to open file")?)
                        .context("Failed to hash file")?,
                );

                if expected_mode != computed_mode {
                    return Err(anyhow!(
                        "Invalid record, mode mismatch expected {expected_mode:04o} got {computed_mode:04o}"
                    ));
                }
                if h != computed {
                    return Err(anyhow!(
                        "Invalid record, checksum mismatch expected {h} got {computed}"
                    ));
                }
            }
            (FileType::File, "-") => {
                return Err(anyhow!(
                    "Invalid record `{}`, it is missing its hash",
                    path.display()
                ));
            }
            (FileType::Symlink, h) => {
                let path = root.join(path);
                let expected_mode = line.mode;
                let computed_mode = fs::symlink_metadata(&path)
                    .context("Failed to get metadata")?
                    .mode()
                    & 0o7777;

                if expected_mode != computed_mode {
                    return Err(anyhow!(
                        "Invalid record, mode mismatch expected {expected_mode:04o} got {computed_mode:04o}"
                    ));
                }
                if PathBuf::from(h) != read_link(path).context("Failed to read symlink")? {
                    return Err(anyhow!(
                        "Invalid record, symlink is pointing to the wrong file"
                    ));
                }
            }
            _ => {}
        }
    }

    for path in &payload_paths {
        if !manifest_paths.contains(path) {
            return Err(anyhow!(
                "payload contains path {} not in manifest",
                path.display()
            ));
        }
    }

    println!("Success! The sky file is valid");

    Ok(())
}
