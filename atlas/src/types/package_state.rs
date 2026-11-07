use std::{
    fs::{self},
    io::ErrorKind::NotFound,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context as AnyhowContext, Result, anyhow};
use common::{
    functions::{
        sky_verify::{read_sky, sky_verify},
        unpack_payload::unpack_payload,
    },
    types::{
        config::SisyphusConfig,
        manifest::{FileType, ManifestLine},
        metadata::Metadata,
        package::PackageIdentity,
    },
};

use rusqlite::{Connection, OptionalExtension, params};
use rustix::fs::{CWD, RenameFlags, renameat_with};
use tempfile::TempDir;

use crate::{
    types::{
        contexts::RemoveContext,
        guards::{GuardedEntry, rollback_entries},
    },
    verify::{
        parse_metadata_lines::parse_metadata_lines, validate_archive::validate_archive_paths,
        validate_payload::validate_payload_paths,
    },
};

pub enum PackageState<'a> {
    NotInstalled {
        package_id: PackageIdentity,
        install_conn: &'a Connection,
        tempdir: &'a TempDir,
        sky_path: &'a PathBuf,
        config: &'a SisyphusConfig,
        repo_conn: &'a Connection,
        install_dir: &'a PathBuf,
    },
    Verified {
        metadata: Metadata,
        manifest_paths: Vec<ManifestLine>,
        tempdir: &'a TempDir,
        install_conn: &'a Connection,
        package_id: PackageIdentity,
        sky_path: &'a PathBuf,
        install_dir: &'a PathBuf,
    },
    Staged {
        tempdir: &'a TempDir,
        metadata: Metadata,
        install_dir: &'a PathBuf,
        sky_path: &'a PathBuf,
    },
    Installed,
}

fn has_parent_dir(path: &Path) -> bool {
    path.components().any(|c| c == Component::ParentDir)
}

fn symlink_target_escapes_root(link_path: &Path, target: &Path) -> bool {
    let mut resolved = PathBuf::new();

    if let Some(parent) = link_path.parent() {
        for component in parent.components() {
            match component {
                Component::Normal(name) => resolved.push(name),
                _ => return true,
            }
        }
    }

    for component in target.components() {
        match component {
            Component::Normal(name) => resolved.push(name),
            Component::ParentDir if !resolved.pop() => return true,
            Component::RootDir | Component::Prefix(_) => return true,
            Component::CurDir | Component::ParentDir => {}
        }
    }

    false
}

fn touches_atlas_state(path: &Path, file_type: &FileType) -> bool {
    let atlas_state = Path::new("var/lib/atlas");

    path == atlas_state
        || path.starts_with(atlas_state)
        || (file_type != &FileType::Dir && atlas_state.starts_with(path))
}

pub(crate) fn ensure_no_symlink_components(
    root: &Path,
    relative: &Path,
    allow_final_symlink: bool,
) -> Result<()> {
    if relative.is_absolute() {
        return Err(anyhow!("Path {} is absolute", relative.display()));
    }

    let mut current = root.to_path_buf();
    let mut components = relative.components().peekable();

    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err(anyhow!(
                "Path {} contains an unsafe component",
                relative.display()
            ));
        };
        current.push(name);

        match fs::symlink_metadata(&current) {
            Ok(metadata)
                if metadata.file_type().is_symlink()
                    && (components.peek().is_some() || !allow_final_symlink) =>
            {
                return Err(anyhow!(
                    "Path {} traverses symlink {}",
                    relative.display(),
                    current.display()
                ));
            }
            Ok(metadata) if components.peek().is_some() && !metadata.is_dir() => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotADirectory,
                    format!("Parent of {} is not a directory", relative.display()),
                )
                .into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }

    Ok(())
}

