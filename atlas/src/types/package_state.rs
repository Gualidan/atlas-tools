use std::{
    fs::{self},
    io::ErrorKind::NotFound,
    path::PathBuf,
};

use anyhow::{Context as AnyhowContext, Result, anyhow};
use common::{
    functions::{
        sky_verify::{read_sky, sky_verify},
        unpack_payload::unpack_payload,
    },
    types::{
        manifest::{FileType, ManifestLine},
        metadata::Metadata,
    },
};

use rusqlite::{OptionalExtension, params};
use rustix::fs::{CWD, RenameFlags, renameat_with};
use tempfile::TempDir;

use crate::{
    types::contexts::{Context, RemoveContext},
    verify::{
        parse_metadata_lines::parse_metadata_lines, validate_archive::validate_archive_paths,
        validate_payload::validate_payload_paths,
    },
};

pub enum PackageState {
    NotInstalled,
    Verified {
        metadata: Metadata,
        manifest_paths: Vec<ManifestLine>,
    },
    Staged {
        tempdir: TempDir,
        metadata: Metadata,
    },
    Installed,
}

impl PackageState {
    pub fn verify(self, ctx: &mut Context) -> Result<PackageState> {
        match self {
            PackageState::NotInstalled => {
                let mut stmt = ctx.repo_conn.prepare("SELECT 1 FROM packages WHERE name = ? AND version = ? AND release = ? AND architecture = ? LIMIT 1")?;
                let rows: bool = stmt.query_row(
                    params![
                        ctx.package_id.name,
                        ctx.package_id.version,
                        ctx.package_id.release,
                        ctx.package_id.architecture.to_string()
                    ],
                    |row| row.get(0),
                )?;

                if !rows {
                    return Err(anyhow!("Verification failed"));
                }

                let metadata = sky_verify(&ctx.sky_path, &ctx.package_id, &ctx.settings)?;
                validate_archive_paths(&ctx.sky_path)
                    .context("Failed to validate archive's paths")?;
                validate_payload_paths(&ctx.sky_path)
                    .context("Failed to validate payload's paths")?;

                let (_, _, files, _) =
                    read_sky(&PathBuf::from(&ctx.sky_path)).context("Failed to read sky file")?;
                let manifest_paths =
                    parse_metadata_lines(files).context("Failed to parse manifest's lines")?;

                Ok(PackageState::Verified {
                    metadata,
                    manifest_paths,
                })
            }
            _ => {
                return Err(anyhow!(
                    "Invalid transaction not allowed to verify from this state"
                ));
            }
        }
    }

    pub fn stage(self, ctx: &mut Context) -> Result<PackageState> {
        match self {
            PackageState::Verified {
                metadata,
                manifest_paths,
            } => {
                let tempdir = ctx.temp_dir.take().context("No temporary directory")?;
                let mut stmt = ctx
                                    .install_conn
                                    .prepare(
                                        "SELECT package_name FROM owned_files WHERE path = ? AND package_name != ? AND architecture = ?",
                                    )
                                    .context("Failed to prepare database query")?;

                // File conflict check
                for line in manifest_paths {
                    if matches!(line.file_type, FileType::Dir) {
                        continue;
                    }

                    let Some(owner) = stmt
                        .query_row(
                            params![
                                line.path.to_string_lossy(),
                                ctx.package_id.name,
                                ctx.package_id.architecture.to_string(),
                            ],
                            |row| row.get::<_, String>(0),
                        )
                        .optional()?
                    else {
                        continue;
                    };

                    return Err(anyhow!(
                        "Another package {owner} already owns this path {}",
                        line.path.display()
                    ));
                }

                unpack_payload(&ctx.sky_path, &tempdir)?;

                Ok(PackageState::Staged { tempdir, metadata })
            }
            _ => {
                return Err(anyhow!(
                    "Invalid transaction not allowed to stage from this state"
                ));
            }
        }
    }

    pub fn commit(self, ctx: &mut Context) -> Result<(PackageState, Metadata)> {
        match self {
            PackageState::Staged { tempdir, metadata } => {
                fs::create_dir_all(&ctx.install_dir)
                    .context("Failed to create the install directory")?;

                match renameat_with(
                    CWD,
                    tempdir.path(),
                    CWD,
                    ctx.install_dir.clone(),
                    RenameFlags::EXCHANGE,
                ) {
                    Ok(_) => {
                        ctx.temp_dir = Some(tempdir);
                        Ok((PackageState::Installed, metadata))
                    }
                    Err(e) => Err(e.into()),
                }
            }
            _ => {
                return Err(anyhow!(
                    "Invalid transaction not allowed to commit from this state"
                ));
            }
        }
    }

    pub fn remove(self, ctx: &mut RemoveContext) -> Result<PackageState> {
        match self {
            PackageState::Installed => {
                let mut stmt  = ctx.install_conn.prepare("SELECT package_name FROM package_dependencies WHERE dependency_name = ? AND architecture = ?").context("Failed to prepare database query")?;
                let Some(dependent): Option<String> = stmt
                    .query_row(
                        params![ctx.package_id.name, ctx.package_id.architecture.to_string()],
                        |row| row.get(0),
                    )
                    .optional()?
                else {
                    match fs::remove_dir_all(ctx.install_dir.clone()) {
                        Ok(_) => {
                            return Ok(PackageState::NotInstalled);
                        }
                        Err(e) => {
                            // Treat the fact that the dir is not there as something that means its already be done
                            //  and tolerate it to avoid a scenario where updating a package bricks uninstall
                            if e.kind() == NotFound {
                                return Ok(PackageState::NotInstalled);
                            }
                            return Err(anyhow!(e));
                        }
                    }
                };

                Err(anyhow!(
                    "Cannot remove package {}: {dependent} depends on it",
                    ctx.package_id.name
                ))
            }

            _ => {
                return Err(anyhow!(
                    "Invalid transaction not allowed to remove from this state"
                ));
            }
        }
    }
}
