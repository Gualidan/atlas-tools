use std::path::PathBuf;

use crate::types::package::Package;
use crate::types::settings::Settings;

pub struct Context<'a> {
    pub package: &'a Package,
    pub sky_path: PathBuf,
    pub install_dir: PathBuf,
    pub temp_dir: PathBuf,
    pub settings: &'a Settings,
    pub dependencies: Vec<String>,
}
