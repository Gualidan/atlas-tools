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

/// Loads the required `config.yaml` from `SISYPHUS_CONFIG_DIR`, or from the
/// user's configuration directory under `sisyphus` when the override is unset.
/// Creates the configuration directory if needed. Propagates directory lookup,
/// directory creation, configuration reading, and deserialization errors.
pub fn load_config() -> Result<SisyphusConfig> {
    let config_dir = match std::env::var_os("SISYPHUS_CONFIG_DIR") {
        Some(path) => PathBuf::from(path),
        None => config_dir()
            .context("Failed to derive user's config directory")?
            .join("sisyphus"),
    };

    std::fs::create_dir_all(&config_dir).context("Failed to create sisyphus' config directory")?;

    let builder =
        Config::builder().add_source(File::from(config_dir.join("config.yaml")).required(true));

    let config = builder.build().context("Failed to build config")?;
    config
        .try_deserialize()
        .context("Failed to deserialize config")
}
