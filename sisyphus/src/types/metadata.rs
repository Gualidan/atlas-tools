use crate::types::package::{Architectures, Source};
use serde::Serialize;

#[derive(Serialize)]
pub struct Metadata {
    format: String,
    name: String,
    version: String,
    release: u32,
    architecture: Architectures,
    deps: Vec<String>,
    sources: Vec<Source>,
    build: Build,
}

#[derive(Serialize)]
struct Build {
    recipe_sha256: String,
    built_at: String,
    builder: String,
}
