use std::{
    fs::{self, File, remove_dir_all, remove_file},
    io::{Cursor, Read},
    path::PathBuf,
};

use anyhow::{Context as AnyhowContext, Result, anyhow};
use common::functions::sky_verify::sky_verify;

use rusqlite::params;
use rustix::fs::{CWD, RenameFlags, renameat_with};

use tar::Archive;
use walkdir::WalkDir;
use zstd::Decoder;

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
        let mut stmt = ctx.conn.prepare("SELECT 1 FROM packages WHERE name = ? AND version = ? AND release = ? AND architecture = ? LIMIT 1")?;
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

        sky_verify(&ctx.sky_path, &ctx.package, &ctx.settings)?;

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
        let tar_gz = File::open(&ctx.sky_path).context("Failed to open sky file")?;
        let tar = Decoder::new(tar_gz).context("Failed to decompress the sky file")?;
        let mut archive = Archive::new(tar);

        for entry in archive.entries()? {
            let mut file = entry.context("Failed to get entry")?;

            match file
                .path()
                .context("Failed to derive entry's path")?
                .file_name()
                .and_then(|n| n.to_str())
            {
                Some("payload.tar.zst") => {
                    let mut buffer = Vec::new();
                    file.read_to_end(&mut buffer)
                        .context("Failed to read entry")?;
                    let tar = Decoder::new(Cursor::new(&buffer))
                        .context("Failed to decompress the sky file")?;
                    let mut archive = Archive::new(tar);
                    match archive.unpack(&ctx.temp_dir) {
                        Ok(_) => {}
                        Err(e) => {
                            for entry in WalkDir::new(&ctx.temp_dir) {
                                let entry = entry.context("Failed to get entry")?;
                                if entry.file_type().is_dir() {
                                    remove_dir_all(&entry.path())
                                        .context("Failed to recursively delete the entry")?;
                                } else {
                                    remove_file(&entry.path())
                                        .context("Failed to delete the entry")?;
                                }
                            }
                            return Err(e.into());
                        }
                    }
                }
                _ => {}
            }
        }

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
        let mut stmt = ctx.conn.prepare("SELECT sky_path FROM packages WHERE name = ? AND architecture = ? ORDER BY version DESC, release DESC LIMIT 1").context("Failed to prepare database query")?;
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
