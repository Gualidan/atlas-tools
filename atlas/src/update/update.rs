use std::{
    cmp::Ordering::{Equal, Greater, Less},
    env::consts::ARCH,
    path::PathBuf,
};

use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, OptionalExtension, params};
use semver::Version;

use crate::{
    cli::cli::UpdateArgs, install::install::install, types::runtime_config::RuntimeConfig,
    uninstall::uninstall::uninstall,
};

pub fn update(runtime_config: &RuntimeConfig, args: UpdateArgs) -> Result<()> {
    let install_db_conn = Connection::open(&runtime_config.install_db)
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
        .prepare("SELECT version FROM packages WHERE name = ? AND architecture = ?")
        .context("Failed to prepare database statement")?;
    let Some(repo_version): Option<String> = repo_stmt
        .query_row(params![args.package, ARCH], |row| row.get(0))
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
            uninstall(runtime_config, &args.package)?;
            install(
                &args.package,
                runtime_config,
                PathBuf::from(install_dir)
                    .parent()
                    .context("Failed to get path's parent")?
                    .to_path_buf(),
            )?;
            Ok(())
        }
    }
}
