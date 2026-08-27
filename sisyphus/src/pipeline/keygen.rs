use crate::{
    signing::signing::keygen,
    types::{error::KeygenError, sisyphus_config::SisyphusConfig},
};
use std::path::PathBuf;

pub fn run_keygen(path: Option<PathBuf>) -> Result<(), KeygenError> {
    let keypair_path = if let Some(path) = path {
        keygen(&Some(path))?
    } else {
        keygen(&None)?
    };
    SisyphusConfig::new(keypair_path);

    Ok(())
}
