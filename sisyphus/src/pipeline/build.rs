use crate::{
    build::build::build,
    extract::extract::extract,
    fetch::fetch::fetch,
    types::{error::BuildPipelineError, recipe::Recipe, runtime_config::RuntimeConfig},
};
use std::path::PathBuf;

pub fn run_build(
    recipe: &PathBuf,
    runtime_config: Option<RuntimeConfig>,
) -> Result<(), BuildPipelineError> {
    let package = Recipe { path: &recipe }.parse()?;
    let runtime_config = runtime_config.unwrap_or(RuntimeConfig::build(recipe)?);

    // Phase 1: Fetch
    let (fetched, _download_dir) = fetch(&package, true)?;

    // Phase 2: Extract
    let (destination, source_root) = extract(fetched)?;

    // Phase 3: Dependency resolution / Build
    build(&package, runtime_config)?;

    Ok(())
}
