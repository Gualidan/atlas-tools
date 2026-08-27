use std::path::PathBuf;

use crate::{
    checksum::checksum::checksum_gen,
    types::{error::ChecksumError, recipe::Recipe},
};

pub fn run_checksum(recipe_path: &PathBuf) -> Result<(), ChecksumError> {
    let package = Recipe { path: recipe_path }.parse()?;
    checksum_gen(&package)?;
    Ok(())
}
