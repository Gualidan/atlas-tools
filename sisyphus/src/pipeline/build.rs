use crate::{
    build::build::build,
    extract::extract::extract,
    fetch::fetch::fetch,
    package::package::package_gen,
    types::{
        config::load_settings, error::BuildPipelineError, recipe::Recipe,
        runtime_config::RuntimeConfig,
    },
};
use std::path::PathBuf;

pub fn run_build(
    recipe: &PathBuf,
    runtime_config: Option<RuntimeConfig>,
    output_path: Option<PathBuf>,
) -> Result<(), BuildPipelineError> {
    let package = Recipe { path: &recipe }.parse()?;
    let runtime_config = runtime_config.unwrap_or(RuntimeConfig::build(recipe)?);
    let config = load_settings()?;

    // Phase 1: Fetch
    let (fetched, _download_dir) = fetch(&package, true)?;

    // Phase 2: Extract
    let (_destination, source_root) = extract(fetched)?;

    // Phase 3: Dependency resolution / Build
    let pkgdir = build(&package, runtime_config, &source_root)?;

    // Phase 4: Package generation
    package_gen(
        recipe,
        &pkgdir.path().to_path_buf(),
        &package,
        &config.priv_key_path,
        output_path,
    )?;

    Ok(())
}
