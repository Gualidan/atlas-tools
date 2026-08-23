use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CliError {
    #[error("not found")]
    InvalidArgument(#[from] io::Error),
}

#[derive(Error, Debug)]
pub enum BuildError {
    #[error("Failed to create temporary directory `sisyphus_root`")]
    RootDirError(#[from] io::Error),
}

#[derive(Error, Debug)]
pub enum RecipeError {
    #[error("Failed to deserialize recipe")]
    ParseError(#[from] serde_saphyr::DeserializeError),
    #[error("Failed to read recipe file")]
    ReadError(#[from] io::Error),
}
