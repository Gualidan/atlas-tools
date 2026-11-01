use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::{
    checksum::checksum::checksum_gen,
    types::{recipe::Recipe, runtime_config::RuntimeConfig},
};

/// Generates a checksum for the source referenced by a recipe.
///
/// # Errors
///
/// Returns an error if the recipe is invalid, runtime configuration cannot be
/// built, or the source cannot be fetched and hashed.
pub fn run_checksum(recipe_path: &PathBuf) -> Result<()> {
    let package = Recipe { path: recipe_path }.parse()?;
    let runtime_config =
        RuntimeConfig::build(recipe_path).context("Failed to build runtime config")?;

    checksum_gen(&package, &runtime_config)?;
    Ok(())
}
