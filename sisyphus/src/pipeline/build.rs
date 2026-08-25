use crate::{
    extract::extract::extract,
    fetch::fetch::fetch,
    types::{config::Config, error::BuildError, recipe::Recipe},
};
use std::path::PathBuf;

pub fn run_build(recipe: &PathBuf, config: &Option<PathBuf>) -> Result<(), BuildError> {
    let pkg = Recipe { path: recipe }.parse()?;

    let config = Config::from_file_or_default(config);

    // Phase 1: Fetch
    let fetched = fetch(pkg)?;

    // Phase 2: Extract
    let (destination, source_root) = extract(fetched)?;

    Ok(())
}
