use crate::types::error::ExtractError;
use std::path::PathBuf;

/// Find the single top-level directory in an extracted archive
/// Many source tarballs have a single directory at the root (e.g., package-1.0.0/)
/// This function finds and returns that directory path
pub fn find_source_root(extracted_dir: &PathBuf) -> Result<PathBuf, ExtractError> {
    // Read all entries (files and directories) from the extracted directory
    let entries: Vec<_> = std::fs::read_dir(extracted_dir)?
        .filter_map(|e| e.ok())
        .collect();

    // If there's exactly one entry and it's a directory, return it
    if entries.len() == 1 {
        if let Ok(file_type) = entries[0].file_type() {
            if file_type.is_dir() {
                return Ok(entries[0].path());
            }
        }
    }

    // Otherwise (multiple entries, or single file, or empty) return the parent
    Ok(extracted_dir.clone())
}
