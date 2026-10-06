use std::{
    fs::{self, File, remove_dir_all, remove_file},
    io::{Cursor, Read},
};

use common::{functions::sky_verify::sky_verify, types::context::Context};

use rusqlite::params;
use rustix::fs::{CWD, RenameFlags, renameat_with};

use tar::Archive;
use walkdir::WalkDir;
use zstd::Decoder;

use crate::types::error::State as StateError;

pub struct NotInstalled;

pub struct Verified;

pub struct Staged;

pub struct Installed;

pub trait State {
    fn verify(self, ctx: &mut Context) -> Result<Option<Box<dyn State>>, StateError>;
    fn stage(self, ctx: &mut Context) -> Result<Box<dyn State>, StateError>;
    fn commit(self, ctx: &mut Context) -> Result<Box<dyn State>, StateError>;
    fn remove(self, ctx: &mut Context) -> Result<Box<dyn State>, StateError>;
    fn update(self, ctx: &mut Context) -> Result<Box<dyn State>, StateError>;
}

impl State for NotInstalled {
    fn verify(self, ctx: &mut Context) -> Result<Option<Box<dyn State>>, StateError> {
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
            return Err(StateError::VerificationFailed);
        }

        sky_verify(&ctx.sky_path, &ctx.package, &ctx.settings)?;

        Ok(Some(Box::new(Verified)))
    }

    fn stage(self, _ctx: &mut Context) -> Result<Box<dyn State>, StateError> {
        Err(StateError::NotVerified)
    }

    fn commit(self, _ctx: &mut Context) -> Result<Box<dyn State>, StateError> {
        Err(StateError::NotStaged)
    }

    fn remove(self, _ctx: &mut Context) -> Result<Box<dyn State>, StateError> {
        Err(StateError::NotInstalled(
            "not allowed to remove".to_string(),
        ))
    }

    fn update(self, _ctx: &mut Context) -> Result<Box<dyn State>, StateError> {
        Err(StateError::NotInstalled(
            "not allowed to update".to_string(),
        ))
    }
}

impl State for Verified {
    fn verify(self, _ctx: &mut Context) -> Result<Option<Box<dyn State>>, StateError> {
        Ok(Some(Box::new(self)))
    }

    fn stage(self, ctx: &mut Context) -> Result<Box<dyn State>, StateError> {
        let tar_gz = File::open(&ctx.sky_path)?;
        let tar = Decoder::new(tar_gz)?;
        let mut archive = Archive::new(tar);

        for entry in archive.entries()? {
            let mut file = entry?;

            match file.path()?.file_name().and_then(|n| n.to_str()) {
                Some("payload.tar.zst") => {
                    let mut buffer = Vec::new();
                    file.read_to_end(&mut buffer)?;
                    let tar = Decoder::new(Cursor::new(&buffer))?;
                    let mut archive = Archive::new(tar);
                    match archive.unpack(&ctx.temp_dir) {
                        Ok(_) => {}
                        Err(e) => {
                            for entry in WalkDir::new(&ctx.temp_dir) {
                                let entry = entry?;
                                if entry.file_type().is_dir() {
                                    remove_dir_all(&entry.path())?;
                                } else {
                                    remove_file(&entry.path())?;
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

    fn commit(self, _ctx: &mut Context) -> Result<Box<dyn State>, StateError> {
        Err(StateError::NotStaged)
    }

    fn remove(self, _ctx: &mut Context) -> Result<Box<dyn State>, StateError> {
        Err(StateError::NotInstalled(
            "not allowed to remove".to_string(),
        ))
    }

    fn update(self, _ctx: &mut Context) -> Result<Box<dyn State>, StateError> {
        Err(StateError::NotInstalled(
            "not allowed to update".to_string(),
        ))
    }
}

impl State for Staged {
    fn verify(self, _ctx: &mut Context) -> Result<Option<Box<dyn State>>, StateError> {
        Err(StateError::Staged("not allowed to verify".to_string()))
    }

    fn stage(self, _ctx: &mut Context) -> Result<Box<dyn State>, StateError> {
        Ok(Box::new(self))
    }

    fn commit(self, ctx: &mut Context) -> Result<Box<dyn State>, StateError> {
        match renameat_with(
            CWD,
            ctx.temp_dir.clone(),
            CWD,
            ctx.install_dir.clone(),
            RenameFlags::EXCHANGE,
        ) {
            Ok(_) => Ok(Box::new(Installed)),
            Err(e) => {
                for entry in WalkDir::new(&ctx.temp_dir) {
                    let entry = entry?;
                    if entry.file_type().is_dir() {
                        remove_dir_all(&entry.path())?;
                    } else {
                        remove_file(&entry.path())?;
                    }
                }
                Err(StateError::Io(e.into()))
            }
        }
    }

    fn remove(self, ctx: &mut Context) -> Result<Box<dyn State>, StateError> {
        fs::remove_dir_all(ctx.temp_dir.clone())?;
        Ok(Box::new(NotInstalled))
    }

    fn update(self, _ctx: &mut Context) -> Result<Box<dyn State>, StateError> {
        Err(StateError::NotInstalled(
            "not allowed to update".to_string(),
        ))
    }
}

impl State for Installed {
    fn verify(self, _ctx: &mut Context) -> Result<Option<Box<dyn State>>, StateError> {
        Err(StateError::Installed("not allowed to verify".to_string()))
    }

    fn stage(self, _ctx: &mut Context) -> Result<Box<dyn State>, StateError> {
        Err(StateError::Installed("not allowed to stage".to_string()))
    }

    fn commit(self, _ctx: &mut Context) -> Result<Box<dyn State>, StateError> {
        Err(StateError::Installed("not allowed to commit".to_string()))
    }

    fn remove(self, ctx: &mut Context) -> Result<Box<dyn State>, StateError> {
        fs::remove_dir_all(ctx.install_dir.clone())?;
        Ok(Box::new(NotInstalled))
    }

    fn update(self, ctx: &mut Context) -> Result<Box<dyn State>, StateError> {
        let mut stmt = ctx.conn.prepare("SELECT sky_path FROM packages WHERE name = ? AND architecture = ? ORDER BY version DESC, release DESC LIMIT 1")?;
        let sky_path: String = stmt.query_row(
            params![
                ctx.package.name.as_str(),
                ctx.package.architecture.to_string()
            ],
            |row| row.get(0),
        )?;
        Ok(Box::new(Installed))
    }
}
