use std::{
    fs::{File, read_link, symlink_metadata},
    io::Cursor,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::PathBuf,
};

use tar::Header;
use walkdir::WalkDir;

use crate::{
    checksum::hash::hash,
    types::{
        error::PackageError,
        manifest::{FileType, ManifestLine},
    },
};

pub fn payload_gen(pkgdir: PathBuf) -> Result<(String, Vec<u8>), PackageError> {
    let mut metadata: Vec<ManifestLine> = vec![];

    let writer = Cursor::new(Vec::new());
    let mut archive = tar::Builder::new(zstd::Encoder::new(writer, 3)?);

    for entry in WalkDir::new(&pkgdir) {
        let entry = entry?;
        if entry.path() == pkgdir {
            continue;
        }

        let file_type = symlink_metadata(&entry.path())?.file_type();
        let path = entry.path();

        let mode = symlink_metadata(&path)?.mode();
        let relative_path = path.strip_prefix(&pkgdir)?.to_path_buf();

        let manifest_line = match file_type {
            ft if ft.is_file() => {
                let mut file = File::open(&path)?;

                let manifest_line = ManifestLine {
                    file_type: FileType::File,
                    mode,
                    target_or_hash: hex::encode(hash(&File::open(&path)?)?),
                    path: relative_path.clone(),
                };

                let mut header = Header::new_gnu();
                header.set_username("root")?;
                header.set_uid(0);
                header.set_gid(0);
                header.set_mode(mode & 0o7777);
                header.set_size(path.metadata()?.len());

                archive.append_data(&mut header, relative_path, &mut file)?;
                manifest_line
            },
            ft if ft.is_dir() => {
                let mut header = Header::new_gnu();
                header.set_username("root")?;
                header.set_entry_type(tar::EntryType::Directory);
                header.set_uid(0);
                header.set_gid(0);
                header.set_mode(mode & 0o7777);
                header.set_size(0);
                let data: &[u8] = &[];

                archive.append_data(&mut header, &relative_path, data)?;

                let manifest_line = ManifestLine {
                    file_type: FileType::Dir,
                    mode,
                    target_or_hash: "-".to_string(),
                    path: relative_path,
                };
                manifest_line
            },
            ft if ft.is_symlink() => {
                let target = read_link(path)?;

                let mut header = Header::new_gnu();
                header.set_username("root")?;
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_link_name(&target)?;
                header.set_size(0);
                header.set_uid(0);
                header.set_mode(mode & 0o7777);
                header.set_gid(0);
                let data: &[u8] = &[];

                archive.append_data(&mut header, &relative_path, data)?;

                let manifest_line = ManifestLine {
                    file_type: FileType::Symlink,
                    mode,
                    target_or_hash: read_link(path)?.to_string_lossy().to_string(),
                    path: relative_path,
                };
                manifest_line
            }
            _ => return Err(PackageError::FileTypeError("File in pkgdir has an unexpected file type (allowed filetypes are: file, directory, symlink".to_string(), path.to_path_buf()))
        };
        metadata.push(manifest_line);
    }
    let archive = archive.into_inner()?.finish()?.into_inner();
    metadata.sort_by(|a, b| {
        a.path
            .as_os_str()
            .as_bytes()
            .cmp(b.path.as_os_str().as_bytes())
    });

    let mut manifest = String::new();
    for line in metadata {
        manifest.push_str(&line.format()?);
    }

    Ok((manifest, archive))
}
