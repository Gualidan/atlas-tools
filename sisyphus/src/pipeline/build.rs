use crate::{
    extract::extract::extract,
    fetch::fetch::fetch,
    types::{error::BuildPipelineError, recipe::Recipe, runtime_config::RuntimeConfig},
};
use std::path::PathBuf;

pub fn run_build(recipe: &PathBuf, config: &Option<PathBuf>) -> Result<(), BuildPipelineError> {
    let pkg = Recipe { path: recipe }.parse()?;

    let config = RuntimeConfig::from_file(config)?;

    // Phase 1: Fetch
    let fetched = fetch(pkg)?;

    // Phase 2: Extract
    let (destination, source_root) = extract(fetched)?;

    // Phase 3: Build / Dependency resolution

    Ok(())
}
