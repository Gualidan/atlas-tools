use std::{fs::symlink_metadata, path::PathBuf};

use walkdir::WalkDir;

use crate::types::error::PackageError;

pub fn manifest_gen(pkgdir: PathBuf) -> Result<(), PackageError> {
    for entry in WalkDir::new(&pkgdir) {
        let entry = entry?;
        let file_type = symlink_metadata(&entry.path())?.file_type();
        let entry = entry.path();
        let (path, file_type) = match file_type {
            ft if ft.is_file() => (&entry.strip_prefix(&pkgdir)?.to_path_buf(), ft),
            ft if ft.is_dir() => (&entry.strip_prefix(&pkgdir)?.to_path_buf(), ft),
            ft if ft.is_symlink() => (&entry.strip_prefix(&pkgdir)?.to_path_buf(), ft),
            _ => return Err(PackageError::FileTypeError("File in pkgdir has an unexpected file type (allowed filetypes are: file, directory, symlink".to_string(), entry.to_path_buf()))
        };
    }
    Ok(())
}
