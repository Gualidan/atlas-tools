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

pub fn metadata_gen(recipe: &PathBuf, package: &Package) -> Result<Metadata, PackageError> {
    let reader = File::open(recipe)?;

    let metadata = Metadata {
        format: 1,
        name: package.name.to_owned(),
        version: package.version.to_owned(),
        release: package.release.to_owned(),
        architecture: package.architecture.to_owned(),
        deps: package.deps.to_owned(),
        source: package.source.to_owned(),
        build: Build {
            recipe_sha256: hex::encode(hash(reader)?),
            built_at: Utc::now().to_rfc3339(),
            builder: format!("sisyphus {}", env!("CARGO_PKG_VERSION")),
        },
    };
    Ok(metadata)
}
