use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::{
    checksum::checksum::checksum_gen,
    types::{recipe::Recipe, runtime_config::RuntimeConfig},
};

pub fn run_checksum(recipe_path: &PathBuf) -> Result<()> {
    let package = Recipe { path: recipe_path }.parse()?;
    let runtime_config =
        RuntimeConfig::build(recipe_path).context("Failed to build runtime config")?;

    checksum_gen(&package, &runtime_config)?;
    Ok(())
}
