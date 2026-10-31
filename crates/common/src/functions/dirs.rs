use std::{env, path::PathBuf};

use anyhow::{Context, Ok, Result};
use uzers::{get_user_by_name, os::unix::UserExt};
use whoami::username;

/// Returns the data directory for the invoking user.
///
/// # Errors
///
/// Returns an error if the user cannot be determined or has no matching account.
pub fn data_dir() -> Result<PathBuf> {
    let username = if username()? == "root" {
        env::var("SUDO_USER").context("Failed to read SUDO_USER variable")?
    } else {
        username()?
    };

    let user = get_user_by_name(&username).context("Failed to get user")?;
    Ok(user.home_dir().join(".local").join("share"))
}

/// Returns the cache directory for the invoking user.
///
/// # Errors
///
/// Returns an error if the user cannot be determined or has no matching account.
pub fn cache_dir() -> Result<PathBuf> {
    let username = if username()? == "root" {
        env::var("SUDO_USER").context("Failed to read SUDO_USER variable")?
    } else {
        username()?
    };

    let user = get_user_by_name(&username).context("Failed to get user")?;
    Ok(user.home_dir().join(".cache"))
}

/// Returns the configuration directory for the invoking user.
///
/// # Errors
///
/// Returns an error if the user cannot be determined or has no matching account.
pub fn config_dir() -> Result<PathBuf> {
    let username = if username()? == "root" {
        env::var("SUDO_USER").context("Failed to read SUDO_USER variable")?
    } else {
        username()?
    };

    let user = get_user_by_name(&username).context("Failed to get user")?;
    Ok(user.home_dir().join(".config"))
}
