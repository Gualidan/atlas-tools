use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};

use common::types::manifest::{FileType, ManifestLine};

/// Parses a sky archive's `files` manifest into records in input order.
/// Each line contains a type (`d`, `f`, or `l`), octal mode, target or hash, and
/// path separated by tabs. Empty input yields no records. Returns an error for
/// missing fields, unknown types, or invalid modes; paths and hashes are not validated.
///
/// # Errors
///
/// Returns an error if a line has missing fields, an unknown type, or an invalid mode.
pub fn parse_metadata_lines(files: &str) -> Result<Vec<ManifestLine>> {
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
                    unknown => {
                        return Err(anyhow!("Invalid file type: {unknown}"));
                    }
                };
                manifest_lines.push(ManifestLine {
                    file_type: kind,
                    mode,
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
