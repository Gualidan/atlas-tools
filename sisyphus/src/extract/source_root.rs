use anyhow::{Context, Result};
use std::path::PathBuf;

/// Returns the sole readable top-level entry if it is a directory
/// (e.g., `package-1.0.0/`), otherwise returns `extracted_dir` itself.
/// Failure to open the directory propagates; unreadable entries are skipped,
/// and failure to determine the sole entry's type falls back to `extracted_dir`.
pub fn find_source_root(extracted_dir: &PathBuf) -> Result<PathBuf> {
    // Read all entries (files and directories) from the extracted directory
    let entries: Vec<_> = std::fs::read_dir(extracted_dir)
        .context("Failed to read extracted directory")?
        .filter_map(|e| e.ok())
        .collect();

    // If there's exactly one entry and it's a directory, return it
    if entries.len() == 1
        && let Ok(file_type) = entries[0].file_type()
        && file_type.is_dir()
    {
        return Ok(entries[0].path());
    }

    // Otherwise (multiple entries, or single file, or empty) return the parent
    Ok(extracted_dir.clone())
}
