use anyhow::{Context, Result, anyhow};
use common::types::package::{Architectures, PackageIdentity};
use rusqlite::{Connection, OptionalExtension, params};

/// Recursively resolve a packages deps by querying the install database
/// checks for dependents for each package returning an error if it finds one
/// and returns a vector of all the package identities to remove
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

/// Queries a packages deps recursively failing if a dependent is found
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

    for dep in deps {
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
    }

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
