use crate::{cli::confirmation::ASSUME_YES, signing::signing::keygen};
use anyhow::{Context, Result, anyhow};
use common::functions::dirs::config_dir;
use inquire::{
    Confirm,
    InquireError::{NotTTY, OperationCanceled, OperationInterrupted},
};
use std::{fs::File, io::Write, path::PathBuf};

/// Generates a signing key pair and writes the Sisyphus configuration file.
///
/// # Errors
///
/// Returns an error if key generation or configuration file I/O fails, or if
/// confirmation cannot be obtained when required.
pub fn run_keygen(path: &Option<PathBuf>) -> Result<()> {
    let config_dir = config_dir()
        .context("Failed to derive user's config directory")?
        .join("sisyphus");

    std::fs::create_dir_all(&config_dir).context("Failed to create config directory")?;
    let config_file = config_dir.join("config.yaml");

    if config_file.exists() && ASSUME_YES.load(std::sync::atomic::Ordering::Relaxed) {
        let ans = Confirm::new(&format!(
            "A sisyphus configuration file already exists at {}. Overwrite it with a new one?",
            config_file.display()
        ))
        .with_default(true)
        .prompt();

        match ans {
            Ok(true) => {
                println!("Proceeding...");
            }
            Ok(false) | Err(OperationInterrupted | OperationCanceled) => {
                println!("Transaction aborted");
                return Ok(());
            }
            Err(NotTTY) => {
                return Err(anyhow!(
                    "Command is running outside of a TTY try passing --yes"
                ));
            }
            _ => {
                return Err(anyhow!("Failed to read confirmation"));
            }
        }
    }

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

    File::create(config_file)
        .context("Failed to create config file")?
        .write_all(yaml_input.as_bytes())
        .context("Failed to write to config file")?;

    println!("Done");
    Ok(())
}
