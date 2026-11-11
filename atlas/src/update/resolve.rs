use std::{
    cmp::Ordering::{Equal, Greater, Less},
    collections::HashSet,
    env::consts::ARCH,
};

use anyhow::{Context, Result, anyhow, bail};
use rusqlite::{Connection, OptionalExtension, params};
use semver::Version;

pub struct Resolution {
    pub candidates: Vec<UpdateCandidate>,
    pub error: Option<anyhow::Error>,
}

pub struct UpdateCandidate {
    pub name: String,
    pub version: String,
    pub release: u32,
    pub install_dir: String,
    pub sky_path: String,
    pub groups: Vec<String>,
}

struct ResolvedPackage {
    name: String,
    parent: Option<String>,
}

/// Resolves a single installed package and repository entry for the host
/// architecture, retaining its recorded install directory. Returns one candidate
/// for a newer semantic version, or none for an equal version regardless of release.
/// Entries missing from either database, downgrades, version parsing, and database
/// errors propagate.
///
/// # Errors
///
/// Returns an error if the package cannot be resolved.
pub fn resolve_one(
    install_conn: &Connection,
    repo_conn: &Connection,
    name: &str,
) -> Result<Vec<UpdateCandidate>> {
    let mut update_candidates: Vec<UpdateCandidate> = Vec::new();
    let mut path: Vec<String> = Vec::new();
    let mut done: HashSet<String> = HashSet::new();
    let mut packages: Vec<ResolvedPackage> = Vec::new();

    visit(
        &mut path,
        &mut done,
        &mut packages,
        name.to_string(),
        repo_conn,
    )?;

    let mut install_stmt = install_conn
                .prepare(
                    "SELECT version, install_dir FROM installed_packages WHERE name = ? AND architecture = ?",
                )
                .context("Failed to prepare database query")?;

    for package in packages {
        let parent = package.parent;
        let mut repo_stmt = repo_conn
                 .prepare(
                            "SELECT version, release, sky_path, groups FROM packages WHERE name = ? AND architecture = ?",
                        )
                        .context("Failed to prepare database statement")?;

        let Some((repo_version, repo_release, sky_path, groups)) = repo_stmt
            .query_row(params![package.name, ARCH], |row| {
                let version: String = row.get(0)?;
                let release: u32 = row.get(1)?;
                let sky_path: String = row.get(2)?;
                let groups: String = row.get(3)?;
                Ok((version, release, sky_path, groups))
            })
            .optional()?
        else {
            if let Some(parent) = parent {
                return Err(anyhow!(
                    "Dependency of package `{parent}` `{}` not found in repository",
                    package.name
                ));
            }
            return Err(anyhow!("Package `{name}` not found in repository"));
        };

        let Some((installed_version, install_dir)) = install_stmt
            .query_row(params![package.name, ARCH], |row| {
                let version: String = row.get(0)?;
                let install_dir: String = row.get(1)?;
                Ok((version, install_dir))
            })
            .optional()?
        else {
            let Some(parent) = parent else {
                return Err(anyhow!(
                    "Package `{}` not found in installed packages",
                    package.name
                ));
            };
            return Err(anyhow!(
                "New `{}` version declares `{}` as a dependency but it is not yet installed, try reinstalling `{}`",
                parent,
                package.name,
                parent,
            ));
        };

        match compare_versions(
            &installed_version,
            &repo_version,
            "Failed to parse version number",
        )? {
            Less => update_candidates.push(UpdateCandidate {
                name: package.name,
                version: repo_version,
                release: repo_release,
                install_dir,
                sky_path,
                groups: serde_saphyr::from_str(&groups).context("Failed to parse groups")?,
            }),
            Equal => {}
            Greater => {
                return Err(downgrade_error(
                    &package.name,
                    &installed_version,
                    &repo_version,
                ));
            }
        }
    }

    Ok(update_candidates)
}

