use crate::signing::signing::keygen;
use anyhow::{Context, Result};
use dirs::config_dir;
use std::{fs::File, io::Write, path::PathBuf};

pub fn run_keygen(path: &Option<PathBuf>) -> Result<()> {
    let config_dir = config_dir()
        .context("Failed to derive user's config directory")?
        .join("sisyphus");

    let keypair_path = if let Some(path) = path {
        keygen(&Some(path.clone())).context("Failed to generate key pair")?
    } else {
        keygen(&None).context("Failed to generate key pair")?
    };

    let yaml_input = format!(
        "priv_key_path: {}\npub_key_path: {}\n",
        keypair_path.0.display(),
        keypair_path.1.display(),
    );

    std::fs::create_dir_all(&config_dir).context("Failed to create config directory")?;

    File::create(config_dir.join("config.yaml"))
        .context("Failed to create config file")?
        .write_all(yaml_input.as_bytes())
        .context("Failed to write to config file")?;

    Ok(())
}
