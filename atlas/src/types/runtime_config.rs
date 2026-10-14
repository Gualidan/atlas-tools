use std::path::PathBuf;

use anyhow::{Context, Result};
use dirs::data_dir;

pub struct RuntimeConfig {
    pub repo_db: PathBuf,
    pub install_db: PathBuf,
    pub sky_repo: PathBuf,
}

impl RuntimeConfig {
    pub fn build() -> Result<Self> {
        Ok(RuntimeConfig {
            repo_db: data_dir()
                .context("Failed to derive user's data directory")?
                .join("sisyphus")
                .join("repo")
                .join("repo")
                .with_extension("db"),
            install_db: data_dir()
                .context("Failed to derive user's cache directory")?
                .join("atlas")
                .join("installed")
                .with_extension("db"),
            sky_repo: data_dir()
                .context("Failed to derive user's data directory")?
                .join("sisyphus")
                .join("repo"),
        })
    }
    pub fn build_at(path: &PathBuf) -> Result<Self> {
        Ok(RuntimeConfig {
            repo_db: path.join("repo").with_extension("db"),
            install_db: path.join("installed").with_extension("db"),
            sky_repo: path.join("repo"),
        })
    }
}
