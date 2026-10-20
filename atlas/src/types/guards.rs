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
                Err(_) => eprint!("Swap back exchange failed"),
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
