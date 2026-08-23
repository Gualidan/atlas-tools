use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CliError {
    #[error("not found")]
    InvalidArgument(#[from] io::Error),
}

#[derive(Error, Debug)]
pub enum BuildError {}
