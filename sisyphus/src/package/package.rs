use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::{
    package::{metadata::metadata_gen, payload::payload_gen, sign::sign_artifacts, sky::sky_gen},
    types::{package::Package, runtime_config::RuntimeConfig},
};
use common::functions::dirs::data_dir;

pub fn package_gen(
    recipe: &PathBuf,
    pkgdir: &PathBuf,
    package: &Package,
    priv_key_path: &PathBuf,
    runtime_config: &RuntimeConfig,
) -> Result<PathBuf> {
    let (manifest, payload) = payload_gen(pkgdir)?;
    let metadata = metadata_gen(&recipe, package)?;
    let signature = sign_artifacts(priv_key_path, &metadata, &payload, &manifest)?;

    let output_path = PathBuf::from(
        data_dir()
            .context("Failed to derive user's data directory")?
            .join("sisyphus")
            .join("repo")
            .join(format!("{}", package.architecture)),
    );
    std::fs::create_dir_all(&output_path)
        .context("Failed to recursively create the output directory for the sky file")?;

    let output_path = sky_gen(
        &metadata,
        &payload,
        &manifest,
        &signature,
        package,
        output_path,
        runtime_config,
    )
    .context("Failed to generate sky file")?;

    Ok(output_path)
}
