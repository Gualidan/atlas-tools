use std::{
    fs::{File, remove_dir_all, remove_file},
    io::{Cursor, Read},
    path::PathBuf,
};

use anyhow::{Context, Result};
use tar::Archive;
use tempfile::TempDir;
use walkdir::WalkDir;
use zstd::Decoder;

pub fn unpack_payload(path: &PathBuf, destination: &TempDir) -> Result<()> {
    let tar_gz = File::open(path).context("Failed to open sky file")?;
    let tar = Decoder::new(tar_gz).context("Failed to decompress the sky file")?;
    let mut archive = Archive::new(tar);

    for entry in archive.entries()? {
        let mut file = entry.context("Failed to get entry")?;

        match file
            .path()
            .context("Failed to get entry's path")?
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
                match archive.unpack(&destination) {
                    Ok(_) => {}
                    Err(e) => {
                        for entry in WalkDir::new(&destination) {
                            let entry = entry.context("Failed to get entry")?;
                            if entry.file_type().is_dir() {
                                remove_dir_all(&entry.path())
                                    .context("Failed to recursively delete the entry")?;
                            } else {
                                remove_file(&entry.path()).context("Failed to delete the entry")?;
                            }
                        }
                        return Err(e.into());
                    }
                }
            }
            _ => {}
        }
    }

    Ok(())
}
