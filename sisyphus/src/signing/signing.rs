use anyhow::{Context, Result, anyhow};
use common::functions::dirs::config_dir;
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::fs::{self, OpenOptions, Permissions};
use std::io::ErrorKind::AlreadyExists;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::time::SystemTime;

pub fn keygen(keypath: &Option<PathBuf>) -> Result<(PathBuf, PathBuf)> {
    let path = match keypath {
        Some(p) => std::path::PathBuf::from(p),
        None => {
            let keys_dir = config_dir()
                .context("Failed to derive user's config directory")?
                .join("sisyphus")
                .join("keys");
            let filename = format!(
                "{}-{}-{}.key",
                whoami::username().context("Failed to get user's username")?,
                whoami::hostname().context("Failed to get system hostname")?,
                SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .context("Failed to derive duration since Unix Epoch")?
                    .as_secs()
            );
            keys_dir.join(filename)
        }
    };

    if path.exists() {
        return Err(anyhow!("Key already exists not allowed to overwrite it"));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .context("Failed to create directory where the keys are stored")?;
    }

    let keypair = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
        .context("Failed to generate pkcs8 key pair")?;
    let pkcs8 = Ed25519KeyPair::from_pkcs8(keypair.as_ref())
        .context("Failed to generate ED255519 key pair from pkcs8 key pair")?;
    let priv_key = keypair.as_ref();
    let pub_key = pkcs8.public_key().as_ref();

    let pub_key_path = path.with_extension("pub");

    let mut pub_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pub_key_path)
        .map_err(|e| match e.kind() {
            AlreadyExists => anyhow!(
                "Key at path {} already exists, not allowed to overwrite it",
                &path.display()
            ),
            _ => anyhow!("Failed to open file at path {}", &path.display()),
        })?;
    pub_file
        .write_all(pub_key)
        .context("Failed to write public key to disk")?;

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| match e.kind() {
            AlreadyExists => anyhow!(
                "Key at path {} already exists, not allowed to overwrite it",
                &path.display()
            ),
            _ => anyhow!("Failed to open file at path {}", &path.display()),
        })?;
    file.write_all(priv_key)
        .context("Failed to write private key to disk")?;

    // Set mode for public key file to ensure it is no affected by the private key file's mode
    fs::set_permissions(&pub_key_path, Permissions::from_mode(0o644))
        .context("Failed to set mode for public key file")?;

    Ok((path, pub_key_path))
}

pub fn sign(priv_key_path: &PathBuf, data: &[u8]) -> Result<Vec<u8>> {
    let priv_key = std::fs::read(priv_key_path).context("Failed to read private key")?;
    let pkcs8 = Ed25519KeyPair::from_pkcs8(&priv_key)
        .context("Failed to generate ED25519 key pair from pkcs8 key pair")?;

    let signature = pkcs8.sign(data);

    Ok(signature.as_ref().to_vec())
}
