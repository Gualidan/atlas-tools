use std::{
    fs::{self, File, remove_dir_all, remove_file},
    io::{Cursor, Read},
};

use common::{functions::sky_verify::sky_verify, types::context::Context};
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
        renameat_with(
            CWD,
            ctx.temp_dir.clone(),
            CWD,
            ctx.install_dir.clone(),
            RenameFlags::EXCHANGE,
        )?;
        Ok(Box::new(Installed))
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

        let sky_repo = ctx.sky_path.ancestors().nth(2).unwrap();

        for entry in sky_repo
            .read_dir()
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(&ctx.package.name))
        {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let version = file_name.split("-").collect::<Vec<_>>();
            let version = Version::parse(version.get(1).unwrap())?;

            match current_version.cmp(&version) {
                std::cmp::Ordering::Less => {
                    ctx.sky_path = entry.path();
                    return Ok(Box::new(Verified));
                }
                std::cmp::Ordering::Equal => {
                    return Err(StateError::UpToDate);
                }
                std::cmp::Ordering::Greater => {
                    return Err(StateError::UpToDate);
                }
            };
        }

        Err(StateError::UpToDate)
    }
}
