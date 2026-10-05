use std::{
    fs::{self, File},
    io::{Cursor, Read},
};

use common::{functions::sky_verify::sky_verify, types::context::Context};
use tar::Archive;
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
                    archive.unpack(&ctx.temp_dir)?;
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
        fs::rename(ctx.temp_dir.clone(), ctx.install_dir.clone())?;
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

    fn update(self, _ctx: &mut Context) -> Result<Box<dyn State>, StateError> {
        todo!("implement update logic")
    }
}
