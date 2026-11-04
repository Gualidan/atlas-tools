use std::{
    fs::{self},
    path::{Path, PathBuf},
};

/// Undoes the state machine's commit-exchange when the `DatabaseRecord` step
/// fails: an armed drop swaps the previous install's content back into
/// `install_dir`. `SwapBackGuard::new` captures the world *before* the
/// exchange runs, so both paths and `install_dir_existed` describe the
/// pre-install world, not what the directories look like at commit time.
pub struct SwapBackGuard<'a> {
    install_dir: Option<&'a PathBuf>,
    owned_files: Option<Vec<String>>,
}

impl<'a> SwapBackGuard<'a> {
    /// Arms rollback for directories that have been exchanged. `staging_dir`
    /// must hold the previous destination contents; `install_dir_existed`
    /// describes the destination before the exchange.
    #[must_use]
    pub fn new(install_dir: &'a PathBuf, owned_files: Vec<String>) -> Self {
        SwapBackGuard {
            install_dir: Some(install_dir),
            owned_files: Some(owned_files),
        }
    }
    pub fn disarm(&mut self) {
        // Only called after the install is fully recorded; dropping a
        // disarmed guard must touch neither directory (the commit must
        // survive).
        self.install_dir.take();
        self.owned_files.take();
    }
}

impl Drop for SwapBackGuard<'_> {
    /// Attempts to exchange the directories back while armed, then removes the
    /// destination if it did not exist before installation. Rollback failures
    /// are swallowed; disarmed guards leave both directories untouched.
    fn drop(&mut self) {
        if let Some(install_dir) = &self.install_dir
            && let Some(owned_files) = &self.owned_files
        {
            for file in owned_files {
                let path = Path::new(install_dir).join(file);
                if path.exists() && !path.is_dir() {
                    fs::remove_file(&path).expect("Failed to remove file");
                } else if path.is_dir() && path.read_dir().unwrap().next().is_none() {
                    fs::remove_dir(&path).expect("Failed to remove directory");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::symlink, path::PathBuf};

    use tempfile::TempDir;

    use super::SwapBackGuard;

    fn setup() -> (TempDir, PathBuf) {
        let parent = tempfile::tempdir().expect("failed to create parent temp dir");
        let install_dir = parent.path().join("install");
        fs::create_dir(&install_dir).expect("failed to create install dir");

        (parent, install_dir)
    }

    #[test]
    fn dropping_the_guard_removes_owned_files_and_symlinks_only() {
        let (_parent, install_dir) = setup();
        let owned_file = install_dir.join("owned-file");
        let symlink_target = install_dir.join("symlink-target");
        let owned_symlink = install_dir.join("owned-symlink");
        let unowned_file = install_dir.join("unowned-file");

        fs::write(&owned_file, b"owned").expect("failed to write owned file");
        fs::write(&symlink_target, b"target").expect("failed to write symlink target");
        symlink(&symlink_target, &owned_symlink).expect("failed to create owned symlink");
        fs::write(&unowned_file, b"unowned").expect("failed to write unowned file");
        drop(SwapBackGuard::new(
            &install_dir,
            vec!["owned-file".to_owned(), "owned-symlink".to_owned()],
        ));

        assert!(!owned_file.exists(), "owned regular file should be removed");
        assert!(!owned_symlink.exists(), "owned symlink should be removed");
        assert!(
            symlink_target.exists(),
            "the symlink target must be preserved"
        );
        assert!(unowned_file.exists(), "unowned file should be preserved");
    }

    #[test]
    fn disarming_the_guard_makes_dropping_it_a_no_op() {
        let (_parent, install_dir) = setup();
        let owned_file = install_dir.join("owned-file");
        fs::write(&owned_file, b"owned").expect("failed to write owned file");
        let mut guard = SwapBackGuard::new(&install_dir, vec!["owned-file".to_owned()]);
        guard.disarm();
        drop(guard);

        assert!(
            owned_file.exists(),
            "a disarmed guard must preserve owned files"
        );
    }

    #[test]
    fn dropping_with_no_owned_files_does_nothing() {
        let (_parent, install_dir) = setup();
        let file = install_dir.join("file");
        fs::write(&file, b"keep").expect("failed to write file");

        drop(SwapBackGuard::new(&install_dir, Vec::new()));

        assert!(
            file.exists(),
            "with no owned files, the guard should leave files untouched"
        );
        assert!(
            install_dir.exists(),
            "the guard should not remove the install dir"
        );
    }
}
