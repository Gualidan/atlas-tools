use std::path::PathBuf;

use rusqlite::Connection;

use common::types::package::Package;
use common::types::settings::Settings;
use tempfile::TempDir;

pub struct Context<'a> {
    pub package: &'a Package,
    pub sky_path: PathBuf,
    pub sky_repo: PathBuf,
    pub install_dir: PathBuf,
    pub temp_dir: TempDir,
    pub settings: &'a Settings,
    pub dependencies: Vec<String>,
    pub conn: &'a Connection,
}
