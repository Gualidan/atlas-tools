use std::{fs, io::Write, path::PathBuf};

use dirs::config_dir;

use crate::types::error::ConfigError;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SisyphusConfig {
    pub public_key: PathBuf,
    pub private_key: PathBuf,
}

impl SisyphusConfig {
    pub fn new(keypair: Vec<PathBuf>) -> Result<Self, ConfigError> {
        let config = SisyphusConfig {
            public_key: keypair[1].clone(),
            private_key: keypair[0].clone(),
        };

        let config_path = config_dir()
            .ok_or(ConfigError::DirError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Config directory not found",
            )))?
            .join("sisyphus")
            .join("sisyphus")
            .with_extension("yaml");

        if !config_path.exists() {
            let parent = config_path
                .parent()
                .ok_or(ConfigError::DirError(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Config directory not found",
                )))?;
            fs::create_dir_all(parent)?;
            fs::File::create(&config_path)?
                .write_all(serde_saphyr::to_string(&config)?.as_bytes())?;
        }

        Ok(config)
    }
}
