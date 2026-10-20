use std::{
    fs::{self},
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

use crate::types::context::Context;

pub struct NotInstalled;

pub struct Verified;

pub struct Staged;

pub struct Installed;

pub trait State {
    fn verify(self: Box<Self>, _ctx: &mut Context) -> Result<Box<dyn State>> {
        return Err(anyhow!("Illegal transaction cannot verify from this state"));
    }
    fn stage(self: Box<Self>, _ctx: &mut Context) -> Result<Box<dyn State>> {
        return Err(anyhow!("Illegal transaction cannot stage from this state"));
    }
    fn commit(self: Box<Self>, _ctx: &mut Context) -> Result<Box<dyn State>> {
        return Err(anyhow!("Illegal transaction cannot commit from this state"));
    }
    fn remove(self: Box<Self>, _ctx: &mut Context) -> Result<Box<dyn State>> {
        return Err(anyhow!("Illegal transaction cannot remove from this state"));
    }
    fn update(self: Box<Self>, _ctx: &mut Context) -> Result<Box<dyn State>> {
        return Err(anyhow!("Illegal transaction cannot update from this state"));
    }
}

impl State for NotInstalled {
    fn verify(self: Box<Self>, ctx: &mut Context) -> Result<Box<dyn State>> {
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
        ctx.metadata = Some(metadata);
        let (_, _, files, _) =
            read_sky(&PathBuf::from(&ctx.sky_path)).context("Failed to read sky file")?;
        ctx.manifest_paths =
            Some(parse_metadata_lines(files).context("Failed to parse manifest's lines")?);

        Ok(Box::new(Verified))
    }
}

impl State for Verified {
    fn verify(self: Box<Self>, _ctx: &mut Context) -> Result<Box<dyn State>> {
        Ok(self)
    }

    fn stage(self: Box<Self>, ctx: &mut Context) -> Result<Box<dyn State>> {
        let Some(ref mut temp_dir) = ctx.temp_dir else {
            return Err(anyhow!("No temp dir"));
        };
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

        unpack_payload(&ctx.sky_path, &temp_dir)?;

        Ok(Box::new(Staged))
    }
}

impl State for Staged {
    fn stage(self: Box<Self>, _ctx: &mut Context) -> Result<Box<dyn State>> {
        Ok(self)
    }

    fn commit(self: Box<Self>, ctx: &mut Context) -> Result<Box<dyn State>> {
        let Some(ref mut temp_dir) = ctx.temp_dir else {
            return Err(anyhow!("No temp dir"));
        };
        fs::create_dir_all(&ctx.install_dir).context("Failed to create the install directory")?;

        match renameat_with(
            CWD,
            temp_dir.path(),
            CWD,
            ctx.install_dir.clone(),
            RenameFlags::EXCHANGE,
        ) {
            Ok(_) => Ok(Box::new(Installed)),
            Err(e) => Err(e.into()),
        }
    }

    fn remove(self: Box<Self>, ctx: &mut Context) -> Result<Box<dyn State>> {
        let Some(ref mut temp_dir) = ctx.temp_dir else {
            return Err(anyhow!("No temp dir"));
        };
        fs::remove_dir_all(temp_dir.path())
            .context("Failed to recursively delete the temporary directory")?;

        Ok(Box::new(NotInstalled))
    }
}

impl State for Installed {
    fn remove(self: Box<Self>, ctx: &mut Context) -> Result<Box<dyn State>> {
        let mut stmt  = ctx.install_conn.prepare("SELECT package_name FROM package_dependencies WHERE dependency_name = ? AND architecture = ?").context("Failed to prepare datatbase query")?;
        let Some(dependent): Option<String> = stmt
            .query_row(
                params![ctx.package_id.name, ctx.package_id.architecture.to_string()],
                |row| row.get(0),
            )
            .optional()?
        else {
            fs::remove_dir_all(ctx.install_dir.clone())
                .context("Failed to recursively remove the install directory")?;
            return Ok(Box::new(NotInstalled));
        };

        Err(anyhow!(
            "Cannot remove package {}: {dependent} depends on it",
            ctx.package_id.name
        ))
    }

    fn update(self: Box<Self>, ctx: &mut Context) -> Result<Box<dyn State>> {
        let mut stmt = ctx.repo_conn.prepare("SELECT sky_path FROM packages WHERE name = ? AND architecture = ? ORDER BY version DESC, release DESC LIMIT 1").context("Failed to prepare database query")?;
        let sky_path: String = stmt.query_row(
            params![
                ctx.package_id.name.as_str(),
                ctx.package_id.architecture.to_string()
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
