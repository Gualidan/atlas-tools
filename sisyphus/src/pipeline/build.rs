use common::types::settings::load_settings;

use crate::{
    build::{build::build, dependencies::resolve},
    extract::extract::extract,
    fetch::fetch::fetch,
    package::package::package_gen,
    types::{error::BuildPipelineError, recipe::Recipe, runtime_config::RuntimeConfig},
};
use std::path::PathBuf;

pub fn run_build(
    recipe: &PathBuf,
    runtime_config: Option<&RuntimeConfig>,
    nested: bool,
) -> Result<(), BuildPipelineError> {
    let package = Recipe { path: &recipe }.parse()?;
    let binding = RuntimeConfig::build(recipe)?;
    let config = load_settings()?;
    let runtime_config = runtime_config.unwrap_or(&binding);

    // Phase 1: Fetch
    let (fetched, _download_dir) = fetch(&package, true)?;

    // Phase 2: Extract
    let (_destination, source_root) = extract(fetched)?;

    // Phase 3: Dependency resolution / Build
    let build_plan = if !nested {
        Some(resolve(runtime_config, &package, &config)?)
    } else {
        None
    };

    let pkgdir = build(runtime_config, &package, &source_root, &build_plan)?;

    // Phase 4: Package generation
    package_gen(
        recipe,
        &pkgdir.path().to_path_buf(),
        &package,
        &config.priv_key_path,
        runtime_config,
    )?;

    Ok(())
}
