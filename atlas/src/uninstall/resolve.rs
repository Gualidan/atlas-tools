use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result, anyhow};
use common::types::package::{Architectures, PackageIdentity};
use rusqlite::{Connection, OptionalExtension, params};

use crate::install::db::InstallReason;

/// Tracks packages on the current depth-first path and packages already
/// resolved, so shared dependencies are visited once and cycles are rejected.
#[derive(Clone, Copy, PartialEq)]
enum VisitStatus {
    Visiting,
    Visited,
}

type InstalledRow = (String, u32, String, String, String);

/// Resolves `name` to installed identities and directory paths for removal.
///
/// The plan contains the requested package plus every non-explicit dependency
/// reachable through `package_dependencies`. Explicit dependencies stay
/// installed and stop the traversal. After the set is built, any recorded
/// dependent outside the set blocks the plan, without filtering by
/// architecture. Does not remove files or change database records.
///
/// Returns an error for a missing installed row, database failures, an
/// unsupported architecture, a dependency cycle, a dependency with no
/// installed row, or an external dependent.
///
/// # Errors
/// This function might error because of a failure during the resolution
pub fn packages_to_uninstall(
    conn: &Connection,
    name: String,
) -> Result<Vec<(PackageIdentity, String)>> {
    let mut packages_to_uninstall: Vec<(PackageIdentity, String)> = Vec::new();
    let mut statuses: HashMap<String, VisitStatus> = HashMap::new();
    let mut stack: Vec<String> = Vec::new();

    visit(
        conn,
        &mut statuses,
        &mut stack,
        &mut packages_to_uninstall,
        name,
        true,
    )?;

    reject_external_dependents(conn, &packages_to_uninstall)?;

    Ok(packages_to_uninstall)
}

/// Appends `name` and its non-explicit dependencies to `packages_to_uninstall`
/// in dependency-first order. A root is always included; a dependency recorded
/// as explicit is left installed and not traversed. A package revisited while
/// still `Visiting` forms a cycle and is rejected.
fn visit(
    conn: &Connection,
    statuses: &mut HashMap<String, VisitStatus>,
    stack: &mut Vec<String>,
    packages_to_uninstall: &mut Vec<(PackageIdentity, String)>,
    name: String,
    is_root: bool,
) -> Result<()> {
    match statuses.get(&name) {
        Some(VisitStatus::Visited) => return Ok(()),
        Some(VisitStatus::Visiting) => {
            let start = stack.iter().position(|entry| entry == &name).unwrap_or(0);
            let mut cycle = stack[start..].to_vec();
            cycle.push(name);
            return Err(anyhow!("Dependency cycle: {}", cycle.join(" -> ")));
        }
        None => {}
    }

    let Some((version, release, architecture, install_dir, install_reason)) =
        look_up_installed(conn, &name)?
    else {
        if is_root {
            return Err(rusqlite::Error::QueryReturnedNoRows.into());
        }
        let parent = stack.last().map_or("unknown", String::as_str);
        return Err(anyhow!(
            "Dependency `{name}` of package `{parent}` not found"
        ));
    };

    if !is_root && InstallReason::from(install_reason) == InstallReason::Explicit {
        statuses.insert(name, VisitStatus::Visited);
        return Ok(());
    }

    statuses.insert(name.clone(), VisitStatus::Visiting);
    stack.push(name.clone());

    for dependency in dependencies_of(conn, &name)? {
        visit(
            conn,
            statuses,
            stack,
            packages_to_uninstall,
            dependency,
            false,
        )?;
    }

    stack.pop();
    statuses.insert(name.clone(), VisitStatus::Visited);
    packages_to_uninstall.push((
        PackageIdentity {
            name,
            version,
            release,
            architecture: Architectures::try_from(architecture)?,
        },
        install_dir,
    ));

    Ok(())
}

/// Reads the installed row for `name`, returning `None` when no row exists.
/// Prepare failures are contextualized; row conversion failures propagate.
fn look_up_installed(conn: &Connection, name: &str) -> Result<Option<InstalledRow>> {
    let mut stmt = conn
        .prepare(
            "SELECT version, release, architecture, install_dir, install_reason \
             FROM installed_packages WHERE name = ?",
        )
        .context("Failed to prepare database query")?;

    let row = stmt
        .query_row(params![name], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .optional()?;

    Ok(row)
}

/// Reads the direct dependency names recorded for `name`, ordered so the
/// traversal and any resulting error are deterministic.
fn dependencies_of(conn: &Connection, name: &str) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare(
            "SELECT dependency_name FROM package_dependencies \
             WHERE package_name = ? ORDER BY dependency_name",
        )
        .context("Failed to prepare database query")?;

    let dependencies = stmt
        .query_map(params![name], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(dependencies)
}

/// Rejects the plan when a package outside it depends on a package inside it.
/// Dependents that are themselves being removed are allowed, so a complete
/// dependency chain can be uninstalled as long as no external package needs it.
fn reject_external_dependents(
    conn: &Connection,
    packages_to_uninstall: &[(PackageIdentity, String)],
) -> Result<()> {
    let removal_set: HashSet<&str> = packages_to_uninstall
        .iter()
        .map(|(package, _)| package.name.as_str())
        .collect();

    let mut stmt = conn
        .prepare("SELECT package_name, dependency_name FROM package_dependencies")
        .context("Failed to prepare database query")?;

    let edges = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    for edge in edges {
        let (dependent, dependency) = edge?;
        if removal_set.contains(dependency.as_str()) && !removal_set.contains(dependent.as_str()) {
            return Err(anyhow!(
                "Can't uninstall {dependency} because {dependent} depends on it"
            ));
        }
    }

    Ok(())
}
