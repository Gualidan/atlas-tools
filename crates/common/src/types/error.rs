use std::io;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum RecipeError {
    #[error("failed to deserialize recipe, caused by: {0}")]
    Parse(#[from] serde_saphyr::DeserializeError),
    #[error("failed to read recipe file, caused by: {0}")]
    Read(#[from] io::Error),
    #[error("failed to validate recipe, caused by: {0}")]
    Validation(String),
}

#[derive(Error, Debug)]
pub enum ChecksumError {
    #[error("failed to parse recipe, caused by: {0}")]
    Parse(#[from] RecipeError),
    #[error("failed to fetch package, caused by: {0}")]
    Fetch(#[from] FetchError),
    #[error("failed to read file, caused by: {0}")]
    Read(#[from] io::Error),
    #[error("checksum mismatch, caused by: {0}")]
    ChecksumMismatch(String),
}

#[derive(Error, Debug)]
pub enum VerifyError {
    #[error("failed to deserialize: {0}")]
    Deserialize(#[from] serde_saphyr::DeserializeError),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("failed to hash: {0}")]
    Hash(#[from] ChecksumError),
    #[error("invalid public key")]
    InvalidPublicKey,
    #[error("failed to verify signature: {0}")]
    Signature(#[from] ring::error::Unspecified),
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("I/O error during config loading, caused by: {0}")]
    IoError(#[from] io::Error),
    #[error("failed to serialize config, caused by: {0}")]
    DirError(#[from] serde_saphyr::SerializeError),
    #[error("error during config creation, caused by: {0}")]
    ParseError(#[from] config::ConfigError),
}

#[derive(Error, Debug)]
pub enum FetchError {
    #[error("Git not found, caused by: {0}")]
    GitNotFound(#[from] which::Error),

    #[error("I/O error during git clone, caused by: {0}")]
    CloneError(#[from] std::io::Error),

    #[error("failed to perform HTTP fetch, caused by: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("failed to verify checksum, caused by: {0}")]
    ChecksumError(#[from] Box<ChecksumError>),

    #[error("failed to convert to utf8, caused by: {0}")]
    Utf8Error(#[from] std::string::FromUtf8Error),

    #[error("failed to parse URL, caused by: {0}")]
    UrlError(#[from] url::ParseError),
}

impl From<ChecksumError> for FetchError {
    fn from(err: ChecksumError) -> Self {
        FetchError::ChecksumError(Box::new(err))
    }
}
