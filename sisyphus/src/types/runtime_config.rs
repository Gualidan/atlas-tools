use dirs::{cache_dir, data_dir};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::types::error::RuntimeConfigError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub recipe_repo: PathBuf,
    pub sky_repo: PathBuf,
    pub srcdir: PathBuf,
    pub db_path: PathBuf,
    pub fetch_cache: PathBuf,
    pub cache_db_path: PathBuf,
}

impl RuntimeConfig {
    pub fn build(recipe: &PathBuf) -> Result<RuntimeConfig, RuntimeConfigError> {
        let recipe = recipe.canonicalize()?;
        Ok(RuntimeConfig {
            recipe_repo: recipe
                .ancestors()
                .nth(2)
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
            db_path: data_dir()
                .ok_or(RuntimeConfigError::DirError(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Cache directory not found",
                )))?
                .join("sisyphus")
                .join("repo")
                .join("repo")
                .with_extension("db"),
            cache_db_path: cache_dir()
                .ok_or(RuntimeConfigError::DirError(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Cache directory not found",
                )))?
                .join("sisyphus")
                .join("cache")
                .with_extension("db"),
            fetch_cache: cache_dir()
                .ok_or(RuntimeConfigError::DirError(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Cache directory not found",
                )))?
                .join("sisyphus")
                .join("sources"),
        })
    }
}
