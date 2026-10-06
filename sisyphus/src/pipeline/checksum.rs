use std::path::PathBuf;

use common::types::error::{ChecksumError, RecipeError};

use crate::{
    checksum::checksum::checksum_gen,
    types::{error::RuntimeConfigError, recipe::Recipe, runtime_config::RuntimeConfig},
};

pub fn run_checksum(recipe_path: &PathBuf) -> Result<(), ChecksumError> {
    let package = Recipe { path: recipe_path }.parse()?;
    let runtime_config = RuntimeConfig::build(recipe_path).map_err(|e| match e {
        RuntimeConfigError::DirError(io_err) => ChecksumError::Read(io_err),
        RuntimeConfigError::ParseError(deserialize_err) => {
            ChecksumError::Parse(RecipeError::Parse(deserialize_err))
        }
        RuntimeConfigError::SerializeError(serialize_err) => {
            ChecksumError::Read(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Serialization error: {}", serialize_err),
            ))
        }
        RuntimeConfigError::DeriveRuntimeConfigError(checksum_err) => checksum_err,
    })?;

    checksum_gen(&package, &runtime_config)?;
    Ok(())
}
