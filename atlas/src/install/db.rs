use std::fs::{self, File};

use anyhow::{Context, Result, anyhow};
use common::{
    functions::sky_verify::read_sky,
    types::{manifest::FileType, metadata::Metadata, package::PackageIdentity},
};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    types::runtime_config::RuntimeConfig, verify::parse_metadata_lines::parse_metadata_lines,
};

pub enum InstallReason {
    Dependency,
    Explicit,
}

impl ToString for InstallReason {
    fn to_string(&self) -> String {
        match self {
            InstallReason::Dependency => String::from("dependency"),
            InstallReason::Explicit => String::from("explicit"),
        }
    }
}

pub fn ensure_install_schema(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS installed_packages (
        name TEXT NOT NULL,
        version TEXT NOT NULL,
        release INTEGER NOT NULL,
        architecture TEXT NOT NULL,
        sky_path TEXT NOT NULL,
        install_dir TEXT NOT NULL,
        installed_at DATETIME DEFAULT CURRENT_TIMESTAMP,
        install_reason TEXT NOT NULL,
        PRIMARY KEY (name, architecture)
    )",
        [],
    )
    .context("Failed to create installed_packages table")?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS package_dependencies (
        package_name TEXT NOT NULL,
        architecture TEXT NOT NULL,
        dependency_name TEXT NOT NULL,
        PRIMARY KEY (package_name, architecture, dependency_name),
        FOREIGN KEY (dependency_name, architecture)
                    REFERENCES installed_packages(name, architecture),
        FOREIGN KEY (package_name, architecture)
            REFERENCES installed_packages(name, architecture),
        CHECK (package_name != dependency_name)
    )",
        [],
    )
    .context("Failed to create package_dependencies table")?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS owned_files (
        path TEXT NOT NULL,
        package_name TEXT NOT NULL,
        architecture TEXT NOT NULL,
        PRIMARY KEY (path, architecture),
        FOREIGN KEY (package_name, architecture) REFERENCES installed_packages(name, architecture)
        )",
        [],
    )
    .context("Failed to create owned_files table")?;

    Ok(())
}

pub fn insert_installed_package(
    runtime_config: &RuntimeConfig,
    metadata: &Metadata,
    package: &PackageIdentity,
    install_dir: String,
    sky_path: &str,
    install_reason: InstallReason,
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

    ensure_install_schema(&transaction)?;

    transaction.execute("INSERT OR REPLACE INTO installed_packages (name, version, release, architecture, sky_path, install_dir, install_reason) VALUES (?, ?, ?, ?, ?, ?, ?)", params![&package.name, &package.version, package.release, package.architecture.to_string(), sky_path, install_dir, install_reason.to_string()]).context("Failed to insert root package into install database")?;

    // Insert root package's paths into "owned files"
    let (_, _, files, _) = read_sky(sky_path).context("Failed to read sky file")?;
    let manifest_lines = parse_metadata_lines(files).context("Failed to parse manifest's lines")?;

    for line in manifest_lines
        .iter()
        .filter(|l| !matches!(l.file_type, FileType::Dir))
    {
        transaction.execute(
            "INSERT OR REPLACE INTO owned_files (path, package_name, architecture) VALUES (?, ?, ?)",
            params![line.path.to_str().context("Failed to get the path as a string")?, package.name, package.architecture.to_string()],
        ).context("Failed to insert owned files into install database")?;
    }

    let repo_conn = Connection::open(&runtime_config.repo_db)
        .context("Failed to open connection to the repository database")?;

    for dep in &metadata.deps {
        let exists: Option<()> = repo_conn
            .query_row(
                "SELECT 1 FROM packages WHERE name = ? AND architecture = ? LIMIT 1",
                params![dep, package.architecture.to_string()],
                |_| Ok(()),
            )
            .optional()?;
        if exists.is_none() {
            return Err(anyhow!(
                "Dependency `{dep}` not found in repository for package `{}`",
                package.name
            ));
        }

        transaction.execute("INSERT OR REPLACE INTO package_dependencies (package_name, architecture, dependency_name) VALUES (?, ?, ?)", params![package.name, package.architecture.to_string(), dep]).context("Failed to insert dependency edge into install database")?;
    }

    transaction
        .commit()
        .context("Failed to commit changes to install database")?;

    Ok(())
}

