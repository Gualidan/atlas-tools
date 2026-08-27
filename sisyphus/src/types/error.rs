use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CliError {
    #[error("Recipe not found")]
    RecipeNotFound(#[from] io::Error),

    #[error("Failed to run build")]
    BuildFailed(#[from] BuildPipelineError),
}

#[derive(Error, Debug)]
pub enum BuildPipelineError {
    #[error("Failed to create temporary directory `sisyphus_root`")]
    RootDirError(#[from] io::Error),
    #[error("Failed to parse recipe")]
    RecipeError(#[from] RecipeError),
    #[error("Failed to fetch package")]
    FetchError(#[from] FetchError),
    #[error("Failed to extract archive")]
    ExtractError(#[from] ExtractError),
    #[error("Failed to parse config")]
    ConfigError(#[from] ConfigError),
    #[error("Failed to build resolve dependencies")]
    DependencyError(#[from] BuildError),
}

#[derive(Error, Debug)]
pub enum RecipeError {
    #[error("Failed to deserialize recipe")]
    ParseError(#[from] serde_saphyr::DeserializeError),
    #[error("Failed to read recipe file")]
    ReadError(#[from] io::Error),
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to resolve directory")]
    DirError(#[from] io::Error),

    #[error("Failed to parse config")]
    ParseError(#[from] serde_saphyr::DeserializeError),
}

#[derive(Error, Debug)]
pub enum FetchError {
    #[error("Git not found")]
    GitNotFound(#[from] which::Error),

    #[error("I/O error during git clone")]
    CloneError(#[from] std::io::Error),

    #[error("Failed to perform HTTP fetch")]
    HttpError(#[from] reqwest::Error),
}

#[derive(Error, Debug)]
pub enum ExtractError {
    #[error("I/O error during extraction")]
    OpenError(#[from] io::Error),

    #[error("Failed to extract archive")]
    ExtractError(#[from] compress_tools::Error),
}

#[derive(Error, Debug)]
pub enum BuildError {
    #[error("I/O error during build")]
    ResolveError(#[from] io::Error),
    #[error("Circular dependency detected")]
    CircularDependency(String),
    #[error("Failed to list directory contents")]
    ListError(#[from] walkdir::Error),
    #[error("Failed to resolve dependencies")]
    ResolveDependenciesError(),
}
