use dirs::{config_dir, data_dir};
use serde::{Deserialize, Serialize};
use serde_saphyr::from_str;
use std::{fs, path::PathBuf};

use crate::types::error::ConfigError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub recipe_repo: PathBuf,
    pub sky_repo: PathBuf,
}

impl RuntimeConfig {
    pub fn build(recipe: &PathBuf) -> Result<RuntimeConfig, ConfigError> {
        Ok(RuntimeConfig {
            recipe_repo: recipe
                .parent()
                .ok_or(ConfigError::DirError(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Recipe directory not found",
                )))?
                .to_path_buf(),
            sky_repo: data_dir()
                .ok_or(ConfigError::DirError(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Data directory not found",
                )))?
                .join("sisyphus")
                .join("repo"),
        })
    }
}
