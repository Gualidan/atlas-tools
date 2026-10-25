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
    install::db::update_installed_package,
    types::{guards::SwapBackGuard, package_state::PackageState, runtime_config::RuntimeConfig},
};

pub fn update(runtime_config: &RuntimeConfig, args: UpdateArgs) -> Result<()> {
    let mut install_db_conn = Connection::open(&runtime_config.install_db)
        .context("Failed to connect to install database")?;

    let mut install_stmt  = install_db_conn.prepare("SELECT version, install_dir FROM installed_packages WHERE name = ? AND architecture = ?").context("Failed to prepare database query")?;
    let Some((installed_version, install_dir)) = install_stmt
        .query_row(params![&args.package, ARCH], |row| {
            let version: String = row.get::<_, String>(0)?;
            let install_dir: String = row.get::<_, String>(1)?;
            Ok((version, install_dir))
        })
        .optional()?
    else {
        return Err(anyhow!(
            "Package {} not found in installed packages",
            &args.package
        ));
    };

    let repo_db_conn = Connection::open(&runtime_config.repo_db)
        .context("Failed to open connection to repository database")?;

    let mut repo_stmt = repo_db_conn
        .prepare(
            "SELECT version, release, sky_path FROM packages WHERE name = ? AND architecture = ?",
        )
        .context("Failed to prepare database statement")?;
    let Some((repo_version, repo_release, sky_path)): Option<(String, u32, String)> = repo_stmt
        .query_row(params![args.package, ARCH], |row| {
            let version: String = row.get(0)?;
            let release: u32 = row.get(1)?;
            let sky_path: String = row.get(2)?;
            Ok((version, release, sky_path))
        })
        .optional()?
    else {
        return Err(anyhow!("Package {} not found", &args.package));
    };

    let installed_version =
        Version::parse(&installed_version).context("Failed to parse version number")?;
    let available_version =
        Version::parse(&repo_version).context("Failed to parse version number")?;

    match installed_version.cmp(&available_version) {
        Greater => {
            return Err(anyhow!(
                "Not allowed to downgrade your installed package is version {installed_version} while the repository's is version {repo_version}"
            ));
        }
        Equal => {
            println!("Package {} is already up to date", &args.package);
            return Ok(());
        }
        Less => {
            let package_id = PackageIdentity {
                name: args.package.as_str(),
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

            Ok(())
        }
    }
}

pub fn update_one(
    package_id: PackageIdentity,
    repo_conn: &Connection,
    mut install_conn: &mut Connection,
    sky_path: PathBuf,
    tempdir: TempDir,
    install_dir: PathBuf,
) -> Result<()> {
    let config = load_config().context("Failed to load configuration")?;

    let state = PackageState::NotInstalled {
        package_id,
        install_conn: &install_conn,
        sky_path: &sky_path,
        tempdir,
        config: &config,
        repo_conn: &repo_conn,
        install_dir: &install_dir,
    };
    let state = state.verify()?;
    let state = state.stage()?;
    let (_, metadata, staging_dir) = state.commit()?;
    let mut guard = SwapBackGuard::new(&install_dir, &staging_dir, true);

    update_installed_package(
        &mut install_conn,
        repo_conn,
        &metadata,
        &package_id,
        &sky_path.to_string_lossy(),
        &install_dir.to_string_lossy(),
    )?;

    guard.disarm();
    Ok(())
}
