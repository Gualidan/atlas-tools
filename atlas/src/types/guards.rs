use std::{fs::remove_dir_all, path::PathBuf};

use rustix::fs::{CWD, RenameFlags, renameat_with};

pub struct SwapBackGuard {
    install_dir: Option<PathBuf>,
    staging_dir: Option<PathBuf>,
    install_dir_existed: bool,
}

impl SwapBackGuard {
    pub fn new(install_dir: PathBuf, staging_dir: PathBuf, install_dir_existed: bool) -> Self {
        SwapBackGuard {
            install_dir: Some(install_dir),
            staging_dir: Some(staging_dir),
            install_dir_existed,
        }
    }
    pub fn disarm(&mut self) {
        self.install_dir.take();
        self.staging_dir.take();
    }
}

impl Drop for SwapBackGuard {
    fn drop(&mut self) {
        if let Some(install_dir) = &self.install_dir
            && let Some(tempdir) = &self.staging_dir
        {
            match renameat_with(CWD, install_dir, CWD, tempdir, RenameFlags::EXCHANGE) {
                Ok(_) => {}
                Err(_) => eprintln!("Swap back exchange failed"),
            }
            if !self.install_dir_existed {
                match remove_dir_all(install_dir) {
                    Ok(_) => {}
                    Err(_) => eprintln!("Attempt at placeholder removal failed"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SwapBackGuard;
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    /// Creates a parent directory holding two sibling directories, each with a
    /// marker file. The install side holds "old" and the staging side holds
    /// "new". Both live under the same parent, so they are on the same
    /// filesystem and the exchange always works.
    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let parent = tempfile::tempdir().expect("failed to create parent temp dir");
        let install_dir = parent.path().join("install");
        let staging_dir = parent.path().join("staging");

        fs::create_dir(&install_dir).expect("failed to create install dir");
        fs::create_dir(&staging_dir).expect("failed to create staging dir");
        fs::write(marker(&install_dir), b"old").expect("failed to write install marker");
        fs::write(marker(&staging_dir), b"new").expect("failed to write staging marker");

        (parent, install_dir, staging_dir)
    }

    fn marker(dir: &Path) -> PathBuf {
        dir.join("marker")
    }

    fn content(dir: &Path) -> String {
        fs::read_to_string(marker(dir)).expect("failed to read marker")
    }

    #[test]
    fn dropping_the_guard_swaps_the_two_directories_back() {
        let (_parent, install_dir, staging_dir) = setup();

        {
            let _guard = SwapBackGuard::new(install_dir.clone(), staging_dir.clone(), true);
        }

        assert_eq!(
            content(&install_dir),
            "new",
            "the install dir should now hold the staging side's content"
        );
        assert_eq!(
            content(&staging_dir),
            "old",
            "the staging dir should now hold the install side's content"
        );
    }

    #[test]
    fn disarming_the_guard_makes_dropping_it_a_no_op() {
        let (_parent, install_dir, staging_dir) = setup();

        {
            let mut guard = SwapBackGuard::new(install_dir.clone(), staging_dir.clone(), true);
            guard.disarm();
        }

        assert_eq!(
            content(&install_dir),
            "old",
            "a disarmed guard must not touch the install dir"
        );
        assert_eq!(
            content(&staging_dir),
            "new",
            "a disarmed guard must not touch the staging dir"
        );
    }

    #[test]
    fn dropping_the_guard_removes_a_freshly_created_install_dir() {
        let (_parent, install_dir, staging_dir) = setup();

        {
            let _guard = SwapBackGuard::new(install_dir.clone(), staging_dir.clone(), false);
        }

        assert!(
            !install_dir.exists(),
            "a placeholder install dir that did not exist before must be removed"
        );
        assert_eq!(
            content(&staging_dir),
            "old",
            "the staging dir should hold the install side's original content"
        );
    }
}
