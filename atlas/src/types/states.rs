use std::{
    fs::{self, remove_dir_all, remove_file},
    path::PathBuf,
};

use anyhow::{Context as AnyhowContext, Result, anyhow};
use common::functions::{
    parse_metadata_lines::parse_metadata_lines,
    sky_verify::{read_sky, sky_verify},
    unpack_payload::unpack_payload,
};

use rusqlite::{OptionalExtension, params};
use rustix::fs::{CWD, RenameFlags, renameat_with};

use walkdir::WalkDir;

use crate::types::context::Context;

pub struct NotInstalled;

pub struct Verified;

pub struct Staged;

pub struct Installed;

pub trait State {
    fn verify(self: Box<Self>, ctx: &mut Context) -> Result<Box<dyn State>>;
    fn stage(self: Box<Self>, ctx: &mut Context) -> Result<Box<dyn State>>;
    fn commit(self: Box<Self>, ctx: &mut Context) -> Result<Box<dyn State>>;
    fn remove(self: Box<Self>, ctx: &mut Context) -> Result<Box<dyn State>>;
    fn update(self: Box<Self>, ctx: &mut Context) -> Result<Box<dyn State>>;
}

impl State for NotInstalled {
    fn verify(self: Box<Self>, ctx: &mut Context) -> Result<Box<dyn State>> {
        let mut stmt = ctx.repo_conn.prepare("SELECT 1 FROM packages WHERE name = ? AND version = ? AND release = ? AND architecture = ? LIMIT 1")?;
        let rows: bool = stmt.query_row(
            params![
                ctx.package.name,
                ctx.package.version,
                ctx.package.release,
                ctx.package.architecture.to_string()
            ],
            |row| row.get(0),
        )?;

        if !rows {
            return Err(anyhow!("Verification failed"));
        }

        let metadata = sky_verify(&ctx.sky_path, &ctx.package, &ctx.settings)?;
        ctx.metadata = Some(metadata);
        let (_, _, files, _) =
            read_sky(&PathBuf::from(&ctx.sky_path)).context("Failed to read sky file")?;
        ctx.manifest_paths =
            Some(parse_metadata_lines(files).context("Failed to parse manifest's lines")?);

        Ok(Box::new(Verified))
    }

    fn stage(self: Box<Self>, _ctx: &mut Context) -> Result<Box<dyn State>> {
        Err(anyhow!("Package is not verified not allowed to stage it"))
    }

    fn commit(self: Box<Self>, _ctx: &mut Context) -> Result<Box<dyn State>> {
        Err(anyhow!("Package is not staged not allowed to commit it"))
    }

    fn remove(self: Box<Self>, _ctx: &mut Context) -> Result<Box<dyn State>> {
        Err(anyhow!("Package is not installed not allowed to remove it"))
    }

    fn update(self: Box<Self>, _ctx: &mut Context) -> Result<Box<dyn State>> {
        Err(anyhow!("Package is not installed not allowed to update it"))
    }
}

impl State for Verified {
    fn verify(self: Box<Self>, _ctx: &mut Context) -> Result<Box<dyn State>> {
        Ok(self)
    }

