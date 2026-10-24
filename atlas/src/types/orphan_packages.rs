use std::path::PathBuf;

use common::types::package::Architectures;

pub struct OrphanPackage {
    pub name: String,
    pub architecture: Architectures,
    pub install_dir: PathBuf,
    pub version: String,
    pub release: u32,
}
