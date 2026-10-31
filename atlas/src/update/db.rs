use anyhow::{Context, Result, anyhow};
use common::{
    functions::sky_verify::read_sky,
    types::{manifest::FileType, metadata::Metadata, package::PackageIdentity},
};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    install::db::ensure_install_schema, verify::parse_metadata_lines::parse_metadata_lines,
};

/// Updates the installed row matching the package's name and architecture and
/// replaces its owned files and outgoing dependency edges in one transaction.
/// The installation timestamp, reason, and incoming dependency edges are retained.
///
/// `sky_path` supplies the new non-directory manifest paths; `install_dir` is
/// recorded without moving files. Enables foreign keys on `conn`. Dependencies
/// must exist for this architecture in both databases. Archive, manifest, and
/// database errors propagate and leave the transaction uncommitted. An absent
/// installed row can succeed when no file or dependency rows are added.
///
/// # Errors
///
/// Returns an error if the database transaction, archive manifest, or repository
/// dependency validation fails.
pub fn update_installed_package(
    conn: &mut Connection,
    repo_conn: &Connection,
    metadata: &Metadata,
    package_id: &PackageIdentity,
    sky_path: &str,
    install_dir: &str,
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
    let manifest_lines =
        parse_metadata_lines(&files).context("Failed to parse manifest's lines")?;

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

/// Clears the installed package update log in a transaction.
///
/// # Errors
///
/// Returns an error if the log cannot be deleted or the transaction cannot be committed.
pub fn clear_update_log(conn: &mut Connection) -> Result<()> {
    let transaction = conn
        .transaction()
        .context("Failed to get transaction from database connection")?;

    transaction.execute("DELETE FROM installed_packages_update_log", [])?;

    transaction.commit()?;

    Ok(())
}