    fn stage(self: Box<Self>, ctx: &mut Context) -> Result<Box<dyn State>> {
        let mut stmt = ctx
            .install_conn
            .prepare(
                "SELECT package_name FROM owned_files WHERE path = ? AND package_name != ? AND architecture = ?",
            )
            .context("Failed to prepare database query")?;
        let Some(manifest_paths) = &ctx.manifest_paths else {
            return Err(anyhow!("Failed to get the manifest's paths"));
        };

        // File conflict check
        for line in manifest_paths {
            let Some(owner) = stmt
                .query_row(
                    params![
                        line.path.to_string_lossy(),
                        ctx.package.name,
                        ctx.package.architecture.to_string(),
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

        unpack_payload(&ctx.sky_path, &ctx.temp_dir)?;

        Ok(Box::new(Staged))
    }

    fn commit(self: Box<Self>, _ctx: &mut Context) -> Result<Box<dyn State>> {
        Err(anyhow!("Package is not staged not allowed to commit it"))
    }

    fn remove(self: Box<Self>, _ctx: &mut Context) -> Result<Box<dyn State>> {
        Err(anyhow!("Package is not installed not allowed to remove it"))
    }

    fn update(self: Box<Self>, _ctx: &mut Context) -> Result<Box<dyn State>> {
        Err(anyhow!("Package is not installed not allowed to update it"))
    }
}

impl State for Staged {
    fn verify(self: Box<Self>, _ctx: &mut Context) -> Result<Box<dyn State>> {
        Err(anyhow!("Package is staged not allowed to verify it"))
    }

    fn stage(self: Box<Self>, _ctx: &mut Context) -> Result<Box<dyn State>> {
        Ok(self)
    }

    fn commit(self: Box<Self>, ctx: &mut Context) -> Result<Box<dyn State>> {
        fs::create_dir_all(&ctx.install_dir).context("Failed to create the install directory")?;
        ctx.temp_dir.disable_cleanup(true);

        match renameat_with(
            CWD,
            ctx.temp_dir.path(),
            CWD,
            ctx.install_dir.clone(),
            RenameFlags::EXCHANGE,
        ) {
            Ok(_) => {
                remove_dir_all(ctx.temp_dir.path())
                    .context("Failed to recursively delete the temporary directory")?;
                Ok(Box::new(Installed))
            }
            Err(e) => {
                for entry in WalkDir::new(&ctx.temp_dir) {
                    let entry = entry.context("Failed to get entry")?;
                    if entry.file_type().is_dir() {
                        remove_dir_all(&entry.path())
                            .context("Failed to recursively delete the entry")?;
                    } else {
                        remove_file(&entry.path()).context("Failed to delete the entry")?;
                    }
                }
                Err(e.into())
            }
        }
    }

    fn remove(self: Box<Self>, ctx: &mut Context) -> Result<Box<dyn State>> {
        fs::remove_dir_all(ctx.temp_dir.path())
            .context("Failed to recursively delete the temporary directory")?;
        Ok(Box::new(NotInstalled))
    }

    fn update(self: Box<Self>, _ctx: &mut Context) -> Result<Box<dyn State>> {
        Err(anyhow!("Package is not installed not allowed to update it"))
    }
}

impl State for Installed {
    fn verify(self: Box<Self>, _ctx: &mut Context) -> Result<Box<dyn State>> {
        Err(anyhow!("Package is installed not allowed to verify it"))
    }

    fn stage(self: Box<Self>, _ctx: &mut Context) -> Result<Box<dyn State>> {
        Err(anyhow!("Package is installed not allowed to stage it"))
    }

    fn commit(self: Box<Self>, _ctx: &mut Context) -> Result<Box<dyn State>> {
        Err(anyhow!("Package is installed not allowed to commit it"))
    }

    fn remove(self: Box<Self>, ctx: &mut Context) -> Result<Box<dyn State>> {
        fs::remove_dir_all(ctx.install_dir.clone())
            .context("Failed to recursively remove the install directory")?;
        Ok(Box::new(NotInstalled))
    }

    fn update(self: Box<Self>, ctx: &mut Context) -> Result<Box<dyn State>> {
        let mut stmt = ctx.repo_conn.prepare("SELECT sky_path FROM packages WHERE name = ? AND architecture = ? ORDER BY version DESC, release DESC LIMIT 1").context("Failed to prepare database query")?;
        let sky_path: String = stmt.query_row(
            params![
                ctx.package.name.as_str(),
                ctx.package.architecture.to_string()
            ],
            |row| row.get(0),
        )?;
        if sky_path == ctx.sky_path {
            return Err(anyhow!("Package is already up to date"));
        }

        ctx.sky_path = PathBuf::from(sky_path);
        Ok(Box::new(Verified))
    }
}
