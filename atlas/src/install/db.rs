use std::{
    fmt::Display,
    fs::{self, File},
};

use anyhow::{Context, Result, anyhow};
use common::{
    functions::sky_verify::read_sky,
    types::{metadata::Metadata, package::PackageIdentity},
};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    types::runtime_config::RuntimeConfig, verify::parse_metadata_lines::parse_metadata_lines,
};

#[derive(PartialEq, Clone, Copy)]
pub enum InstallReason {
    Dependency,
    Explicit,
}

impl Display for InstallReason {
    /// Writes the database label `dependency` or `explicit`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallReason::Dependency => write!(f, "dependency"),
            InstallReason::Explicit => write!(f, "explicit"),
        }
    }
}

impl From<String> for InstallReason {
    fn from(value: String) -> Self {
        match value.as_str() {
            "explicit" => Self::Explicit,
            "dependency" => Self::Dependency,
            _ => panic!("Invalid string impossible to convert it to an install reason"),
        }
    }
}

/// # Errors
/// This function might error due to database errors.
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
        "CREATE TABLE IF NOT EXISTS installed_packages_update_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            architecture TEXT NOT NULL,
            version TEXT NOT NULL,
            release INTEGER NOT NULL,
            updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )
    .context("Failed to create installed_packages_update_log table")?;

    conn.execute(
        "CREATE TRIGGER IF NOT EXISTS installed_packages_after_update
         AFTER UPDATE ON installed_packages
         BEGIN
             INSERT INTO installed_packages_update_log (
                 name,
                 architecture,
                 version,
                 release,
                 updated_at
             )
             VALUES (
                 NEW.name,
                 NEW.architecture,
                 NEW.version,
                 NEW.release,
                 CURRENT_TIMESTAMP
             );
         END;",
        [],
    )
    .context("Failed to create installed_packages_after_update trigger")?;

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

/// Records a package, its files and created directories, and its dependency edges
/// in one install database transaction, creating the database and schema as needed.
///
/// Dependencies must exist for the package's architecture in both the repository
/// and install databases. Filesystem, archive, manifest, and database errors
/// propagate; an uncommitted transaction is rolled back.
///
/// # Errors
///
/// Returns an error if the database schema, package archive, manifest, or
/// repository dependency records cannot be read or updated.
#[allow(clippy::too_many_arguments)]
pub fn insert_installed_package(
    runtime_config: &RuntimeConfig,
    metadata: &Metadata,
    package: &PackageIdentity,
    install_dir: &str,
    sky_path: &str,
    install_reason: InstallReason,
    created_dirs: &[std::path::PathBuf],
) -> Result<()> {
    if !runtime_config
        .install_db
        .parent()
        .context("Failed to get path's parent")?
        .exists()
    {
        fs::create_dir(
            runtime_config
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

    write_owned_files(&transaction, sky_path, package, created_dirs)?;

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

// Both writers retain directories already owned by this identity, and claim
// previously unowned directories only when commit actually created them.
pub(crate) fn write_owned_files(
    conn: &Connection,
    sky_path: &str,
    package: &PackageIdentity,
    created_dirs: &[std::path::PathBuf],
) -> Result<()> {
    let architecture = package.architecture.to_string();
    let mut stmt =
        conn.prepare("SELECT path FROM owned_files WHERE package_name = ? AND architecture = ?")?;
    let previous = stmt
        .query_map(params![package.name, architecture], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<std::collections::HashSet<_>>>()?;
    drop(stmt);
    conn.execute(
        "DELETE FROM owned_files WHERE package_name = ? AND architecture = ?",
        params![package.name, architecture],
    )?;
    let (_, _, files, _) = read_sky(sky_path).context("Failed to read sky file")?;
    let mut paths = Vec::new();
    for line in parse_metadata_lines(&files).context("Failed to parse manifest's lines")? {
        if line.file_type != common::types::manifest::FileType::Dir
            || created_dirs.contains(&line.path)
            || previous.contains(line.path.to_str().context("Invalid path")?)
        {
            paths.push(line.path);
        }
    }
    for path in created_dirs {
        if !paths.contains(path) {
            paths.push(path.clone());
        }
    }
    for path in paths {
        conn.execute(
            "INSERT INTO owned_files (path, package_name, architecture) VALUES (?, ?, ?)",
            params![
                path.to_str().context("Invalid path")?,
                package.name,
                architecture
            ],
        )
        .context("Failed to insert owned files into install database")?;
    }
    Ok(())
}
