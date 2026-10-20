use std::{fs::remove_dir_all, path::PathBuf};

pub struct InstallGuard {
    install_dir: Option<PathBuf>,
}

impl InstallGuard {
    pub fn new(path: PathBuf) -> Self {
        InstallGuard {
            install_dir: Some(path),
        }
    }
    pub fn disarm(&mut self) {
        self.install_dir.take();
    }
}

impl Drop for InstallGuard {
    fn drop(&mut self) {
        if let Some(path) = &self.install_dir {
            let _ = remove_dir_all(path);
        }
    }
}
