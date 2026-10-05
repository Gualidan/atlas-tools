use std::{fs::File, path::PathBuf};

use chrono::Utc;

use crate::types::error::PackageError;
use common::{
    functions::hash::hash,
    types::{
        metadata::{Build, Metadata},
        package::Package,
    },
};

pub fn metadata_gen(recipe: &PathBuf, package: &Package) -> Result<String, PackageError> {
    let reader = File::open(recipe)?;

    let metadata = Metadata {
        format: 1,
        name: package.name.clone(),
        version: package.version.clone(),
        release: package.release.clone(),
        architecture: package.architecture.clone(),
        deps: package.deps.clone(),
        source: package.source.clone(),
        build: Build {
            recipe_sha256: hex::encode(hash(reader)?),
            built_at: Utc::now().to_rfc3339(),
            builder: format!("sisyphus {}", env!("CARGO_PKG_VERSION")),
        },
    };
    let metadata_string = serde_saphyr::to_string(&metadata)?;
    Ok(metadata_string)
}
