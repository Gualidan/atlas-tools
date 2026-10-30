use std::{
    cmp::Ordering::{Equal, Greater, Less},
    collections::HashSet,
    env::consts::ARCH,
};

use anyhow::{Context, Result, anyhow};
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
}

struct ResolvedPackage {
    name: String,
    parent: Option<String>,
}

/// Resolves the packages that have a newer repository version. With `name`, only
/// that installed package is considered; without it, every installed package is
/// visited in name order. Bulk resolution returns the first per-package failure
/// in `Resolution.error` alongside earlier candidates. Single-package failures
/// and bulk query setup or installed-row errors propagate directly. No updates
/// are applied here.
///
/// # Errors
/// This function might error because of a failure during resolution
pub fn resolve_update_order(
    install_conn: &Connection,
    repo_conn: &Connection,
    name: Option<&str>,
) -> Result<Resolution> {
    match name {
        Some(name) => Ok(Resolution {
            candidates: resolve_one(install_conn, repo_conn, name)?,
            error: None,
        }),
        None => resolve_all(install_conn, repo_conn),
    }
}

/// Resolves a single installed package and repository entry for the host
/// architecture, retaining its recorded install directory. Returns one candidate
/// for a newer semantic version, or none for an equal version regardless of release.
/// Entries missing from either database, downgrades, version parsing, and database
/// errors propagate.
fn resolve_one(
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
                            "SELECT version, release, sky_path FROM packages WHERE name = ? AND architecture = ?",
                        )
                        .context("Failed to prepare database statement")?;

        let Some((repo_version, repo_release, sky_path)) = repo_stmt
            .query_row(params![package.name, ARCH], |row| {
                let version: String = row.get(0)?;
                let release: u32 = row.get(1)?;
                let sky_path: String = row.get(2)?;
                Ok((version, release, sky_path))
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
fn resolve_all(install_conn: &Connection, repo_conn: &Connection) -> Result<Resolution> {
    let mut install_stmt = install_conn
        .prepare(
            "SELECT name, version, install_dir, sky_path, release FROM installed_packages ORDER BY name",
        )
        .context("Failed to prepare database query")?;

    let installed_rows = install_stmt
        .query_map([], |row| {
            Ok(UpdateCandidate {
                name: row.get(0)?,
                version: row.get(1)?,
                install_dir: row.get(2)?,
                sky_path: row.get(3)?,
                release: row.get(4)?,
            })
        })
        .context("Failed to list all installed packages")?
        .collect::<Result<Vec<UpdateCandidate>, _>>()?;

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
