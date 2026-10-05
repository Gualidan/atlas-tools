use std::{
    cmp::Ordering,
    fs::{self, File, remove_dir_all, remove_file},
    io::{Cursor, Read},
};

use common::{functions::sky_verify::sky_verify, types::context::Context};
use regex::Regex;
use rustix::fs::{CWD, RenameFlags, renameat_with};
use semver::Version;
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
        let metadata = sky_verify(&ctx.sky_path, &ctx.package, &ctx.settings)?;

        if metadata.is_none() {
            return Err(StateError::VerificationFailed);
        }

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
        let current_version = Version::parse(&ctx.package.version)?;

        let sky_repo = ctx.sky_repo.clone();

        let re = Regex::new(
            r"^(?P<name>.+)-(?P<version>\d+(?:\.\d+)*)-(?P<release>\d+)-(?P<architecture>[^.]+)\.sky$"
        )
        .unwrap();

        for entry in sky_repo
            .read_dir()
            .map_err(StateError::Io)?
            .filter_map(Result::ok)
        {
            let filename = entry.file_name();
            let filename = filename.to_string_lossy();

            // Avoid matching similarly named packages.
            let Some(captures) = re.captures(&filename) else {
                continue;
            };

            if captures.name("name").unwrap().as_str() != ctx.package.name {
                continue;
            }

            let version_text = captures.name("version").unwrap().as_str();

            let Ok(version) = Version::parse(version_text) else {
                continue;
            };

            match current_version.cmp(&version) {
                Ordering::Less => {
                    // The repository has a newer version.
                    ctx.sky_path = entry.path();
                    return Ok(Box::new(Verified));
                }

                Ordering::Equal | Ordering::Greater => {
                    // This file is not newer.
                    continue;
                }
            }
        }

        Err(StateError::UpToDate)
    }
}
