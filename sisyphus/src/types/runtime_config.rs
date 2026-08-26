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
    pub fn build(
        recipe_repo: PathBuf,
        sky_repo: Option<PathBuf>,
    ) -> Result<RuntimeConfig, ConfigError> {
        Ok(RuntimeConfig {
            recipe_repo: recipe_repo,
            sky_repo: sky_repo.unwrap_or(data_dir().ok_or(ConfigError::DirError(
                std::io::Error::new(std::io::ErrorKind::NotFound, "Data directory not found"),
            ))?),
        })
    }

    pub fn from_file(config_path: &Option<PathBuf>) -> Result<RuntimeConfig, ConfigError> {
        Ok(from_str(&fs::read_to_string(
            &config_path
                .as_ref()
                .unwrap_or(
                    &config_dir().ok_or(ConfigError::DirError(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "Config directory not found",
                    )))?,
                )
                .join("sisyphus")
                .join("sisyphus")
                .with_extension("yaml"),
        )?)?)
    }
}
