use anyhow::{Context, Result};
use dirs::{cache_dir, data_dir};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub recipe_repo: PathBuf,
    pub sky_repo: PathBuf,
    pub srcdir: PathBuf,
    pub repo_db_path: PathBuf,
    pub fetch_cache: PathBuf,
    pub cache_db_path: PathBuf,
    pub install_db_path: PathBuf,
}

impl RuntimeConfig {
    pub fn build(recipe: &PathBuf) -> Result<RuntimeConfig> {
        let recipe = recipe.canonicalize()?;
        Ok(RuntimeConfig {
            recipe_repo: recipe
                .ancestors()
                .nth(2)
                .context("Failed to derive recipe directory")?
                .to_path_buf(),
            sky_repo: data_dir()
                .context("Failed to derive user's data directory")?
                .join("sisyphus")
                .join("repo"),
            srcdir: cache_dir()
                .context("Failed to derive user's cache directory")?
                .join("sisyphus"),
            repo_db_path: data_dir()
                .context("Failed to derive user's data directory")?
                .join("sisyphus")
                .join("repo")
                .join("repo")
                .with_extension("db"),
            cache_db_path: cache_dir()
                .context("Failed to derive user's cache directory")?
                .join("sisyphus")
                .join("cache")
                .with_extension("db"),
            fetch_cache: cache_dir()
                .context("Failed to derive user's cache directory")?
                .join("sisyphus")
                .join("sources"),
            install_db_path: data_dir()
                .context("Failed to derive user's data directory")?
                .join("atlas")
                .join("installed")
                .with_extension("db"),
        })
    }
}
