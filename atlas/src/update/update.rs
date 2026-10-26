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
use inquire::Confirm;
use rusqlite::{Connection, OptionalExtension, params};
use rustix::path::Arg;
use semver::Version;
use tempfile::{Builder, TempDir};

use crate::{
    cli::cli::UpdateArgs,
    install::db::update_installed_package,
    types::{guards::SwapBackGuard, package_state::PackageState, runtime_config::RuntimeConfig},
};

/// Upgrades `args.package` for the host architecture, or visits all installed
/// packages in name order when it is `None`. Repository lookups always use the
/// host architecture, even for installed rows with another architecture.
///
/// Replaces each package in its recorded directory when the repository's semantic
/// version is newer. Equal versions succeed without updating, even if release
/// numbers differ; downgrades fail. An empty installed list succeeds.
///
/// Propagates missing-package, version parsing, database, configuration,
/// verification, staging, and filesystem errors from the update. Stops at the
/// first error; earlier successful updates remain applied.
pub fn update(runtime_config: &RuntimeConfig, args: UpdateArgs) -> Result<()> {
    let mut install_db_conn = Connection::open(&runtime_config.install_db)
        .context("Failed to connect to install database")?;

    let repo_db_conn = Connection::open(&runtime_config.repo_db)
        .context("Failed to open connection to repository database")?;

    if let Some(package) = args.package {
        let mut install_stmt  = install_db_conn.prepare("SELECT version, install_dir FROM installed_packages WHERE name = ? AND architecture = ?").context("Failed to prepare database query")?;
        let Some((installed_version, install_dir)) = install_stmt
            .query_row(params![&package, ARCH], |row| {
                let version: String = row.get::<_, String>(0)?;
                let install_dir: String = row.get::<_, String>(1)?;
                Ok((version, install_dir))
            })
            .optional()?
        else {
            return Err(anyhow!(
                "Package {} not found in installed packages",
                package
            ));
        };

        let mut repo_stmt = repo_db_conn
                .prepare(
                    "SELECT version, release, sky_path FROM packages WHERE name = ? AND architecture = ?",
                )
                .context("Failed to prepare database statement")?;
        let Some((repo_version, repo_release, sky_path)): Option<(String, u32, String)> = repo_stmt
            .query_row(params![package, ARCH], |row| {
                let version: String = row.get(0)?;
                let release: u32 = row.get(1)?;
                let sky_path: String = row.get(2)?;
                Ok((version, release, sky_path))
            })
            .optional()?
        else {
            return Err(anyhow!("Package {} not found", package));
        };

        let ans = Confirm::new(&format!(
            "This will update {}, do you wish to proceed? [Y/n]",
            package
        ))
        .with_default(true)
        .prompt();

        match ans {
            Ok(_) => {}
            Err(_) => return Err(anyhow!("Invalid answer try again")),
        }

        let installed_version =
            Version::parse(&installed_version).context("Failed to parse version number")?;
        let available_version =
            Version::parse(&repo_version).context("Failed to parse version number")?;

        match installed_version.cmp(&available_version) {
            Greater => Err(anyhow!(
                "Not allowed to downgrade your installed package is version {installed_version} while the repository's is version {repo_version}"
            )),
            Equal => {
                println!("Package {} is already up to date", package);
                return Ok(());
            }
            Less => Ok({
                let package_id = PackageIdentity {
                    name: &package,
                    version: &repo_version,
                    release: repo_release,
                    architecture: Architectures::try_from(ARCH.to_string())?,
                };

                let install_dir = PathBuf::from(install_dir);

                let tempdir = Builder::new()
                    .prefix("sisyphus_install_tmp_dir_")
                    .tempdir_in(
                        install_dir
                            .parent()
                            .context("Failed to get install dir's parent")?,
                    )
                    .context("Failed to create temporary directory")?;

                drop(install_stmt);

                update_one(
                    package_id,
                    &repo_db_conn,
                    &mut install_db_conn,
                    PathBuf::from(sky_path),
                    tempdir,
                    install_dir,
                )?;
            }),
        }?
    } else {
        let mut install_stmt = install_db_conn
            .prepare("SELECT name, version, install_dir FROM installed_packages ORDER BY name")
            .context("Failed to prepare database query")?;
        struct InstalledRow {
            name: String,
            version: String,
            install_dir: String,
        }

        let installed_rows = install_stmt
            .query_map([], |row| {
                Ok(InstalledRow {
                    name: row.get(0)?,
                    version: row.get(1)?,
                    install_dir: row.get(4)?,
                })
            })
            .context("Failed to list all installed packages")?
            .collect::<Result<Vec<InstalledRow>, _>>()?;

        drop(install_stmt);

        let mut repo_stmt = repo_db_conn
                                    .prepare(
                                        "SELECT version, release, sky_path FROM packages WHERE name = ? AND architecture = ?",
                                    )
                                    .context("Failed to prepare database statement")?;

        for package in installed_rows {
            let Some((repo_version, repo_release, sky_path)): Option<(String, u32, String)> =
                repo_stmt
                    .query_row(params![package.name, ARCH], |row| {
                        let version: String = row.get(0)?;
                        let release: u32 = row.get(1)?;
                        let sky_path: String = row.get(2)?;
                        Ok((version, release, sky_path))
                    })
                    .optional()?
            else {
                return Err(anyhow!("Package {} not found", package.name));
            };

            let installed_version =
                Version::parse(&package.version).context("Failed to parse version")?;
            let available_version =
                Version::parse(&repo_version).context("Failed to parse version")?;

            match installed_version.cmp(&available_version) {
                Greater => Err(anyhow!(
                    "Not allowed to downgrade your installed package is version {installed_version} while the repository's is version {repo_version}"
                )),
                Equal => {
                    println!("Package {} is already up to date", package.name);
                    continue;
                }
                Less => Ok({
                    let package_id = PackageIdentity {
                        name: &package.name,
                        version: &repo_version,
                        release: repo_release,
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
                        PathBuf::from(sky_path),
                        tempdir,
                        install_dir,
                    )?;
                }),
            }?
        }
    }

    Ok(())
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
