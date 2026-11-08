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
    pub groups: Vec<String>,
    #[serde(default)]
    pub prepare: Option<String>,
    #[serde(default)]
    pub build: Option<String>,
    #[serde(default)]
    pub check: Option<String>,
    pub package: String,
}

fn has_only_unique_elements<T: Eq + std::hash::Hash>(vec: &[T]) -> bool {
    let mut seen = std::collections::HashSet::new();
    for item in vec {
        if !seen.insert(item) {
            return false;
        }
    }
    true
}

/// Package and group identifiers use the same v1 grammar:
/// `[a-z0-9][a-z0-9+._-]*`.
fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first_char) = chars.next() else {
        return false;
    };
    (first_char.is_ascii_lowercase() || first_char.is_ascii_digit())
        && chars.all(|c| {
            c.is_ascii_lowercase()
                || c.is_ascii_digit()
                || c == '+'
                || c == '.'
                || c == '_'
                || c == '-'
        })
}

impl Package {
    /// Validates the package schema and required recipe fields.
    ///
    /// # Errors
    ///
    /// Returns an error if the schema, package name, version, source URL, or
    /// required package phase is invalid, or if the package depends on itself.
    pub fn validate(&self) -> Result<()> {
        if self.schema == 0 || self.schema > 1 {
            return Err(anyhow!("Only schema version 1 is supported"));
        }

        if !is_valid_name(&self.name) {
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
        if self.groups.contains(&self.name) {
            return Err(anyhow!("Package is not allowed to list itself as a group"));
        }
        if !has_only_unique_elements(&self.groups) {
            return Err(anyhow!("Group list must not contain duplicate entries"));
        }
        if let Some(group) = self.groups.iter().find(|group| !is_valid_name(group)) {
            return Err(anyhow!(
                "Group name `{group}` is invalid refer to the package naming documentation"
            ));
        }
        if self.package.is_empty() {
            return Err(anyhow!("Package phase field is empty"));
        }
        Ok(())
    }
}
