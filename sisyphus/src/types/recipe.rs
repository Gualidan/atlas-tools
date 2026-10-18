use anyhow::{Context, Result};
use common::types::package::Package;
use serde_saphyr::from_str;
use std::{fs, path::PathBuf};

pub struct Recipe<'a> {
    pub path: &'a PathBuf,
}

impl<'a> Recipe<'a> {
    pub fn parse(self) -> Result<Package> {
        let contents = fs::read_to_string(&self.path).context("Failed to read recipe file")?;
        let package: Package = from_str(&contents).context("Failed to parse recipe file")?;
        package.validate().context("Failed to validate recipe")?;
        Ok(package)
    }
}
