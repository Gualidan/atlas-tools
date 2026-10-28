use anyhow::{Context, Result, anyhow};
use common::types::package::{Architectures, PackageIdentity};
use rusqlite::{Connection, OptionalExtension, params};

use crate::install::db::InstallReason;

/// Resolves `name` to installed identities and directory paths for removal.
/// Rejects any recorded dependent of `name`, without filtering by architecture.
/// Does not remove files or change database records.
///
/// Returns an error for a missing installed row, database failures, malformed
/// dependency YAML, or an unsupported architecture. Dependency-free packages
/// yield one entry; packages with dependencies inherit [`visit`]'s recursion
/// limitation.
pub fn packages_to_uninstall<'a>(
    conn: &Connection,
    name: String,
) -> Result<Vec<(PackageIdentity, String)>> {
    let mut packages_to_uninstall: Vec<(PackageIdentity, String)> = Vec::new();
    let mut dependent_stmt = conn
        .prepare("SELECT package_name FROM package_dependencies WHERE dependency_name = ?")
        .context("Failed to prepare database query")?;

    let dependent: Option<String> = dependent_stmt
        .query_row(params![name], |row| row.get(0))
        .optional()?;

    if dependent.is_some() {
        return Err(anyhow!(
            "Can't uninstall {} because {} depends on it",
            name,
            dependent.unwrap()
        ));
    }

    visit(conn, &mut packages_to_uninstall, name)?;

    Ok(packages_to_uninstall)
}

/// Appends `name`'s installed identity and directory path to
/// `packages_to_uninstall` when its recorded dependency list is empty.
/// Lookups use only the name, without filtering by architecture. Does not
/// remove files or change database records.
///
/// Returns an error for a missing installed row, database failures, malformed
/// dependency YAML, or an unsupported architecture. With a nonempty dependency
/// list, a recorded dependent of `name` causes an error; otherwise the same
/// package is revisited without progress and can exhaust the stack.
pub fn visit(
    conn: &Connection,
    packages_to_uninstall: &mut Vec<(PackageIdentity, String)>,
    name: String,
) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT version, release, architecture, install_dir FROM installed_packages WHERE name = ?",
        )
        .context("Failed to prepare database query")?;
    let (version, release, architecture, install_dir): (String, u32, String, String) = stmt
        .query_row(params![name], |row| {
            let version: String = row.get(0)?;
            let release: u32 = row.get(1)?;
            let architecture: String = row.get(2)?;
            let install_dir: String = row.get(3)?;

            Ok((version, release, architecture, install_dir))
        })?;

    let mut deps_stmt = conn
        .prepare(
            "WITH RECURSIVE deps(package_name, dependency_name) AS (
                SELECT package_name, dependency_name
                FROM package_dependencies
                WHERE package_name = ?1

                UNION

                SELECT d.package_name, p.dependency_name
                FROM package_dependencies p
                JOIN deps d
                  ON d.dependency_name = p.package_name
                             )
            SELECT DISTINCT dependency_name
            FROM deps
            ORDER BY dependency_name",
        )
        .context("Failed to prepare database query")?;

    let deps: Vec<String> = deps_stmt
        .query_map(params![name], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut dep_info_stmt = conn.prepare(
        "SELECT version, release, architecture, install_dir, install_reason FROM installed_packages WHERE name = ?",
    )?;

    for dep in &deps {
        let (version, release, architecture, install_dir, install_reason): (
            String,
            u32,
            String,
            String,
            String,
        ) = dep_info_stmt.query_row(params![dep], |row| {
            let version: String = row.get(0)?;
            let release: u32 = row.get(1)?;
            let architecture: String = row.get(2)?;
            let install_dir: String = row.get(3)?;
            let install_reason: String = row.get(4)?;

            Ok((version, release, architecture, install_dir, install_reason))
        })?;
        if InstallReason::from(install_reason) == InstallReason::Explicit {
            println!("explicitly istalled: {dep}");
            continue;
        }
        packages_to_uninstall.push((
            PackageIdentity {
                name: dep.clone(),
                version,
                release,
                architecture: Architectures::try_from(architecture)?,
            },
            install_dir,
        ));
    }

    packages_to_uninstall.push((
        PackageIdentity {
            name,
            version,
            release,
            architecture: Architectures::try_from(architecture)?,
        },
        install_dir.clone(),
    ));

    Ok(())
}
