use std::{
    collections::HashSet,
    path::{Component, Path},
};

use anyhow::{Context, Result, anyhow};

use common::functions::sky_verify::get_sky_archive;

/// Validates that a sky archive contains only the expected, unique root entries.
///
/// # Errors
///
/// Returns an error if archive entries cannot be read, paths are unsafe, entries
/// are unexpected or duplicated, or required entries are missing.
pub fn validate_archive_paths<P: AsRef<Path>>(sky_path: P) -> Result<()> {
    let mut archive = get_sky_archive(&sky_path)?;
    let mut seen_names: HashSet<String> = HashSet::new();

    for entry in archive
        .entries()
        .context("Failed to get archive's entries")?
    {
        let entry = entry.context("Failed to get entry")?;
        let path = entry.path().context("Failed to get entry's path")?;

        if path.is_absolute() {
            return Err(anyhow!(
                "Archive member {} has an absolute path",
                path.display()
            ));
        }

        for component in path.components() {
            if let component @ (Component::ParentDir | Component::CurDir) = component {
                return Err(anyhow!(
                    "Archive member {} has a {component:?} component",
                    path.display(),
                ));
            }
        }

        let Some(filename) = path.file_name().and_then(|n| n.to_str()) else {
            return Err(anyhow!("Failed to get path's filename"));
        };

        match filename {
            name @ ("metadata.yaml" | "payload.tar.zst" | "files" | "signature.ed25519") => {
                if !seen_names.insert(name.to_string()) {
                    return Err(anyhow!("Duplicate archive member {name}"));
                }
            }
            other => {
                return Err(anyhow!("Unexpected archive member {other}"));
            }
        }
    }

    if !seen_names.contains("metadata.yaml")
        || !seen_names.contains("payload.tar.zst")
        || !seen_names.contains("files")
        || !seen_names.contains("signature.ed25519")
    {
        return Err(anyhow!("Archive is missing one of its expected files"));
    }
    Ok(())
}
