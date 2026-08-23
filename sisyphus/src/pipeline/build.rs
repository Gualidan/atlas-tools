use std::path::PathBuf;

use crate::types::{config::Config, error::BuildError, recipe::Recipe};

pub fn run_build(recipe: &PathBuf, config: &Option<PathBuf>) -> Result<(), BuildError> {
    let pkg = Recipe { path: recipe }.parse()?;

    let config = Config::from_file_or_default(config);
    Ok(())
}
