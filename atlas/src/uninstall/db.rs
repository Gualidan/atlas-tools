use anyhow::{Context, Result};
use common::types::package::PackageIdentity;
use rusqlite::{Connection, params};

/// Removes a package's ownership, dependency, and installation records atomically.
///
/// # Errors
///
/// Returns an error if foreign keys cannot be enabled, a database operation
/// fails, or the transaction cannot be committed.
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
