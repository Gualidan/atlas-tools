use crate::types::error::KeygenError;
use libc::umask;
use std::path::PathBuf;
use std::time::SystemTime;
use which::which;

pub fn keygen(keypath: Option<PathBuf>) -> Result<Vec<PathBuf>, KeygenError> {
    which("openssl")?;
    unsafe { umask(0o077) };
    let path = match keypath {
        Some(p) => std::path::PathBuf::from(&p),
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

    let result = std::process::Command::new("openssl")
        .arg("genrsa")
        .arg("-out")
        .arg(&path)
        .arg("4096")
        .status()?;

    if !result.success() {
        return Err(KeygenError::OpenSslError(result));
    }
    let pub_key = path.with_extension("key.pub");
    let result = std::process::Command::new("openssl")
        .arg("rsa")
        .arg("-in")
        .arg(&path)
        .arg("-pubout")
        .arg("-out")
        .arg(&pub_key)
        .status()?;
    if !result.success() {
        println!("Deriving the public key failed, deleting the orphan private key...");
        std::fs::remove_file(path)?;
        println!("Done");
        return Err(KeygenError::OpenSslError(result));
    }
    Ok(vec![path, pub_key])
}
