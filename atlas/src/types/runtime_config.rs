use std::path::PathBuf;

use dirs::data_dir;

use crate::types::error::RuntimeConfig as RuntimeConfigError;

pub struct RuntimeConfig {
    pub repo_db: PathBuf,
    pub install_db: PathBuf,
    pub sky_repo: PathBuf,
}

impl RuntimeConfig {
    pub fn build() -> Result<Self, RuntimeConfigError> {
        Ok(RuntimeConfig {
            repo_db: data_dir()
                .ok_or(RuntimeConfigError::Dir(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Cache directory not found",
                )))?
                .join("sisyphus")
                .join("repo")
                .join("repo")
                .with_extension("db"),
            install_db: data_dir()
                .ok_or(RuntimeConfigError::Dir(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Cache directory not found",
                )))?
                .join("atlas")
                .join("installed")
                .with_extension("db"),
            sky_repo: data_dir()
                .ok_or(RuntimeConfigError::Dir(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Data directory not found",
                )))?
                .join("sisyphus")
                .join("repo"),
        })
    }
    pub fn build_at(path: &PathBuf) -> Result<Self, RuntimeConfigError> {
        Ok(RuntimeConfig {
            repo_db: path.join("repo").with_extension("db"),
            install_db: path.join("installed").with_extension("db"),
            sky_repo: path.join("repo"),
        })
    }
}
