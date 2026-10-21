use std::{
    collections::HashSet,
    io::{Cursor, Read},
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use tar::{
    Archive,
    EntryType::{Link, Regular, Symlink},
};
use zstd::Decoder;

use common::functions::sky_verify::get_sky_archive;

pub fn validate_payload_paths<P: AsRef<Path>>(sky_path: P) -> Result<()> {
    let mut archive = get_sky_archive(&sky_path).context("Failed to get the sky archive")?;
    let mut seen_paths: HashSet<PathBuf> = HashSet::new();
    let mut found_payload = false;

    for entry in archive
        .entries()
        .context("Failed to get archive's entries")?
    {
        let mut entry = entry.context("Failed to get entry")?;
        let path = entry.path().context("Failed to get entry's path")?;

        let Some(filename) = path.file_name().and_then(|n| n.to_str()) else {
            return Err(anyhow!("Failed to get path's filename"));
        };

        match filename {
            "payload.tar.zst" => {
                found_payload = true;
                let mut buffer: Vec<u8> = Vec::new();
                entry
                    .read_to_end(&mut buffer)
                    .context("Failed to read file's bytes")?;

                let tar = Decoder::new(Cursor::new(buffer))
                    .context("Failed to create a zstd decoder for payload.tar.zst")?;
                let mut archive = Archive::new(tar);

                for entry in archive
                    .entries()
                    .context("Failed to get archive's entries")?
                {
                    let entry = entry.context("Failed to get entry")?;
                    let path = entry.path().context("Failed to get entry's path")?;
                    let entry_type = entry.header().entry_type();
                    let mode = entry
                        .header()
                        .mode()
                        .context("Failed to get entry's mode")?;

                    match (entry_type, mode) {
                        (Regular | Link | tar::EntryType::Directory | Symlink, mode)
                            if mode & 0o6000 == 0 => {}
                        _ => {
                            return Err(anyhow!(
                                "Entry at path {} has an invalid type {entry_type:?} or mode {mode:04o}",
                                path.display(),
                            ));
                        }
                    }

                    if path.is_absolute() {
                        return Err(anyhow!("Path {} is absolute", path.display()));
                    }

                    for component in path.components() {
                        match component {
                            component @ Component::ParentDir | component @ Component::CurDir => {
                                return Err(anyhow!(
                                    "Path {} contains a {component:?} component",
                                    path.display()
                                ));
                            }
                            _ => {}
                        }
                    }

                    if !seen_paths.insert(path.to_path_buf()) {
                        return Err(anyhow!("Duplicate payload member {}", path.display()));
                    }
                }
            }
            _ => {}
        }
    }

    if !found_payload {
        return Err(anyhow!("Payload not found in sky file"));
    }
    Ok(())
}
