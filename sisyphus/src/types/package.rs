use serde::Deserialize;

use crate::types::error::RecipeError;

#[derive(Deserialize, Debug)]
pub struct Package {
    pub schema: i32,
    pub name: String,
    pub version: String,
    pub release: i32,
    pub architecture: Architectures,
    pub source: Source,
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
pub struct Source {
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
        if self.schema == 0 || self.schema > 1 {
            return Err(RecipeError::ValidationError(
                "Only schema version 1 is supported".to_string(),
            ));
        }

        if self.name.is_empty()
            || !self.name.chars().next().unwrap().is_ascii_lowercase()
                && !self.name.chars().next().unwrap().is_ascii_digit()
            || !self.name.chars().skip(1).all(|c| {
                c.is_ascii_lowercase()
                    || c.is_digit(10)
                    || c == '+'
                    || c == '.'
                    || c == '_'
                    || c == '-'
            })
        {
            return Err(RecipeError::ValidationError(
                "Name field is invalid refer to the package naming documentation".to_string(),
            ));
        }
        if self.version.is_empty()
            || self
                .version
                .chars()
                .any(|s| s.is_control() || s.is_whitespace() || s == '/')
        {
            return Err(RecipeError::ValidationError(
                "Version field is invalid: whitespaces, control characters, and slashes are not allowed"
                    .to_string(),
            ));
        }
        if self.release < 0 {
            return Err(RecipeError::ValidationError(
                "Release field must be a positive integer".to_string(),
            ));
        }
        if self.source.url.is_empty() {
            return Err(RecipeError::ValidationError(
                "URL field is empty".to_string(),
            ));
        }
        if self.source.sha256.is_empty() {
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
