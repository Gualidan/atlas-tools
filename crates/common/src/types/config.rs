use anyhow::{Context, Result};
use config::{Config, File};
use serde::Deserialize;
use std::path::PathBuf;

use crate::functions::dirs::config_dir;

#[derive(Debug, Deserialize, Clone)]
pub struct SisyphusConfig {
    pub priv_key_path: PathBuf,
    pub pub_key_path: PathBuf,
}

pub fn load_config() -> Result<SisyphusConfig> {
    let config_dir = config_dir()
        .context("Failed to derive user's config directory")?
        .join("sisyphus");

    std::fs::create_dir_all(&config_dir).context("Failed to create sisyphus' config directory")?;

    let builder =
        Config::builder().add_source(File::from(config_dir.join("config.yaml")).required(true));

    let config = builder.build().context("Failed to build config")?;
    Ok(config
        .try_deserialize()
        .context("Failed to deserialize config")?)
}
