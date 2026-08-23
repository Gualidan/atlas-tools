use dirs::{config_dir, data_dir};
use serde::{Deserialize, Serialize};
use serde_saphyr::from_str;
use std::{fs, path::PathBuf};

use crate::types::error::ConfigError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub recipe_repo: PathBuf,
    pub private_key: PathBuf,
    pub public_key: PathBuf,
}

impl Config {
    pub fn build(
        keypair: Vec<PathBuf>,
        recipe_repo: Option<PathBuf>,
    ) -> Result<Config, ConfigError> {
        Ok(Config {
            recipe_repo: recipe_repo.unwrap_or(data_dir().ok_or(ConfigError::DirError(
                std::io::Error::new(std::io::ErrorKind::NotFound, "Data directory not found"),
            ))?),
            private_key: keypair[0].clone(),
            public_key: keypair[1].clone(),
        })
    }

    pub fn from_file_or_default(config_path: &Option<PathBuf>) -> Result<Config, ConfigError> {
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
