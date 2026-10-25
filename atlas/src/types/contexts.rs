use std::path::PathBuf;

use rusqlite::Connection;

use common::types::package::PackageIdentity;

pub struct RemoveContext<'a> {
    pub package_id: &'a PackageIdentity<'a>,
    pub install_conn: &'a Connection,
    pub install_dir: PathBuf,
}
