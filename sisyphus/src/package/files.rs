use std::{
    fs::{File, read_link, symlink_metadata},
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::PathBuf,
};

use walkdir::WalkDir;

use crate::{
    checksum::hash::hash,
    types::{
        error::PackageError,
        manifest::{FileType, ManifestLine},
    },
};

pub fn manifest_gen(pkgdir: PathBuf) -> Result<String, PackageError> {
    let mut metadata: Vec<ManifestLine> = vec![];
    for entry in WalkDir::new(&pkgdir) {
        let entry = entry?;
        if entry.path() == pkgdir {
            continue;
        }

        let file_type = symlink_metadata(&entry.path())?.file_type();
        let entry = entry.path();

        let mode = symlink_metadata(&entry)?.mode();

        let manifest_line = match file_type {
            ft if ft.is_file() => {
                let file = File::open(&entry)?;

                let manifest_line = ManifestLine {
                    file_type: FileType::File,
                    mode,
                    target_or_hash: hex::encode(hash(file)?),
                    path: entry.strip_prefix(&pkgdir)?.to_path_buf(),
                };
                manifest_line
            },
            ft if ft.is_dir() => {

                let manifest_line = ManifestLine {
                    file_type: FileType::Dir,
                    mode,
                    target_or_hash: "-".to_string(),
                    path: entry.strip_prefix(&pkgdir)?.to_path_buf(),
                };
                manifest_line
            },
            ft if ft.is_symlink() => {
                let manifest_line = ManifestLine {
                    file_type: FileType::Symlink,
                    mode,
                    target_or_hash: read_link(entry)?.to_string_lossy().to_string(),
                    path: entry.strip_prefix(&pkgdir)?.to_path_buf(),
                };
                manifest_line
            }
            _ => return Err(PackageError::FileTypeError("File in pkgdir has an unexpected file type (allowed filetypes are: file, directory, symlink".to_string(), entry.to_path_buf()))
        };
        metadata.push(manifest_line);
    }
    metadata.sort_by(|a, b| {
        a.path
            .as_os_str()
            .as_bytes()
            .cmp(b.path.as_os_str().as_bytes())
    });
    Ok("".to_string())
}
