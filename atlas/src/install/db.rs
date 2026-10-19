use std::fs::{self, File};

use anyhow::{Context, Result};
use common::{
    functions::{parse_metadata_lines::parse_metadata_lines, sky_verify::read_sky},
    types::{metadata::Metadata, package::Package},
};
use rusqlite::{Connection, params};

use crate::types::runtime_config::RuntimeConfig;

pub fn insert_installed_package(
    runtime_config: &RuntimeConfig,
    metadata: Metadata,
    package: &Package,
    install_dir: String,
) -> Result<()> {
    if !runtime_config
        .install_db
        .parent()
        .context("Failed to get path's parent")?
        .exists()
    {
        fs::create_dir(
            &runtime_config
                .install_db
                .parent()
                .context("Failed to get path's parent")?,
        )
        .context("Failed to create directory")?;
    }
    if !runtime_config.install_db.exists() {
        File::create_new(&runtime_config.install_db)
            .context("Failed to create installed database file")?;
    }

    let mut install_db_conn = Connection::open(&runtime_config.install_db)
        .context("Failed to open connection to the install database")?;
    install_db_conn
        .execute("PRAGMA foreign_keys = ON", [])
        .context("Failed to enable foreign key enforcement")?;
    let transaction = install_db_conn
        .transaction()
        .context("Failed to start an install database transaction")?;

    transaction
        .execute(
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
        )
        .context("Failed to create installed_packages table")?;

    transaction
        .execute(
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
        )
        .context("Failed to create package_dependencies table")?;

    transaction
        .execute(
            "CREATE TABLE IF NOT EXISTS owned_files (
        path TEXT NOT NULL,
        package_name TEXT PRIMARY KEY,
        architecture TEXT NOT NULL,",
            [],
        )
        .context("Failed to create owned_files table")?;

    // First insert dependencies into the installed_packages table
    let repo_db_conn = Connection::open(&runtime_config.repo_db)
        .context("Failed to open connection to the repository database")?;
    // Insert the root package into installed_packages
    let mut stmt = repo_db_conn
        .prepare("SELECT sky_path FROM packages WHERE name = ? AND architecture = ? LIMIT 1")
        .context("Failed to prepare repository lookup query")?;
    let sky_path: String = stmt
        .query_row(
            params![package.name, package.architecture.to_string()],
            |row| row.get(0),
        )
        .context("Failed to look up package in repository database")?;
    transaction.execute("INSERT OR REPLACE INTO installed_packages (name, version, release, architecture, sky_path, install_dir) VALUES (?, ?, ?, ?, ?, ?)", params![&package.name, &package.version, package.release, package.architecture.to_string(), sky_path, install_dir]).context("Failed to insert root package into install database")?;

    // Insert root package's paths into "owned files"
    let (_, _, files, _) = read_sky(sky_path).context("Failed to read sky file")?;
    let manifest_lines = parse_metadata_lines(files).context("Failed to parse manifest's lines")?;

    for line in manifest_lines {
        transaction.execute(
            "INSERT OR REPLACE INTO owned_files (path, package_name, architecture) VALUES (?, ?, ?)",
            params![line.path.to_str().context("Failed to get the path as a string")?, package.name, package.architecture.to_string()],
        ).context("Failed to insert owned files into install database")?;
    }

    for dep in &metadata.deps {
        let mut stmt = repo_db_conn
            .prepare("SELECT name, version, release, architecture, sky_path FROM packages WHERE name = ? AND architecture = ? LIMIT 1")
            .with_context(|| format!("Failed to prepare repository lookup query for dependency `{dep}`"))?;
        let (name, version, release, architecture, sky_path): (
            String,
            String,
            u32,
            String,
            String,
        ) = stmt
            .query_row(params![dep, package.architecture.to_string()], |row| {
                let name: String = row.get(0)?;
                let version: String = row.get(1)?;
                let release: u32 = row.get(2)?;
                let architecture: String = row.get(3)?;
                let sky_path: String = row.get(4)?;
                Ok((name, version, release, architecture, sky_path))
            })
            .with_context(|| {
                format!("Failed to look up dependency `{dep}` in repository database")
            })?;

        transaction.execute("INSERT OR REPLACE INTO installed_packages (name, version, release, architecture, sky_path, install_dir) VALUES (?, ?, ?, ?, ?, ?)", params![name, version, release, architecture, sky_path, install_dir]).context("Failed to insert dependency into install database")?;

        transaction.execute("INSERT OR REPLACE INTO package_dependencies (package_name, architecture, dependency_name) VALUES (?, ?, ?)", params![package.name, package.architecture.to_string(), dep]).context("Failed to insert dependency edge into install database")?;

        transaction.execute("INSERT OR REPLACE INTO owned_files (path, package_name, architecture) VALUES (?, ?, ?) ", params![sky_path, name, package.architecture.to_string()]).context("Failed to insert owned files into install database")?;
    }

    transaction
        .commit()
        .context("Failed to commit changes to install database")?;

    Ok(())
}
