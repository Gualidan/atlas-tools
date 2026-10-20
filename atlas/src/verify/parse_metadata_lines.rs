use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};

use common::types::manifest::{FileType, ManifestLine};

/// From the string corresponding to the content of the "files" file of a sky archive parse the manifest and return all of its lines
pub fn parse_metadata_lines(files: String) -> Result<Vec<ManifestLine>> {
    let lines: Vec<&str> = files.lines().collect();
    let mut manifest_lines: Vec<ManifestLine> = Vec::new();

    for line in lines {
        let fields: Vec<&str> = line.splitn(4, '\t').collect();

        match fields[..] {
            [kind, mode, target_or_hash, path] => {
                let mode = u32::from_str_radix(mode, 8)
                    .context("Failed to parse mode, is it malformed?")?;
                let kind = match kind {
                    "d" => FileType::Dir,
                    "f" => FileType::File,
                    "l" => FileType::Symlink,
                    unknown @ _ => {
                        return Err(anyhow!("Invalid file type: {}", unknown));
                    }
                };
                manifest_lines.push(ManifestLine {
                    file_type: kind,
                    mode: mode,
                    target_or_hash: target_or_hash.to_string(),
                    path: PathBuf::from(path),
                });
            }
            _ => {
                return Err(anyhow!("Invalid line"));
            }
        }
    }

    Ok(manifest_lines)
}
