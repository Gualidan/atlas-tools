use std::{fs::remove_dir_all, path::PathBuf};

pub struct InstallGuard {
    pub instal_dir: Option<PathBuf>,
}

impl InstallGuard {
    pub fn new(path: PathBuf) -> Self {
        InstallGuard {
            instal_dir: Some(path),
        }
    }
    pub fn disarm(&mut self) {
        self.instal_dir.take();
    }
}

impl Drop for InstallGuard {
    fn drop(&mut self) {
        if let Some(path) = self.instal_dir.clone() {
            let _ = remove_dir_all(path);
        }
    }
}
