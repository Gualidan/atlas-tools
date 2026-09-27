use dirs::{cache_dir, data_dir};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::types::error::RuntimeConfigError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub recipe_repo: PathBuf,
    pub sky_repo: PathBuf,
    pub srcdir: PathBuf,
}

impl RuntimeConfig {
    pub fn build(recipe: &PathBuf) -> Result<RuntimeConfig, RuntimeConfigError> {
        Ok(RuntimeConfig {
            recipe_repo: recipe
                .parent()
                .ok_or(RuntimeConfigError::DirError(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Recipe directory not found",
                )))?
                .parent()
                .ok_or(RuntimeConfigError::DirError(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Recipe directory not found",
                )))?
                .to_path_buf(),
            sky_repo: data_dir()
                .ok_or(RuntimeConfigError::DirError(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Data directory not found",
                )))?
                .join("sisyphus")
                .join("repo"),
            srcdir: cache_dir()
                .ok_or(RuntimeConfigError::DirError(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Cache directory not found",
                )))?
                .join("sisyphus"),
        })
    }
}
