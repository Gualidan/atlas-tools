use std::path::PathBuf;

use common::types::manifest::ManifestLine;
use common::types::metadata::Metadata;
use rusqlite::Connection;

use common::types::config::SisyphusConfig;
use common::types::package::Package;
use tempfile::TempDir;

pub struct Context<'a> {
    pub package: &'a Package,
    pub sky_path: PathBuf,
    pub sky_repo: PathBuf,
    pub install_dir: PathBuf,
    pub temp_dir: Option<TempDir>,
    pub settings: &'a SisyphusConfig,
    pub dependencies: Vec<String>,
    pub repo_conn: &'a Connection,
    pub install_conn: &'a Connection,
    pub metadata: Option<Metadata>,
    pub manifest_paths: Option<Vec<ManifestLine>>,
}
