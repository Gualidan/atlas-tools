use crate::types::package::{Architectures, Source};
use serde::Serialize;

#[derive(Serialize)]
pub struct Metadata {
    pub format: u32,
    pub name: String,
    pub version: String,
    pub release: u32,
    pub architecture: Architectures,
    pub deps: Vec<String>,
    pub source: Source,
    pub build: Build,
}

#[derive(Serialize)]
pub struct Build {
    pub recipe_sha256: String,
    pub built_at: String,
    pub builder: String,
}
