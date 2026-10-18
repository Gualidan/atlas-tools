use std::{
    env::consts::ARCH,
    fs::{self, File, read_link},
    path::PathBuf,
};

use anyhow::{Context, Result, anyhow};
use common::{
    functions::{
        hash::hash,
        sky_verify::{data_to_sign, read_sky, verify_metadata, verify_signature},
        unpack_payload::unpack_payload,
    },
    types::{
        config::SisyphusConfig,
        manifest::{FileType, ManifestLine},
        package::{Architectures, Package, Source},
    },
};
use rusqlite::{Connection, OptionalExtension, params};
use tempfile::Builder;

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

        package = Some(Package {
            schema: 1,
            name: name.clone(),
            version,
            release,
            architecture: Architectures::try_from(architecture)?,
            source: Source {
                url: String::new(),
                sha256: None,
            },
            makedeps: vec![],
            deps: vec![],
            prepare: None,
            build: None,
            check: None,
            package: String::new(),
        });

        sky_path = s;
    }

    if let Some(ref sp) = args.sky_path {
        sky_path = sp.to_string_lossy().to_string();
    }

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

    let lines: Vec<&str> = files.lines().collect();
    let mut manifest_lines: Vec<ManifestLine> = Vec::new();

    for line in lines {
        let fields: Vec<&str> = line.splitn(4, '\t').collect();

        match fields[..] {
            [kind, mode, target_or_hash, path] => {
                let mode = u32::from_str_radix(mode, 8)
                    .context("Failed to parse mode, is it malformed?")?;
                let kind = match kind {
                    "d" => FileType::Dir,
                    "f" => FileType::File,
                    "l" => FileType::Symlink,
                    unknown @ _ => {
                        return Err(anyhow!("Invalid file type: {}", unknown));
                    }
                };
                manifest_lines.push(ManifestLine {
                    file_type: kind,
                    mode: mode,
                    target_or_hash: target_or_hash.to_string(),
                    path: PathBuf::from(path),
                });
            }
            _ => {
                return Err(anyhow!("Invalid line"));
            }
        }
    }

    let destination = Builder::new()
        .tempdir()
        .context("Failed to create temporary directory")?;

    unpack_payload(&PathBuf::from(&sky_path), &destination).context("Failed to unpack payload")?;
    let root = destination.path();

    for line in manifest_lines {
        let h = line.target_or_hash.as_str();
        match (line.file_type, h) {
            (FileType::Dir, "-") => {
                let path = root.join(line.path);
                if !fs::metadata(path)?.is_dir() {
                    return Err(anyhow!(
                        "Invalid record, the file type should be a directory but it is not."
                    ));
                }
            }
            (FileType::Dir, _) => {
                return Err(anyhow!(
                    "Invalid record `{}` has non-sentinel hash",
                    line.path.display()
                ));
            }
            (FileType::File, h) if h != "-" => {
                let path = root.join(line.path);

                let computed = hex::encode(
                    hash(File::open(path).context("Failed to open file")?)
                        .context("Failed to hash file")?,
                );
                if h != computed {
                    return Err(anyhow!(
                        "Invalid record, checksum mismatch expected {h} got {computed}"
                    ));
                }
            }
            (FileType::File, "-") => {
                return Err(anyhow!(
                    "Invalid record `{}`, it is missing its hash",
                    line.path.display()
                ));
            }
            (FileType::Symlink, h) => {
                let path = root.join(line.path);
                if PathBuf::from(h) != read_link(path).context("Failed to read symlink")? {
                    return Err(anyhow!(
                        "Invalid record, symlink is pointing to the wrong file"
                    ));
                }
            }
            _ => {}
        }
    }

    println!("Success! The sky file is valid");

    Ok(())
}
