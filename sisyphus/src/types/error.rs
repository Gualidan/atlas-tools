use std::{io, process::ExitStatus};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CliError {
    #[error("recipe not found: {0}")]
    RecipeNotFound(#[from] io::Error),

    #[error("failed to run build, caused by: {0}")]
    BuildFailed(#[from] BuildPipelineError),

    #[error("failed to generate keypair, caused by: {0}")]
    KeygenError(#[from] KeygenError),

    #[error("failed to run checksum, caused by: {0}")]
    ChecksumGenError(#[from] ChecksumError),
}

#[derive(Error, Debug)]
pub enum BuildPipelineError {
    #[error("failed to create temporary directory `sisyphus_root`, caused by: {0}")]
    RootDirError(#[from] io::Error),
    #[error("failed to parse recipe, caused by: {0}")]
    RecipeError(#[from] RecipeError),
    #[error("failed to fetch package, caused by: {0}")]
    FetchError(#[from] FetchError),
    #[error("failed to extract archive, caused by: {0}")]
    ExtractError(#[from] ExtractError),
    #[error("failed to parse config, caused by: {0}")]
    ConfigError(#[from] ConfigError),
    #[error("failed to build resolve dependencies, caused by: {0}")]
    DependencyError(#[from] BuildError),
}

#[derive(Error, Debug)]
pub enum RecipeError {
    #[error("failed to deserialize recipe, caused by: {0}")]
    ParseError(#[from] serde_saphyr::DeserializeError),
    #[error("failed to read recipe file, caused by: {0}")]
    ReadError(#[from] io::Error),
    #[error("failed to validate recipe, caused by: {0}")]
    ValidationError(String),
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("failed to resolve directory, caused by: {0}")]
    DirError(#[from] io::Error),

    #[error("failed to parse config, caused by: {0}")]
    ParseError(#[from] serde_saphyr::DeserializeError),

    #[error("failed to serialize config, caused by: {0}")]
    SerializeError(#[from] serde_saphyr::SerializeError),
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
}

impl From<ChecksumError> for FetchError {
    fn from(err: ChecksumError) -> Self {
        FetchError::ChecksumError(Box::new(err))
    }
}

#[derive(Error, Debug)]
pub enum ExtractError {
    #[error("I/O error during extraction, caused by: {0}")]
    OpenError(#[from] io::Error),

    #[error("failed to extract archive, caused by: {0}")]
    ExtractError(#[from] compress_tools::Error),
}

#[derive(Error, Debug)]
pub enum BuildError {
    #[error("I/O error during build, caused by: {0}")]
    ResolveError(#[from] io::Error),
    #[error("Circular dependency detected, caused by: {0}")]
    CircularDependency(String),
    #[error("Failed to list directory contents, caused by: {0}")]
    ListError(#[from] walkdir::Error),
    #[error("Failed to resolve dependencies, caused by: {0}")]
    ResolveDependenciesError(String),
    #[error("failed to run build, caused by: {0}")]
    RunError(#[source] Box<BuildPipelineError>),
}

impl From<BuildPipelineError> for BuildError {
    fn from(err: BuildPipelineError) -> Self {
        BuildError::RunError(Box::new(err))
    }
}

#[derive(Error, Debug)]
pub enum KeygenError {
    #[error("OpenSSL not found, caused by: {0}")]
    OpenSslNotFound(#[from] which::Error),
    #[error("I/O error during key generation, caused by: {0}")]
    IoError(#[from] io::Error),
    #[error("failed to get information (Username, Hostname): {0}")]
    InfoError(#[from] whoami::Error),
    #[error("failed to get system time: {0}")]
    TimeError(#[from] std::time::SystemTimeError),
    #[error("key already exists")]
    KeyExistsError,
    #[error("failed to generate key: {0}")]
    OpenSslError(ExitStatus),
    #[error("failed to write config: {0}")]
    ConfigError(#[from] ConfigError),
}

#[derive(Error, Debug)]
pub enum ChecksumError {
    #[error("failed to parse recipe, caused by: {0}")]
    ParseError(#[from] RecipeError),
    #[error("failed to fetch package, caused by: {0}")]
    FetchError(#[from] FetchError),
    #[error("failed to read file, caused by: {0}")]
    ReadError(#[from] io::Error),
    #[error("checksum mismatch, caused by: {0}")]
    ChecksumMismatchError(String),
}

#[derive(Error, Debug)]
pub enum SandboxError {}
