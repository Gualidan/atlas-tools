use crate::{
    fetch::fetch::fetch,
    types::{config::Config, error::BuildError, recipe::Recipe},
};
use std::path::PathBuf;

pub fn run_build(recipe: &PathBuf, config: &Option<PathBuf>) -> Result<(), BuildError> {
    let pkg = Recipe { path: recipe }.parse()?;

    let config = Config::from_file_or_default(config);

    fetch(pkg)?;
    Ok(())
}
