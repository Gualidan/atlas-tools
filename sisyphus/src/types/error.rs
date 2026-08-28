use std::{io, process::ExitStatus};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CliError {
    #[error("Recipe not found: {0}")]
    RecipeNotFound(#[from] io::Error),

    #[error("Failed to run build: {0}")]
    BuildFailed(#[from] BuildPipelineError),

    #[error("Failed to generate keypair: {0}")]
    KeygenError(#[from] KeygenError),

    #[error("Failed to run checksum: {0}")]
    ChecksumGenError(#[from] ChecksumError),
}

#[derive(Error, Debug)]
pub enum BuildPipelineError {
    #[error("Failed to create temporary directory `sisyphus_root`: {0}")]
    RootDirError(#[from] io::Error),
    #[error("Failed to parse recipe: {0}")]
    RecipeError(#[from] RecipeError),
    #[error("Failed to fetch package: {0}")]
    FetchError(#[from] FetchError),
    #[error("Failed to extract archive: {0}")]
    ExtractError(#[from] ExtractError),
    #[error("Failed to parse config: {0}")]
    ConfigError(#[from] ConfigError),
    #[error("Failed to build resolve dependencies: {0}")]
    DependencyError(#[from] BuildError),
}

#[derive(Error, Debug)]
pub enum RecipeError {
    #[error("Failed to deserialize recipe: {0}")]
    ParseError(#[from] serde_saphyr::DeserializeError),
    #[error("Failed to read recipe file: {0}")]
    ReadError(#[from] io::Error),
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to resolve directory: {0}")]
    DirError(#[from] io::Error),

    #[error("Failed to parse config: {0}")]
    ParseError(#[from] serde_saphyr::DeserializeError),

    #[error("Failed to serialize config: {0}")]
    SerializeError(#[from] serde_saphyr::SerializeError),
}

#[derive(Error, Debug)]
pub enum FetchError {
    #[error("Git not found: {0}")]
    GitNotFound(#[from] which::Error),

    #[error("I/O error during git clone: {0}")]
    CloneError(#[from] std::io::Error),

    #[error("Failed to perform HTTP fetch: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("Failed to verify checksum: {0}")]
    ChecksumError(#[from] Box<ChecksumError>),
}

impl From<ChecksumError> for FetchError {
    fn from(err: ChecksumError) -> Self {
        FetchError::ChecksumError(Box::new(err))
    }
}

#[derive(Error, Debug)]
pub enum ExtractError {
    #[error("I/O error during extraction: {0}")]
    OpenError(#[from] io::Error),

    #[error("Failed to extract archive: {0}")]
    ExtractError(#[from] compress_tools::Error),
}

#[derive(Error, Debug)]
pub enum BuildError {
    #[error("I/O error during build: {0}")]
    ResolveError(#[from] io::Error),
    #[error("Circular dependency detected: {0}")]
    CircularDependency(String),
    #[error("Failed to list directory contents: {0}")]
    ListError(#[from] walkdir::Error),
    #[error("Failed to resolve dependencies")]
    ResolveDependenciesError(),
    #[error("Failed to run build: {0}")]
    RunError(#[source] Box<BuildPipelineError>),
}

impl From<BuildPipelineError> for BuildError {
    fn from(err: BuildPipelineError) -> Self {
        BuildError::RunError(Box::new(err))
    }
}

#[derive(Error, Debug)]
pub enum KeygenError {
    #[error("OpenSSL not found: {0}")]
    OpenSslNotFound(#[from] which::Error),
    #[error("I/O error during key generation: {0}")]
    IoError(#[from] io::Error),
    #[error("Failed to get information (Username, Hostname): {0}")]
    InfoError(#[from] whoami::Error),
    #[error("Failed to get system time: {0}")]
    TimeError(#[from] std::time::SystemTimeError),
    #[error("Key already exists")]
    KeyExistsError,
    #[error("Failed to generate key: {0}")]
    OpenSslError(ExitStatus),
    #[error("Failed to write config: {0}")]
    ConfigError(#[from] ConfigError),
}

#[derive(Error, Debug)]
pub enum ChecksumError {
    #[error("Failed to parse recipe: {0}")]
    ParseError(#[from] RecipeError),
    #[error("Failed to fetch package: {0}")]
    FetchError(#[from] FetchError),
    #[error("Failed to read file: {0}")]
    ReadError(#[from] io::Error),
    #[error("Checksum mismatch")]
    ChecksumMismatchError,
}
