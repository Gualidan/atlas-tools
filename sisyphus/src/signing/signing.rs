use crate::types::error::{KeygenError, SigningError};
use libc::umask;
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::path::PathBuf;
use std::time::SystemTime;

pub fn keygen(keypath: &Option<PathBuf>) -> Result<(PathBuf, PathBuf), KeygenError> {
    let path = match keypath {
        Some(p) => std::path::PathBuf::from(p),
        None => {
            let keys_dir = dirs::config_dir()
                .ok_or(KeygenError::IoError(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Recipe directory not found",
                )))?
                .join("sisyphus")
                .join("keys");
            let filename = format!(
                "{}-{}-{}.key",
                whoami::username()?,
                whoami::hostname()?,
                SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs()
            );
            keys_dir.join(filename)
        }
    };

    if path.exists() {
        return Err(KeygenError::KeyExistsError);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let keypair = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())?;
    let pkcs8 = Ed25519KeyPair::from_pkcs8(keypair.as_ref())?;
    let priv_key = keypair.as_ref();
    let pub_key = pkcs8.public_key().as_ref();

    let pub_key_path = path.with_extension("pub");

    std::fs::write(&pub_key_path, pub_key)?;

    unsafe { umask(0o077) };
    std::fs::write(&path, priv_key)?;

    Ok((path, pub_key_path))
}

pub fn sign(priv_key_path: &PathBuf, data: &[u8]) -> Result<Vec<u8>, SigningError> {
    let priv_key = std::fs::read(priv_key_path)?;
    let pkcs8 = Ed25519KeyPair::from_pkcs8(&priv_key)?;

    let signature = pkcs8.sign(data);

    Ok(signature.as_ref().to_vec())
}