pub fn remove_uninstalled_package(
    conn: &mut Connection,
    package_id: &PackageIdentity,
) -> Result<()> {
    conn.execute("PRAGMA foreign_keys = ON", [])
        .context("Failed to enable foreign key enforcement")?;

    let transaction = conn
        .transaction()
        .context("Failed to get transaction from database connection")?;

    transaction
        .execute(
            "DELETE FROM owned_files WHERE package_name = ? AND architecture = ?",
            params![package_id.name, package_id.architecture.to_string()],
        )
        .context("Failed to execute database query")?;

    transaction
        .execute(
            "DELETE FROM package_dependencies WHERE package_name = ? AND architecture = ?",
            params![package_id.name, package_id.architecture.to_string()],
        )
        .context("Failed to execute database query")?;

    transaction
        .execute(
            "DELETE FROM package_dependencies WHERE dependency_name = ? AND architecture = ?",
            params![package_id.name, package_id.architecture.to_string()],
        )
        .context("Failed to execute database query")?;

    transaction
        .execute(
            "DELETE FROM installed_packages WHERE name = ? AND architecture = ?",
            params![package_id.name, package_id.architecture.to_string()],
        )
        .context("Failed to execute database query")?;

    transaction
        .commit()
        .context("Failed to commit database transaction")?;

    Ok(())
}

pub fn update_installed_package(
    conn: &mut Connection,
    repo_conn: &Connection,
    metadata: &Metadata,
    package_id: &PackageIdentity,
    sky_path: String,
    install_dir: String,
) -> Result<()> {
    conn.execute("PRAGMA foreign_keys = ON", [])
        .context("Failed to enable foreign key enforcement")?;

    let transaction = conn
        .transaction()
        .context("Failed to start db transaction")?;
    ensure_install_schema(&transaction)?;

    transaction
        .execute(
            "DELETE FROM package_dependencies
    WHERE package_name = ? AND architecture = ?",
            params![package_id.name, package_id.architecture.to_string()],
        )
        .context("Failed to execute db query")?;

    transaction
        .execute(
            "DELETE FROM owned_files
               WHERE package_name = ? AND architecture = ?",
            params![package_id.name, package_id.architecture.to_string()],
        )
        .context("Failed to execute db query")?;

    transaction
        .execute(
            "UPDATE installed_packages
    SET version = ?, release = ?, sky_path = ?, install_dir = ?
    WHERE name = ? AND architecture = ?",
            params![
                package_id.version,
                package_id.release,
                sky_path,
                install_dir,
                package_id.name,
                package_id.architecture.to_string()
            ],
        )
        .context("Failed to execute database query")?;

    let (_, _, files, _) = read_sky(sky_path).context("Failed to read sky file")?;
    let manifest_lines = parse_metadata_lines(files).context("Failed to parse manifest's lines")?;

    for line in manifest_lines
        .iter()
        .filter(|l| !matches!(l.file_type, FileType::Dir))
    {
        transaction
            .execute(
                "INSERT INTO owned_files (path, package_name, architecture) VALUES (?, ?, ?)",
                params![
                    line.path
                        .to_str()
                        .context("Failed to get the path as a string")?,
                    package_id.name,
                    package_id.architecture.to_string()
                ],
            )
            .context("Failed to insert owned files into install database")?;
    }

    for dep in &metadata.deps {
        let exists: Option<()> = repo_conn
            .query_row(
                "SELECT 1 FROM packages WHERE name = ? AND architecture = ? LIMIT 1",
                params![dep, package_id.architecture.to_string()],
                |_| Ok(()),
            )
            .optional()?;
        if exists.is_none() {
            return Err(anyhow!(
                "Dependency `{dep}` not found in repository for package `{}`",
                package_id.name
            ));
        }

        transaction.execute("INSERT OR REPLACE INTO package_dependencies (package_name, architecture, dependency_name) VALUES (?, ?, ?)", params![package_id.name, package_id.architecture.to_string(), dep]).context("Failed to insert dependency edge into install database")?;
    }

    transaction
        .commit()
        .context("Failed to commit changes to install database")?;

    Ok(())
}
