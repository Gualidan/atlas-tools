use common::types::error::{ConfigError, VerifyError};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum State {
    #[error(".sky not found")]
    NotFound,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to hash file: {0}")]
    Hash(#[from] common::types::error::ChecksumError),
    #[error("failed to serialize metadata: {0}")]
    SerializeMetadata(#[from] serde_saphyr::Error),
    #[error("invalid public key")]
    InvalidPublicKey,
    #[error("failed to verify sky file: {0}")]
    VerifySignature(#[from] VerifyError),
    #[error("verification failed")]
    VerificationFailed,
    #[error("package is not verified")]
    NotVerified,
    #[error("package is not staged")]
    NotStaged,
    #[error("package is staged")]
    Staged(String),
    #[error("package not installed: {0}")]
    NotInstalled(String),
    #[error("package is installed")]
    Installed(String),
    #[error("failed to parse version: {0}")]
    InvalidVersion(#[from] semver::Error),
    #[error("package already up to date")]
    UpToDate,
    #[error("failed to atomically rename: {0}")]
    Rename(#[from] rustix::io::Errno),
    #[error("failed to walk directory: {0}")]
    WalkDir(#[from] walkdir::Error),
    #[error("failed to query database: {0}")]
    Query(#[from] rusqlite::Error),
}

#[derive(Error, Debug)]
pub enum Cli {
    #[error("failed to load settings: {0}")]
    LoadSettings(#[from] ConfigError),
}
