use std::{fs::remove_dir_all, path::PathBuf};

use rustix::fs::{CWD, RenameFlags, renameat_with};
use tempfile::TempDir;

/// Undoes the state machine's commit-exchange when the `DatabaseRecord` step
/// fails: an armed drop swaps the previous install's content back into
/// `install_dir`. `SwapBackGuard::new` captures the world *before* the
/// exchange runs, so both paths and `install_dir_existed` describe the
/// pre-install world, not what the directories look like at commit time.
pub struct SwapBackGuard<'a> {
    install_dir: Option<&'a PathBuf>,
    staging_dir: Option<&'a TempDir>,
    /// Whether `install_dir` existed before the exchange took place. A
    /// directory that never existed is a placeholder; one that did holds the
    /// previous install's content after the swap-back and must never be
    /// deleted.
    install_dir_existed: bool,
}

impl<'a> SwapBackGuard<'a> {
    /// Arms rollback for directories that have been exchanged. `staging_dir`
    /// must hold the previous destination contents; `install_dir_existed`
    /// describes the destination before the exchange.
    #[must_use]
    pub fn new(
        install_dir: &'a PathBuf,
        staging_dir: &'a TempDir,
        install_dir_existed: bool,
    ) -> Self {
        SwapBackGuard {
            install_dir: Some(install_dir),
            staging_dir: Some(staging_dir),
            install_dir_existed,
        }
    }
    pub fn disarm(&mut self) {
        // Only called after the install is fully recorded; dropping a
        // disarmed guard must touch neither directory (the commit must
        // survive).
        self.install_dir.take();
        self.staging_dir.take();
    }
}

impl Drop for SwapBackGuard<'_> {
    /// Attempts to exchange the directories back while armed, then removes the
    /// destination if it did not exist before installation. Rollback failures
    /// are swallowed; disarmed guards leave both directories untouched.
    fn drop(&mut self) {
        if let Some(install_dir) = &self.install_dir
            && let Some(tempdir) = &self.staging_dir
        {
            // Installs and staging both live under the same parent, so the
            // exchange is same-filesystem and must not fail in practice.
            match renameat_with(
                CWD,
                install_dir.as_path(),
                CWD,
                tempdir.path(),
                RenameFlags::EXCHANGE,
            ) {
                Ok(()) => {}
                Err(_) => eprintln!("Swap back exchange failed"),
            }
            // The exchange put the previous install's content back into
            // `install_dir`. Deleting that unconditionally is what destroys
            // reinstalled packages' data: the removal is only correct when
            // this install created the directory and thus nothing of the
            // user's lives behind this path.
            if !self.install_dir_existed {
                match remove_dir_all(install_dir) {
                    Ok(()) => {}
                    Err(_) => eprintln!("Attempt at placeholder removal failed"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::{Builder, TempDir};

    use super::SwapBackGuard;
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    /// Creates a parent directory holding two sibling directories, each with a
    /// marker file. The install side holds "old" and the staging side holds
    /// "new". Both live under the same parent, so they are on the same
    /// filesystem and the exchange always works.
    fn setup() -> (tempfile::TempDir, PathBuf, TempDir) {
        let parent = tempfile::tempdir().expect("failed to create parent temp dir");
        let install_dir = parent.path().join("install");

        fs::create_dir(&install_dir).expect("failed to create install dir");
        let staging_tempdir = Builder::new()
            .tempdir_in(parent.path())
            .expect("failed to create tempdir");
        fs::write(marker(&install_dir), b"old").expect("failed to write install marker");
        fs::write(marker(staging_tempdir.path()), b"new").expect("failed to write staging marker");

        (parent, install_dir, staging_tempdir)
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
            let _guard = SwapBackGuard::new(&install_dir, &staging_dir, true);
        }

        assert_eq!(
            content(&install_dir),
            "new",
            "the install dir should now hold the staging side's content"
        );
        assert_eq!(
            content(staging_dir.path()),
            "old",
            "the staging dir should now hold the install side's content"
        );
    }

    #[test]
    fn disarming_the_guard_makes_dropping_it_a_no_op() {
        let (_parent, install_dir, staging_dir) = setup();

        {
            let mut guard = SwapBackGuard::new(&install_dir, &staging_dir, true);
            guard.disarm();
        }

        assert_eq!(
            content(&install_dir),
            "old",
            "a disarmed guard must not touch the install dir"
        );
        assert_eq!(
            content(staging_dir.path()),
            "new",
            "a disarmed guard must not touch the staging dir"
        );
    }

    #[test]
    fn dropping_the_guard_removes_a_freshly_created_install_dir() {
        let (_parent, install_dir, staging_dir) = setup();

        {
            let _guard = SwapBackGuard::new(&install_dir, &staging_dir, false);
        }

        assert!(
            !install_dir.exists(),
            "a placeholder install dir that did not exist before must be removed"
        );
        assert_eq!(
            content(staging_dir.path()),
            "old",
            "the staging dir should hold the install side's original content"
        );
    }
}
