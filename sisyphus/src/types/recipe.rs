use crate::types::{error::RecipeError, package::Package};
use serde_saphyr::from_str;
use std::{fs, path::PathBuf};

pub struct Recipe<'a> {
    pub path: &'a PathBuf,
}

impl<'a> Recipe<'a> {
    pub fn parse(self) -> Result<Package, RecipeError> {
        let package: Package = from_str(&fs::read_to_string(&self.path)?)?;
        Ok(package)
    }
}
