use std::{
    fs::File,
    io::{Cursor, Read},
    path::PathBuf,
};

use anyhow::{Context, Result};
use tar::Archive;
use tempfile::TempDir;
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
                archive
                    .unpack(&destination)
                    .context("Failed to unpack archive to destination")?;
            }
            _ => {}
        }
    }

    Ok(())
}