fn visit(
    path: &mut Vec<String>,
    done: &mut HashSet<String>,
    update_candidates: &mut Vec<ResolvedPackage>,
    name: String,
    conn: &Connection,
) -> Result<()> {
    if done.contains(&name) {
        return Ok(());
    }
    if path.contains(&name) {
        let start = path.iter().position(|entry| entry == &name).unwrap_or(0);
        let mut cycle = path[start..].to_vec();
        cycle.push(name.clone());
        return Err(anyhow!("dependency cycle: {}", cycle.join(" -> ")));
    }

    path.push(name.clone());

    let mut stmt = conn
        .prepare("SELECT deps FROM packages WHERE name = ?")
        .context("Failed to prepare database query")?;

    let Some(deps): Option<String> = stmt.query_row(params![name], |row| row.get(0)).optional()?
    else {
        return Err(match path.iter().rev().nth(1) {
            Some(parent) => anyhow!("Dependency of `{parent}` `{name}` not found in repository"),
            None => anyhow!("Package `{name}` not found in repository"),
        });
    };

    let deps: Vec<String> = serde_saphyr::from_str(&deps).context("Failed to serialize YAML")?;

    for dep in deps {
        visit(path, done, update_candidates, dep, conn)?;
    }

    done.insert(name.clone());
    // slice patterns: bind second-to-last and last directly
    let parent = match path.as_slice() {
        [.., parent, _] => Some(parent.clone()),
        [_] => None, // just the root
        [] => unreachable!("path never empty inside visit"),
    };

    update_candidates.push(ResolvedPackage { name, parent });

    path.pop();
    Ok(())
}

/// Resolves upgrades in installed-package name order, using host-architecture
/// repository entries even for installed rows with a different architecture.
/// Equal semantic versions are skipped regardless of release numbers.
///
/// Query setup and installed-row errors propagate directly. The first missing
/// repository entry, repository-row error, invalid version, or downgrade ends
/// resolution and is returned in `Resolution.error` with earlier candidates.
///
/// # Errors
///
/// Returns an error if the packages cannot be resolved.
pub fn resolve_all(install_conn: &Connection, repo_conn: &Connection) -> Result<Resolution> {
    let mut install_stmt = install_conn
        .prepare(
            "SELECT name, version, install_dir, sky_path, release, groups FROM installed_packages ORDER BY name",
        )
        .context("Failed to prepare database query")?;

    let installed_rows = install_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, u32>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .context("Failed to list all installed packages")?
        .map(|row| {
            let (name, version, install_dir, sky_path, release, groups_yaml) = row?;
            Ok(UpdateCandidate {
                name,
                version,
                install_dir,
                sky_path,
                release,
                groups: serde_saphyr::from_str(&groups_yaml).context("Failed to parse groups")?,
            })
        })
        .collect::<Result<Vec<UpdateCandidate>>>()?;

    let mut repo_stmt = repo_conn
        .prepare(
            "SELECT version, release, sky_path FROM packages WHERE name = ? AND architecture = ?",
        )
        .context("Failed to prepare database statement")?;

    let mut to_update = Vec::new();

    for package in installed_rows {
        let repo_record = repo_stmt
            .query_row(params![package.name, ARCH], |row| {
                let version: String = row.get(0)?;
                let release: u32 = row.get(1)?;
                let sky_path: String = row.get(2)?;
                Ok((version, release, sky_path))
            })
            .optional();

        let (repo_version, repo_release, sky_path) = match repo_record {
            Ok(Some(record)) => record,
            Ok(None) => {
                return Ok(Resolution {
                    candidates: to_update,
                    error: Some(anyhow!(
                        "Package `{}` not found in repository",
                        package.name
                    )),
                });
            }
            Err(error) => {
                return Ok(Resolution {
                    candidates: to_update,
                    error: Some(error.into()),
                });
            }
        };

        match compare_versions(&package.version, &repo_version, "Failed to parse version") {
            Ok(Less) => to_update.push(UpdateCandidate {
                name: package.name,
                version: repo_version,
                release: repo_release,
                install_dir: package.install_dir,
                sky_path,
                groups: package.groups,
            }),
            Ok(Equal) => {}
            Ok(Greater) => {
                return Ok(Resolution {
                    candidates: to_update,
                    error: Some(downgrade_error(
                        &package.name,
                        &package.version,
                        &repo_version,
                    )),
                });
            }
            Err(error) => {
                return Ok(Resolution {
                    candidates: to_update,
                    error: Some(error),
                });
            }
        }
    }

    Ok(Resolution {
        candidates: to_update,
        error: None,
    })
}