fn create_owned_dirs(root: &Path, relative: &Path, entries: &mut Vec<GuardedEntry>) -> Result<()> {
    let mut path = PathBuf::new();
    for component in relative.components() {
        path.push(component);
        match fs::create_dir(root.join(&path)) {
            Ok(()) => entries.push(GuardedEntry::CreatedDir(path.clone())),
            Err(error)
                if error.kind() == std::io::ErrorKind::AlreadyExists
                    && fs::symlink_metadata(root.join(&path))?.is_dir() => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

impl<'a> PackageState<'a> {
    /// Advances `NotInstalled` to `Verified` after checking the exact repository
    /// identity, archive signature and metadata, archive paths, and manifest syntax.
    /// Propagates database, archive, key, and validation errors, including a missing
    /// repository row. Other starting states return an invalid-state error.
    ///
    /// # Errors
    ///
    /// Returns an error if database access, archive verification, or validation fails,
    /// or if the state is not `NotInstalled`.
    pub fn verify(self) -> Result<PackageState<'a>> {
        match self {
            PackageState::NotInstalled {
                package_id,
                install_conn,
                tempdir,
                sky_path,
                config,
                repo_conn,
                install_dir,
            } => {
                let mut stmt = repo_conn.prepare("SELECT 1 FROM packages WHERE name = ? AND version = ? AND release = ? AND architecture = ? LIMIT 1")?;
                let rows: bool = stmt.query_row(
                    params![
                        package_id.name,
                        package_id.version,
                        package_id.release,
                        package_id.architecture.to_string()
                    ],
                    |row| row.get(0),
                )?;

                if !rows {
                    return Err(anyhow!("Verification failed"));
                }

                let metadata = sky_verify(sky_path, &package_id, config)?;
                validate_archive_paths(sky_path).context("Failed to validate archive's paths")?;
                validate_payload_paths(sky_path).context("Failed to validate payload's paths")?;

                let (_, _, files, _) =
                    read_sky(PathBuf::from(&sky_path)).context("Failed to read sky file")?;
                let manifest_paths =
                    parse_metadata_lines(&files).context("Failed to parse manifest's lines")?;

                Ok(PackageState::Verified {
                    metadata,
                    manifest_paths,
                    install_conn,
                    package_id,
                    tempdir,
                    sky_path,
                    install_dir,
                })
            }
            _ => Err(anyhow!(
                "Invalid transaction not allowed to verify from this state"
            )),
        }
    }

    /// Advances `Verified` to `Staged` by extracting the payload into its temporary
    /// directory. Rejects non-directory paths owned by another package or
    /// colliding with unowned filesystem entries; paths owned by this package
    /// are allowed.
    /// Propagates database and extraction errors. Other starting states return
    /// an invalid-state error.
    ///
    /// # Errors
    ///
    /// Returns an error if ownership lookup or payload extraction fails, or if
    /// the state is not `Verified`.
    pub fn stage(self) -> Result<PackageState<'a>> {
        match self {
            PackageState::Verified {
                metadata,
                manifest_paths,
                tempdir,
                install_conn,
                package_id,
                sky_path,
                install_dir,
            } => {
                let mut stmt = install_conn
                    .prepare(
                        "SELECT package_name FROM owned_files WHERE path = ? AND architecture = ?",
                    )
                    .context("Failed to prepare database query")?;

                for line in manifest_paths {
                    if matches!(line.file_type, FileType::Dir) {
                        continue;
                    }

                    let owner = stmt
                        .query_row(
                            params![
                                line.path.to_string_lossy(),
                                package_id.architecture.to_string(),
                            ],
                            |row| row.get::<_, String>(0),
                        )
                        .optional()?;

                    if let Some(owner) = owner {
                        if owner != package_id.name {
                            return Err(anyhow!(
                                "Failed to install `{}`: conflicting file of package `{owner}` already exists at `{}`",
                                package_id.name,
                                line.path.display()
                            ));
                        }
                        continue;
                    }

                    match fs::symlink_metadata(install_dir.join(&line.path)) {
                        Ok(_) => {
                            return Err(anyhow!(
                                "Failed to install `{}`: conflicting file `{}` already exists",
                                package_id.name,
                                line.path.display()
                            ));
                        }
                        Err(error) if error.kind() == NotFound => {}
                        Err(error) => {
                            return Err(error).context("Failed to inspect install destination");
                        }
                    }
                }

                unpack_payload(sky_path, tempdir)?;

                Ok(PackageState::Staged {
                    tempdir,
                    metadata,
                    install_dir,
                    sky_path,
                })
            }
            _ => Err(anyhow!(
                "Invalid transaction not allowed to stage from this state"
            )),
        }
    }

    /// Exchanges a `Staged` payload with the install directory, creating the
    /// destination first if needed. Both directories must share a filesystem.
    /// Returns `Installed`, the metadata, and the temporary directory now holding
    /// the previous destination contents; dropping it attempts to remove them.
    ///
    /// Propagates directory creation and exchange errors. A failed exchange can
    /// leave a newly created destination behind. Other states return an error.
    ///
    /// # Errors
    ///
    /// Returns an error if directory creation or exchange fails, or if the state
    /// is not `Staged`.
    #[allow(clippy::too_many_lines)]
    pub fn commit(self) -> Result<(PackageState<'a>, Metadata, &'a TempDir, Vec<GuardedEntry>)> {
        let mut entries: Vec<GuardedEntry> = Vec::new();

        match self {
            PackageState::Staged {
                tempdir,
                metadata,
                install_dir,
                sky_path,
            } => {
                let (_, _, files, _) = read_sky(sky_path).context("Failed to read sky file")?;
                let manifest_lines =
                    parse_metadata_lines(&files).context("Failed to parse metadata lines")?;

                for line in &manifest_lines {
                    if touches_atlas_state(&line.path, &line.file_type) {
                        return Err(anyhow!(
                            "Path {} conflicts with reserved Atlas state directory /var/lib/atlas",
                            line.path.display()
                        ));
                    }
                    ensure_no_symlink_components(
                        install_dir,
                        &line.path,
                        line.file_type != FileType::Dir,
                    )?;
                }

                // Preflight the entire payload before the first destination mutation.
                for line in &manifest_lines {
                    let staged = fs::symlink_metadata(tempdir.path().join(&line.path))?;
                    let destination = fs::symlink_metadata(install_dir.join(&line.path));
                    match destination {
                        Ok(ref dest) if dest.is_dir() != staged.is_dir() => {
                            return Err(anyhow!(
                                "Destination type conflicts at {}",
                                line.path.display()
                            ));
                        }
                        Err(ref error) if error.kind() != NotFound => {
                            return Err(anyhow!("{error}"));
                        }
                        _ => {}
                    }
                    if staged.file_type().is_symlink() {
                        let target = fs::read_link(tempdir.path().join(&line.path))?;
                        if symlink_target_escapes_root(&line.path, &target) {
                            return Err(anyhow!(
                                "Unsafe symlink target at {}",
                                line.path.display()
                            ));
                        }
                    } else if !staged.is_dir() && !staged.is_file() {
                        return Err(anyhow!("Unsupported payload type"));
                    }
                }

                let mutation = (|| -> Result<()> {
                    fs::create_dir_all(install_dir)
                        .context("Failed to create the install directory")?;

                    for line in manifest_lines {
                        let staged_path = tempdir.path().join(&line.path);

                        let file_type = fs::symlink_metadata(&staged_path)
                            .context("Failed to get file type")?
                            .file_type();

                        match () {
                            () if file_type.is_dir() => {
                                if has_parent_dir(&line.path) {
                                    return Err(anyhow!(
                                        "Path {} has a parent directory meaning it is potentially trying to escape the installation directory",
                                        staged_path.display()
                                    ));
                                }

                                let install_path = install_dir.join(&line.path);
                                match fs::symlink_metadata(&install_path) {
                                    Ok(metadata) if metadata.is_dir() => {}
                                    Ok(_) => {
                                        return Err(anyhow!(
                                            "Destination {} exists but is not a directory",
                                            install_path.display()
                                        ));
                                    }
                                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                                        create_owned_dirs(install_dir, &line.path, &mut entries)?;
                                    }
                                    Err(error) => {
                                        return Err(error)
                                            .context("Failed to inspect directory destination");
                                    }
                                }
                            }
                            () if file_type.is_file() => {
                                if has_parent_dir(&line.path) {
                                    return Err(anyhow!(
                                        "Path {} has a parent directory meaning it is potentially trying to escape the installation directory",
                                        line.path.display()
                                    ));
                                }

                                let temp_path = tempdir.path().join(&line.path);
                                let install_path = install_dir.join(&line.path);
                                create_owned_dirs(
                                    install_dir,
                                    line.path.parent().context("Missing parent")?,
                                    &mut entries,
                                )?;

                                match fs::symlink_metadata(&install_path) {
                                    Ok(_) => {
                                        renameat_with(
                                            CWD,
                                            temp_path,
                                            CWD,
                                            install_path,
                                            RenameFlags::EXCHANGE,
                                        )
                                        .context("Failed to exchange files")?;

                                        entries.push(GuardedEntry::SwappedFile(line.path));
                                    }
                                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                        renameat_with(
                                            CWD,
                                            temp_path,
                                            CWD,
                                            install_path,
                                            RenameFlags::NOREPLACE,
                                        )
                                        .context("Failed to exchange files")?;

                                        entries.push(GuardedEntry::CreatedFile(line.path));
                                    }
                                    Err(e) => {
                                        return Err(e.into());
                                    }
                                }
                            }

                            () => {
                                if file_type.is_symlink() {
                                    if has_parent_dir(&line.path)
                                        || symlink_target_escapes_root(
                                            &line.path,
                                            Path::new(&line.target_or_hash),
                                        )
                                    {
                                        return Err(anyhow!(
                                            "Symlink {} has target {} meaning it is potentially trying to escape the installation directory",
                                            line.path.display(),
                                            line.target_or_hash
                                        ));
                                    }

                                    let install_path = install_dir.join(&line.path);
                                    create_owned_dirs(
                                        install_dir,
                                        line.path.parent().context("Missing parent")?,
                                        &mut entries,
                                    )?;

                                    match fs::symlink_metadata(&install_path) {
                                        Ok(_) => {
                                            renameat_with(
                                                CWD,
                                                staged_path,
                                                CWD,
                                                install_path,
                                                RenameFlags::EXCHANGE,
                                            )
                                            .context("Failed to exchange symlinks")?;

                                            entries.push(GuardedEntry::SwappedFile(line.path));
                                        }
                                        Err(error)
                                            if error.kind() == std::io::ErrorKind::NotFound =>
                                        {
                                            renameat_with(
                                                CWD,
                                                staged_path,
                                                CWD,
                                                install_path,
                                                RenameFlags::NOREPLACE,
                                            )
                                            .context("Failed to install symlink")?;

                                            entries.push(GuardedEntry::CreatedSymlink(line.path));
                                        }
                                        Err(error) => {
                                            return Err(error)
                                                .context("Failed to inspect symlink destination");
                                        }
                                    }
                                }
                            }
                        }
                    }

                    Ok(())
                })();
                if let Err(error) = mutation {
                    rollback_entries(install_dir, tempdir, &entries);
                    return Err(error);
                }
                Ok((PackageState::Installed, metadata, tempdir, entries))
            }
            _ => Err(anyhow!(
                "Invalid transaction not allowed to commit from this state"
            )),
        }
    }

    /// Removes the package's owned paths from `ctx.install_dir` in the
    /// `Installed` state. A missing path counts as success; an owned directory
    /// is removed only while it is empty. Dependency checks and database record
    /// removal are left to the caller.
    /// Returns an error for another starting state and propagates filesystem
    /// errors; failed removal may leave some paths deleted.
    ///
    /// # Errors
    ///
    /// Returns an error if filesystem removal or directory listing fails, or if
    /// the state is not `Installed`.
    pub fn remove(self, ctx: &mut RemoveContext) -> Result<()> {
        match self {
            PackageState::Installed => {
                let mut owned_files_query = ctx
                    .install_conn
                    .prepare(
                        "SELECT path FROM owned_files WHERE package_name = ? AND architecture = ? ORDER BY path DESC",
                    )
                    .context("Failed to prepare owned files query")?;
                let owned_files: Vec<String> = owned_files_query
                    .query_map(
                        params![ctx.package_id.name, ctx.package_id.architecture.to_string()],
                        |row| row.get(0),
                    )?
                    .collect::<Result<Vec<_>, _>>()
                    .context("Failed to query owned files")?;
                for file in owned_files {
                    ensure_no_symlink_components(&ctx.install_dir, Path::new(&file), true)?;
                    let path = ctx.install_dir.join(&file);
                    match fs::symlink_metadata(&path) {
                        Ok(metadata) if metadata.is_dir() => {
                            let is_empty = fs::read_dir(&path)
                                .context(format!("Failed to read directory at {}", path.display()))?
                                .next()
                                .transpose()
                                .context(format!("Failed to read directory at {}", path.display()))?
                                .is_none();
                            if is_empty {
                                fs::remove_dir(&path).context(format!(
                                    "Failed to remove directory at {}",
                                    path.display()
                                ))?;
                            }
                        }
                        Ok(_) => {
                            fs::remove_file(&path)
                                .context(format!("Failed to remove file at {}", path.display()))?;
                        }
                        Err(error) if error.kind() == NotFound => {}
                        Err(error) => {
                            return Err(error)
                                .context(format!("Failed to inspect path at {}", path.display()));
                        }
                    }
                }

                Ok(())
            }

            _ => Err(anyhow!(
                "Invalid transaction not allowed to remove from this state"
            )),
        }
    }
}
