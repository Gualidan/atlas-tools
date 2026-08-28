use serde::Deserialize;

use crate::types::error::RecipeError;

#[derive(Deserialize, Debug)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub architecture: Architectures,
    pub sources: Sources,
    #[serde(default)]
    pub makedeps: Vec<String>,
    #[serde(default)]
    pub deps: Vec<String>,
    #[serde(default)]
    pub prepare: Option<String>,
    pub build: String,
    #[serde(default)]
    pub check: Option<String>,
    pub package: String,
}

#[derive(Deserialize, Debug)]
pub struct Sources {
    pub url: String,
    pub sha256: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Architectures {
    X86_64,
    AARCH64,
}

impl Package {
    pub fn validate(&self) -> Result<(), RecipeError> {
        if self.name.is_empty() {
            return Err(RecipeError::ValidationError(
                "Name field is empty".to_string(),
            ));
        }
        if self.version.is_empty() {
            return Err(RecipeError::ValidationError(
                "Version field is empty".to_string(),
            ));
        }
        if self.sources.url.is_empty() {
            return Err(RecipeError::ValidationError(
                "URL field is empty".to_string(),
            ));
        }
        if self.sources.sha256.is_empty() {
            return Err(RecipeError::ValidationError(
                "Checksum field is empty".to_string(),
            ));
        }
        if self.build.is_empty() {
            return Err(RecipeError::ValidationError(
                "Build phase field is empty".to_string(),
            ));
        }
        if self.package.is_empty() {
            return Err(RecipeError::ValidationError(
                "Package phase field is empty".to_string(),
            ));
        }
        Ok(())
    }
}
