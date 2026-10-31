use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use common::functions::dirs::data_dir;

pub struct RuntimeConfig {
    pub repo_db: PathBuf,
    pub install_db: PathBuf,
    pub sky_repo: PathBuf,
}

impl RuntimeConfig {
    /// Builds runtime paths from the invoking user's data directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the user's data directory cannot be determined.
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
    /// Derives `repo.db`, `installed.db`, and the `repo` directory beneath `path`
    /// without accessing the filesystem.
    ///
    /// # Errors
    ///
    /// This method currently always succeeds; the `Result` return type is kept
    /// for compatibility with `build` and existing callers.
    pub fn build_at(path: &Path) -> Result<Self> {
        Ok(RuntimeConfig {
            repo_db: path.join("repo").with_extension("db"),
            install_db: path.join("installed").with_extension("db"),
            sky_repo: path.join("repo"),
        })
    }
}
