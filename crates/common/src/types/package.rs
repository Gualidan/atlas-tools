use std::fmt::Display;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Installable-identity view of a package: the fields the .sky metadata
/// contract guarantees and Atlas resolves against. Recipes carry more, but
/// verification only needs to match these identity fields.
#[derive(Deserialize, Debug, Clone, Copy)]
pub struct PackageIdentity<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub release: u32,
    pub architecture: Architectures,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Source {
    pub url: String,
    pub sha256: String,
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Architectures {
    X86_64,
    ARM,
}

impl Display for Architectures {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_ref().to_string_lossy())
    }
}

impl AsRef<std::ffi::OsStr> for Architectures {
    fn as_ref(&self) -> &std::ffi::OsStr {
        match self {
            Architectures::X86_64 => "x86_64".as_ref(),
            Architectures::ARM => "arm".as_ref(),
        }
    }
}

impl TryFrom<String> for Architectures {
    type Error = anyhow::Error;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "x86_64" => Ok(Architectures::X86_64),
            "arm" => Ok(Architectures::ARM),
            _ => Err(anyhow::anyhow!(
                "Unsupported architecture only ARM and x86_64 are supported"
            )),
        }
    }
}
