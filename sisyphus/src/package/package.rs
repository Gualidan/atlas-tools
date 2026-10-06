use std::path::PathBuf;

use dirs::data_dir;

use crate::{
    package::{metadata::metadata_gen, payload::payload_gen, sign::sign_artifacts, sky::sky_gen},
    types::{error::PackageError, runtime_config::RuntimeConfig},
};
use common::types::package::Package;

pub fn package_gen(
    recipe: &PathBuf,
    pkgdir: &PathBuf,
    package: &Package,
    priv_key_path: &PathBuf,
    runtime_config: &RuntimeConfig,
) -> Result<PathBuf, PackageError> {
    let (manifest, payload) = payload_gen(pkgdir)?;
    let metadata = metadata_gen(&recipe, package)?;
    let metadata_string = serde_saphyr::to_string(&metadata)?;
    let signature = sign_artifacts(priv_key_path, &metadata_string, &payload, &manifest)?;

    let output_path = PathBuf::from(
        data_dir()
            .ok_or(PackageError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Recipe directory not found",
            )))?
            .join("sisyphus")
            .join("repo")
            .join(format!("{}", package.architecture)),
    );
    std::fs::create_dir_all(&output_path)?;

    let output_path = sky_gen(
        &metadata_string,
        &metadata,
        &payload,
        &manifest,
        &signature,
        package,
        output_path,
        runtime_config,
    )?;

    Ok(output_path)
}
