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

#[derive(Debug, Clone, PartialEq)]
pub struct SkippedPackage {
    pub name: String,
    pub required_by: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UninstallReport {
    pub skipped: Vec<SkippedPackage>,
}

/// Resolves `name` to installed identities and directory paths for removal.
///
/// The plan contains the requested package plus every non-explicit dependency
/// reachable through `package_dependencies`. Explicit dependencies stay
/// installed and stop the traversal. Packages with dependents outside the plan,
/// and their planned dependencies, are skipped without filtering by architecture.
/// Does not remove files or change database records.
///
/// Returns an error for a missing installed row, database failures, an
/// unsupported architecture, a dependency cycle, or a dependency with no
/// installed row.
///
/// # Errors
/// This function might error because of a failure during the resolution
pub fn packages_to_uninstall(
    conn: &Connection,
    name: String,
) -> Result<(Vec<(PackageIdentity, String)>, UninstallReport)> {
    let mut packages_to_uninstall: Vec<(PackageIdentity, String)> = Vec::new();
    let mut statuses: HashMap<String, VisitStatus> = HashMap::new();
    let mut stack: Vec<String> = Vec::new();
    let mut skipped: Vec<SkippedPackage> = Vec::new();

    visit(
        conn,
        &mut statuses,
        &mut stack,
        &mut packages_to_uninstall,
        name,
        true,
    )?;

    skip_external_dependents(conn, &mut packages_to_uninstall, &mut skipped)?;

    Ok((packages_to_uninstall, UninstallReport { skipped }))
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

/// Keeps externally required packages and their planned dependencies installed.
fn skip_external_dependents(
    conn: &Connection,
    packages_to_uninstall: &mut Vec<(PackageIdentity, String)>,
    skipped: &mut Vec<SkippedPackage>,
) -> Result<()> {
    let removal_set: HashSet<String> = packages_to_uninstall
        .iter()
        .map(|(package, _)| package.name.clone())
        .collect();
    let mut stmt = conn
        .prepare(
            "SELECT package_name, dependency_name FROM package_dependencies \
             ORDER BY package_name, dependency_name",
        )
        .context("Failed to prepare database query")?;

    let edges = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut dependencies: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut skipped_names = HashSet::new();

    for (dependent, dependency) in &edges {
        if !removal_set.contains(dependency) {
            continue;
        }
        if removal_set.contains(dependent) {
            dependencies.entry(dependent).or_default().push(dependency);
        } else if skipped_names.insert(dependency.clone()) {
            skipped.push(SkippedPackage {
                name: dependency.clone(),
                required_by: dependent.clone(),
            });
        }
    }

    let mut index = 0;
    while index < skipped.len() {
        let dependent = skipped[index].name.clone();
        if let Some(deps) = dependencies.get(dependent.as_str()) {
            for &dependency in deps {
                if skipped_names.insert(dependency.to_owned()) {
                    skipped.push(SkippedPackage {
                        name: dependency.to_owned(),
                        required_by: dependent.clone(),
                    });
                }
            }
        }
        index += 1;
    }

    packages_to_uninstall.retain(|(package, _)| !skipped_names.contains(&package.name));
    Ok(())
}
