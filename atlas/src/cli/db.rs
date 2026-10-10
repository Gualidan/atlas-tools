use common::types::{context::Context, metadata::Metadata, package::Package};
use rusqlite::{Connection, params};
use sisyphus::types::runtime_config::RuntimeConfig;

use crate::types::error::Cli;

pub fn insert_installed_package(
    runtime_config: &RuntimeConfig,
    metadata: Metadata,
    package: &Package,
    ctx: Context,
    install_dir: String,
) -> Result<(), Cli> {
    let mut install_db_conn = Connection::open(&runtime_config.install_db_path)?;
    install_db_conn.execute("PRAGMA foreign_keys = ON", [])?;
    let transaction = install_db_conn.transaction()?;

    transaction.execute(
        "CREATE TABLE IF NOT EXISTS installed_packages (
        name TEXT NOT NULL,
        version TEXT NOT NULL,
        release INTEGER NOT NULL,
        architecture TEXT NOT NULL,
        sky_path TEXT NOT NULL,
        install_dir TEXT NOT NULL,
        installed_at DATETIME DEFAULT CURRENT_TIMESTAMP,
        PRIMARY KEY (name, architecture)
    )",
        [],
    )?;

    transaction.execute(
        "CREATE TABLE IF NOT EXISTS package_dependencies (
        package_name TEXT NOT NULL,
        architecture TEXT NOT NULL,
        dependency_name TEXT NOT NULL,
        PRIMARY KEY (package_name, architecture, dependency_name),
        FOREIGN KEY (package_name, architecture)
            REFERENCES installed_packages(name, architecture),
        FOREIGN KEY (dependency_name, architecture)
            REFERENCES installed_packages(name, architecture),
        CHECK (package_name != dependency_name)
    )",
        [],
    )?;

    // First insert dependencies into the installed_packages table
    let repo_db_conn = Connection::open(&runtime_config.repo_db_path)?;
    // Insert the root package into installed_packages
    let mut stmt = repo_db_conn
        .prepare("SELECT sky_path FROM packages WHERE name = ? AND architecture = ? LIMIT 1")?;
    let sky_path: String = stmt.query_row(
        params![package.name, package.architecture.to_string()],
        |row| row.get(0),
    )?;
    transaction.execute("INSERT OR REPLACE INTO installed_packages (name, version, release, architecture, sky_path, install_dir) VALUES (?, ?, ?, ?, ?, ?)", params![&package.name, &package.version, package.release, package.architecture.to_string(), sky_path, install_dir])?;
    for dep in &metadata.deps {
        let mut stmt = repo_db_conn
            .prepare("SELECT name, version, release, architecture, sky_path FROM packages WHERE name = ? AND architecture = ? LIMIT 1")?;
        let (name, version, release, architecture, sky_path): (
            String,
            String,
            u32,
            String,
            String,
        ) = stmt.query_row(params![dep, package.architecture.to_string()], |row| {
            let name: String = row.get(0)?;
            let version: String = row.get(1)?;
            let release: u32 = row.get(2)?;
            let architecture: String = row.get(3)?;
            let sky_path: String = row.get(4)?;
            Ok((name, version, release, architecture, sky_path))
        })?;

        transaction.execute("INSERT OR REPLACE INTO installed_packages (name, version, release, architecture, sky_path, install_dir) VALUES (?, ?, ?, ?, ?, ?)", params![name, version, release, architecture, sky_path, install_dir])?;

        transaction.execute("INSERT OR REPLACE INTO package_dependencies (package_name, architecture, dependency_name) VALUES (?, ?, ?)", params![package.name, package.architecture.to_string(), dep])?;
    }

    transaction.commit()?;

    Ok(())
}
