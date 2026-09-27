use std::{fs::File, path::PathBuf};

use chrono::Utc;

use crate::{
    checksum::hash::hash,
    types::{
        error::PackageError,
        metadata::{Build, Metadata},
        package::Package,
    },
};

pub fn metadata_gen(recipe: &PathBuf, package: Package) -> Result<String, PackageError> {
    let reader = File::open(recipe)?;

    let metadata = Metadata {
        format: 1,
        name: package.name,
        version: package.version,
        release: package.release,
        architecture: package.architecture,
        deps: package.deps,
        source: package.source,
        build: Build {
            recipe_sha256: hex::encode(hash(reader)?),
            built_at: Utc::now().to_rfc3339(),
            builder: format!("sisyphus {}", env!("CARGO_PKG_VERSION")),
        },
    };
    let metadata_string = serde_saphyr::to_string(&metadata)?;
    Ok(metadata_string)
}
