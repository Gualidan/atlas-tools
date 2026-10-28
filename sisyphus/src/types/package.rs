use anyhow::{Result, anyhow};
use common::types::package::{Architectures, Source};
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct Package {
    pub schema: u32,
    pub name: String,
    pub version: String,
    pub release: u32,
    pub architecture: Architectures,
    pub source: Source,
    #[serde(default)]
    pub makedeps: Vec<String>,
    #[serde(default)]
    pub deps: Vec<String>,
    #[serde(default)]
    pub prepare: Option<String>,
    #[serde(default)]
    pub build: Option<String>,
    #[serde(default)]
    pub check: Option<String>,
    pub package: String,
}

impl Package {
    pub fn validate(&self) -> Result<()> {
        if self.schema == 0 || self.schema > 1 {
            return Err(anyhow!("Only schema version 1 is supported"));
        }

        if self.name.is_empty()
            || !self.name.chars().next().unwrap().is_ascii_lowercase()
                && !self.name.chars().next().unwrap().is_ascii_digit()
            || !self.name.chars().skip(1).all(|c| {
                c.is_ascii_lowercase()
                    || c.is_ascii_digit()
                    || c == '+'
                    || c == '.'
                    || c == '_'
                    || c == '-'
            })
        {
            return Err(anyhow!(
                "Name field is invalid refer to the package naming documentation"
            ));
        }
        if self.version.is_empty()
            || self
                .version
                .chars()
                .any(|s| s.is_control() || s.is_whitespace() || s == '/')
        {
            return Err(anyhow!(
                "Version field is invalid: whitespaces, control characters, and slashes are not allowed"
            ));
        }
        if self.source.url.is_empty() {
            return Err(anyhow!("URL field is empty"));
        }
        if self.deps.contains(&self.name) {
            return Err(anyhow!(
                "Package is not allowed to list itself as a dependency"
            ));
        }
        if self.makedeps.contains(&self.name) {
            return Err(anyhow!(
                "Package is not allowed to list itself as a make dependency"
            ));
        }
        if self.package.is_empty() {
            return Err(anyhow!("Package phase field is empty"));
        }
        Ok(())
    }
}
