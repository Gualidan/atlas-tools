use anyhow::{Context, Result, anyhow};
use common::types::package::{Architectures, PackageIdentity};
use rusqlite::{Connection, OptionalExtension, params};

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
            "SELECT version, release, architecture, deps, install_dir FROM installed_packages WHERE name = ?",
        )
        .context("Failed to prepare database query")?;
    let (version, release, architecture, deps, install_dir): (String, u32, String, String, String) =
        stmt.query_row(params![name], |row| {
            let version: String = row.get(0)?;
            let release: u32 = row.get(1)?;
            let architecture: String = row.get(2)?;
            let deps: String = row.get(3)?;
            let install_dir: String = row.get(4)?;

            Ok((version, release, architecture, deps, install_dir))
        })?;

    let deps: Vec<String> = serde_saphyr::from_str(&deps).context("Failed to deserialize YAMl")?;

    let mut dependent_stmt = conn
        .prepare("SELECT package_name FROM package_dependencies WHERE dependency_name = ?")
        .context("Failed to prepare database query")?;

    /*for dep in deps {
        let dependent: Option<String> = dependent_stmt
            .query_row(params![name], |row| row.get(0))
            .optional()?;

        if dependent.is_none() {
            visit(conn, packages_to_uninstall, name)?;
        }

        return Err(anyhow!(
            "Can't uninstall {} because {} depends on it",
            dep,
            dependent.unwrap()
        ));
    }*/

    packages_to_uninstall.push((
        PackageIdentity {
            name: name,
            version: version,
            release,
            architecture: Architectures::try_from(architecture)?,
        },
        install_dir,
    ));

    Ok(())
}
