use std::path::PathBuf;

use rusqlite::Connection;

use common::types::config::SisyphusConfig;
use common::types::package::PackageIdentity;
use tempfile::TempDir;

pub struct Context<'a> {
    pub package_id: &'a PackageIdentity,
    pub sky_path: PathBuf,
    pub sky_repo: PathBuf,
    pub install_dir: PathBuf,
    pub temp_dir: Option<TempDir>,
    pub settings: &'a SisyphusConfig,
    pub repo_conn: &'a Connection,
    pub install_conn: &'a Connection,
}
