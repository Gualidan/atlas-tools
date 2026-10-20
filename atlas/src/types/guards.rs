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

#[cfg(test)]
mod tests {
    use super::InstallGuard;
    use tempfile::tempdir;

    #[test]
    fn cleans_up_on_drop_without_disarm() {
        let tmp = tempdir().unwrap();
        let install_dir = tmp.path().join("install_dir");
        std::fs::create_dir(&install_dir).unwrap();

        {
            let _guard = InstallGuard::new(install_dir.clone());
        }

        assert!(!install_dir.exists());
    }

    #[test]
    fn does_nothing_on_drop_after_disarm() {
        let tmp = tempdir().unwrap();
        let install_dir = tmp.path().join("install_dir");
        std::fs::create_dir(&install_dir).unwrap();

        {
            let mut guard = InstallGuard::new(install_dir.clone());
            guard.disarm();
        }

        assert!(install_dir.exists());
    }
}
