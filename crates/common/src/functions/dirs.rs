use std::{env, path::PathBuf};

use anyhow::{Context, Ok, Result};
use users::{get_user_by_name, os::unix::UserExt};

pub fn data_dir() -> Result<PathBuf> {
    let username = env::var("SUDO_USER").context("Failed to read SUDO_USER variable")?;
    let user = get_user_by_name(&username).context("Failed to get user")?;
    Ok(user.home_dir().join(".local").join("share"))
}

pub fn cache_dir() -> Result<PathBuf> {
    let username = env::var("SUDO_USER").context("Failed to read SUDO_USER variable")?;
    let user = get_user_by_name(&username).context("Failed to get user")?;
    Ok(user.home_dir().join(".cache"))
}

pub fn config_dir() -> Result<PathBuf> {
    let username = env::var("SUDO_USER").context("Failed to read SUDO_USER variable")?;
    let user = get_user_by_name(&username).context("Failed to get user")?;
    Ok(user.home_dir().join(".config"))
}