#[allow(clippy::too_many_lines)]
/// Resolves a group of packages from the repository and install database.
///
/// # Errors
///
/// Returns an error if the group could not be resolved.
pub fn resolve_group(
    repo_conn: &Connection,
    install_conn: &Connection,
    group: &str,
) -> Result<Resolution> {
    let mut resolved_packages: Vec<ResolvedPackage> = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut resolved: HashSet<String> = HashSet::new();
    let mut package_names_to_resolve: Vec<String> = Vec::new();
    let mut candidates: Vec<UpdateCandidate> = Vec::new();

    let mut stmt = repo_conn.prepare("SELECT name, groups FROM packages")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    for row in rows {
        let (name, groups) = row?;
        let groups: Vec<String> =
            serde_saphyr::from_str(&groups).context("Failed to deserialize groups")?;

        if groups.iter().any(|g| g == group) {
            package_names_to_resolve.push(name);
        }
    }

    if package_names_to_resolve.is_empty() {
        bail!("Unknown package group `{group}`");
    }

    for name in package_names_to_resolve {
        visit(
            &mut stack,
            &mut resolved,
            &mut resolved_packages,
            name.clone(),
            repo_conn,
        )?;
    }

    let mut install_stmt = install_conn
                 .prepare(
                     "SELECT version, install_dir FROM installed_packages WHERE name = ? AND architecture = ?",
                 )
                 .context("Failed to prepare database query")?;

    for package in resolved_packages {
        let parent = package.parent;
        let mut repo_stmt = repo_conn
                     .prepare(
                                "SELECT version, release, sky_path, groups FROM packages WHERE name = ? AND architecture = ?",
                            )
                            .context("Failed to prepare database statement")?;

        let Some((repo_version, repo_release, sky_path, groups)) = repo_stmt
            .query_row(params![package.name, ARCH], |row| {
                let version: String = row.get(0)?;
                let release: u32 = row.get(1)?;
                let sky_path: String = row.get(2)?;
                let groups: String = row.get(3)?;
                Ok((version, release, sky_path, groups))
            })
            .optional()?
        else {
            if let Some(parent) = parent {
                return Err(anyhow!(
                    "Dependency of package `{parent}` `{}` not found in repository",
                    package.name
                ));
            }
            return Err(anyhow!(
                "Package `{}` from group `{}` not found in repository",
                package.name,
                group
            ));
        };

        let Some((installed_version, install_dir)) = install_stmt
            .query_row(params![package.name, ARCH], |row| {
                let version: String = row.get(0)?;
                let install_dir: String = row.get(1)?;
                Ok((version, install_dir))
            })
            .optional()?
        else {
            let Some(parent) = parent else {
                return Err(anyhow!(
                    "Package `{}` from group `{}` is not installed",
                    package.name,
                    group
                ));
            };
            return Err(anyhow!(
                "New `{}` version declares `{}` as a dependency but it is not yet installed, try reinstalling `{}`",
                parent,
                package.name,
                parent,
            ));
        };

        match compare_versions(
            &installed_version,
            &repo_version,
            "Failed to parse version number",
        )? {
            Less => candidates.push(UpdateCandidate {
                name: package.name,
                version: repo_version,
                release: repo_release,
                install_dir,
                sky_path,
                groups: serde_saphyr::from_str(&groups).context("Failed to parse groups")?,
            }),
            Equal => {}
            Greater => {
                return Err(downgrade_error(
                    &package.name,
                    &installed_version,
                    &repo_version,
                ));
            }
        }
    }

    Ok(Resolution {
        candidates,
        error: None,
    })
}

/// Compares `installed` against `available` as semantic versions. Returns a
/// parsing error with `parse_context` attached if either version is invalid.
///
/// # Errors
/// This function might error because it failed to parse the version numbers
pub fn compare_versions(
    installed: &str,
    available: &str,
    parse_context: &'static str,
) -> Result<std::cmp::Ordering> {
    let installed = Version::parse(installed).context(parse_context)?;
    let available = Version::parse(available).context(parse_context)?;
    Ok(installed.cmp(&available))
}

/// Builds a downgrade error identifying the package and both version strings.
fn downgrade_error(name: &str, installed: &str, available: &str) -> anyhow::Error {
    anyhow!(
        "Not allowed to downgrade {name}: installed version is {installed} while the repository's is {available}"
    )
}
