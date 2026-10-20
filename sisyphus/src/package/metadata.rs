use std::{fs::File, path::PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;

use common::{
    functions::hash::hash,
    types::metadata::{Build, Metadata},
};

use crate::types::package::Package;

pub fn metadata_gen(recipe: &PathBuf, package: &Package) -> Result<Metadata> {
    let reader = File::open(recipe).context("Failed to open recipe file")?;

    let metadata = Metadata {
        format: 1,
        name: package.name.to_owned(),
        version: package.version.to_owned(),
        release: package.release.to_owned(),
        architecture: package.architecture.to_owned(),
        deps: package.deps.to_owned(),
        source: package.source.to_owned(),
        build: Build {
            recipe_sha256: hex::encode(hash(reader).context("Failed to hash recipe file")?),
            built_at: Utc::now().to_rfc3339(),
            builder: format!("sisyphus {}", env!("CARGO_PKG_VERSION")),
        },
    };
    Ok(metadata)
}
