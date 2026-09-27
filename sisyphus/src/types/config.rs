use config::{Config, File};
use dirs::config_dir;
use serde::Deserialize;
use std::path::PathBuf;

use crate::types::error::ConfigError;

#[derive(Debug, Deserialize, Clone)]
pub struct Settings {
    pub priv_key_path: PathBuf,
    pub pub_key_path: PathBuf,
}

pub fn load_settings() -> Result<Settings, ConfigError> {
    let config_dir = config_dir()
        .ok_or(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Recipe directory not found",
        ))?
        .join("sisyphus");

    std::fs::create_dir_all(&config_dir)?;

    let builder = Config::builder().add_source(File::with_name("config").required(true));

    let config = builder.build()?;
    Ok(config.try_deserialize()?)
}
