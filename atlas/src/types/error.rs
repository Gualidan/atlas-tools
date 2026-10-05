use common::types::error::VerifyError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum State {
    #[error(".sky not found")]
    NotFound,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to hash file: {0}")]
    Hash(#[from] common::types::error::ChecksumError),
    #[error("failed to parse public key: {0}")]
    ParsePublicKey(#[from] ring::error::Unspecified),
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
    #[error("package not installed: {0}")]
    NotInstalled(String),
}
