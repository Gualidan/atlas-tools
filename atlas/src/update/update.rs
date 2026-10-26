use std::{
    cmp::Ordering::{Equal, Greater, Less},
    env::consts::ARCH,
    path::PathBuf,
};

use anyhow::{Context, Result, anyhow};
use common::types::{
    config::load_config,
    package::{Architectures, PackageIdentity},
};
use rusqlite::{Connection, OptionalExtension, params};
use rustix::path::Arg;
use semver::Version;
use tempfile::{Builder, TempDir};

use crate::{
    cli::cli::UpdateArgs,
    install::{db::update_installed_package, install::confirmation_prompt},
    types::{guards::SwapBackGuard, package_state::PackageState, runtime_config::RuntimeConfig},
};

struct UpdateCandidate {
    name: String,
    version: String,
    release: u32,
    install_dir: String,
    sky_path: String,
}

/// Upgrades `args.package` for the host architecture, or every installed package
/// with an available upgrade when it is `None`. Repository lookups always use
/// the host architecture, even for installed rows with another architecture.
///
/// Replaces each package in its recorded directory when the repository's semantic
/// version is newer. Equal versions succeed without updating, even if release
/// numbers differ; downgrades fail. An empty update set succeeds without asking.
///
/// Prompts before applying the resolved updates. Propagates missing-package,
/// version parsing, database, configuration, verification, staging, and
/// filesystem errors from the update. Stops at the first error; earlier
/// successful updates remain applied.
pub fn update(runtime_config: &RuntimeConfig, args: UpdateArgs) -> Result<()> {
    let mut install_db_conn = Connection::open(&runtime_config.install_db)
        .context("Failed to connect to install database")?;

    let repo_db_conn = Connection::open(&runtime_config.repo_db)
        .context("Failed to open connection to repository database")?;

    let Resolution {
        candidates: to_update,
        error: deferred_error,
    } = resolve_update_order(&install_db_conn, &repo_db_conn, args.package.as_deref())?;

    if to_update.is_empty() {
        if let Some(error) = deferred_error {
            return Err(error);
        }
        match args.package.as_deref() {
            Some(package) => println!("Package {package} is already up to date"),
            None => println!("All packages are up to date"),
        }
        return Ok(());
    }

    let packages = to_update
        .iter()
        .map(|row| (row.name.as_str(), row.version.as_str()))
        .collect::<Vec<(&str, &str)>>();

    if !confirmation_prompt(packages, 4, "update")? {
        return Ok(());
    }

    for package in to_update {
        let package_id = PackageIdentity {
            name: &package.name,
            version: &package.version,
            release: package.release,
            architecture: Architectures::try_from(ARCH.to_string())?,
        };

        let install_dir = PathBuf::from(package.install_dir);

        let tempdir = Builder::new()
            .prefix("sisyphus_install_tmp_dir_")
            .tempdir_in(
                install_dir
                    .parent()
                    .context("Failed to get install dir's parent")?,
            )
            .context("Failed to create temporary directory")?;

        update_one(
            package_id,
            &repo_db_conn,
            &mut install_db_conn,
            PathBuf::from(package.sky_path),
            tempdir,
            install_dir,
        )?;
    }

    match deferred_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

struct Resolution {
    candidates: Vec<UpdateCandidate>,
    error: Option<anyhow::Error>,
}

/// Resolves the packages that have a newer repository version. With `name`, only
/// that installed package is considered; without it, every installed package is
/// visited in name order. Bulk resolution reports a per-package failure
/// alongside the candidates resolved before it, so earlier updates still apply.
fn resolve_update_order(
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

fn resolve_one(
    install_conn: &Connection,
    repo_conn: &Connection,
    name: &str,
) -> Result<Vec<UpdateCandidate>> {
    let mut install_stmt = install_conn
        .prepare(
            "SELECT version, install_dir FROM installed_packages WHERE name = ? AND architecture = ?",
        )
        .context("Failed to prepare database query")?;

    let Some((installed_version, install_dir)) = install_stmt
        .query_row(params![name, ARCH], |row| {
            let version: String = row.get(0)?;
            let install_dir: String = row.get(1)?;
            Ok((version, install_dir))
        })
        .optional()?
    else {
        return Err(anyhow!("Package {name} not found in installed packages"));
    };

    let mut repo_stmt = repo_conn
        .prepare(
            "SELECT version, release, sky_path FROM packages WHERE name = ? AND architecture = ?",
        )
        .context("Failed to prepare database statement")?;

    let Some((repo_version, repo_release, sky_path)) = repo_stmt
        .query_row(params![name, ARCH], |row| {
            let version: String = row.get(0)?;
            let release: u32 = row.get(1)?;
            let sky_path: String = row.get(2)?;
            Ok((version, release, sky_path))
        })
        .optional()?
    else {
        return Err(anyhow!("Package {name} not found"));
    };

    match compare_versions(
        &installed_version,
        &repo_version,
        "Failed to parse version number",
    )? {
        Less => Ok(vec![UpdateCandidate {
            name: name.to_string(),
            version: repo_version,
            release: repo_release,
            install_dir,
            sky_path,
        }]),
        Equal => Ok(Vec::new()),
        Greater => Err(downgrade_error(name, &installed_version, &repo_version)),
    }
}

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
                    error: Some(anyhow!("Package {} not found", package.name)),
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

fn compare_versions(
    installed: &str,
    available: &str,
    parse_context: &'static str,
) -> Result<std::cmp::Ordering> {
    let installed = Version::parse(installed).context(parse_context)?;
    let available = Version::parse(available).context(parse_context)?;
    Ok(installed.cmp(&available))
}

fn downgrade_error(name: &str, installed: &str, available: &str) -> anyhow::Error {
    anyhow!(
        "Not allowed to downgrade {name}: installed version is {installed} while the repository's is {available}"
    )
}

/// Verifies and stages the supplied package, exchanges it into `install_dir`,
/// and updates the existing install records. The caller selects the version;
/// this function does not compare it with the installed version.
///
/// `install_dir` is expected to exist, and `tempdir` must share its filesystem.
/// Recording failures trigger a best-effort swap back. Configuration,
/// verification, staging, filesystem, and database errors propagate.
pub fn update_one(
    package_id: PackageIdentity,
    repo_conn: &Connection,
    install_conn: &mut Connection,
    sky_path: PathBuf,
    tempdir: TempDir,
    install_dir: PathBuf,
) -> Result<()> {
    let config = load_config().context("Failed to load configuration")?;

    let state = PackageState::NotInstalled {
        package_id,
        install_conn,
        sky_path: &sky_path,
        tempdir,
        config: &config,
        repo_conn,
        install_dir: &install_dir,
    };
    let state = state.verify()?;
    let state = state.stage()?;
    let (_, metadata, staging_dir) = state.commit()?;
    let mut guard = SwapBackGuard::new(&install_dir, &staging_dir, true);

    update_installed_package(
        install_conn,
        repo_conn,
        &metadata,
        &package_id,
        &sky_path.to_string_lossy(),
        &install_dir.to_string_lossy(),
    )?;

    guard.disarm();
    Ok(())
}
