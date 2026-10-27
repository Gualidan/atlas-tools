use anyhow::{Context as AnyhowContext, Result, anyhow};
use common::types::{
    config::SisyphusConfig,
    package::{Architectures, PackageIdentity},
};
use inquire::Confirm;
use std::{
    env::consts::ARCH,
    fs::create_dir_all,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

use common::types::config::load_config;
use rusqlite::{Connection, OptionalExtension, params};

use tempfile::{Builder, TempDir};

use crate::{
    install::{
        db::{InstallReason, ensure_install_schema, insert_installed_package},
        resolve::resolve_install_order,
    },
    types::{guards::SwapBackGuard, package_state::PackageState, runtime_config::RuntimeConfig},
};

/// Installs the named package and its repository-declared dependencies for the
/// host architecture under `install_dir/<name>-<version>-<release>`.
/// Already installed dependencies are skipped; the requested package is reinstalled.
///
/// Requests confirmation for the resolved package list unless
/// [`set_assume_yes`] is enabled. Declining succeeds without installing anything.
///
/// Propagates resolution, configuration, confirmation, verification, filesystem,
/// and database errors. Packages successfully installed before a later failure
/// remain installed.
pub fn install(
    package_name: &String,
    runtime_config: &RuntimeConfig,
    install_dir: PathBuf,
) -> Result<()> {
    let conn = Connection::open(&runtime_config.repo_db)
        .context("Failed to open connection to the repository database")?;

    let mut stmt = conn
        .prepare("SELECT name FROM packages WHERE name = ? AND architecture = ?")
        .context("Failed to prepare database query")?;
    let Some(name): Option<String> = stmt
        .query_row(params![package_name, ARCH], |row| row.get(0))
        .optional()?
    else {
        return Err(anyhow!("Package {} not found", package_name));
    };

    let architecture = Architectures::try_from(ARCH.to_string())?;
    let config = load_config().context("Failed to load settings")?;

    let to_install = resolve_install_order(&conn, name.clone(), architecture)?;
    let packages = to_install
        .iter()
        .map(|pkg| (pkg.name.as_str(), pkg.version.as_str()))
        .collect::<Vec<(&str, &str)>>();

    let ans = confirmation_prompt(packages, 4, "install")?;

    if !ans {
        return Ok(());
    }

    create_dir_all(&install_dir).context("Failed to create install_dir's parents")?;

    if let Some(parent) = runtime_config.install_db.parent() {
        std::fs::create_dir_all(parent)
            .context("Failed to create the install database directory")?;
    }

    let install_conn = Connection::open(&runtime_config.install_db)
        .context("Failed to open connection to install database")?;
    install_conn
        .execute("PRAGMA foreign_keys = ON", [])
        .context("Failed to enable foreign key enforcement")?;
    ensure_install_schema(&install_conn).context("Failed to create the install database schema")?;

    let mut stmt = install_conn
        .prepare("SELECT name FROM installed_packages WHERE name = ? AND architecture = ?")
        .context("Failed to prepare database query")?;

    for package in to_install {
        let is_root = package.name == name;

        if !is_root {
            let installed = stmt
                .query_row(
                    params![package.name, package.architecture.to_string()],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .is_some();

            if installed {
                continue;
            }
        };

        let install_reason = if is_root {
            InstallReason::Explicit
        } else {
            InstallReason::Dependency
        };

        let install_dir = install_dir.join(format!(
            "{}-{}-{}",
            package.name, package.version, package.release
        ));
        let install_dir_existed = install_dir.exists();

        let package_id = PackageIdentity {
            name: package.name.as_str(),
            version: package.version.as_str(),
            release: package.release,
            architecture: package.architecture,
        };

        let tempdir = Builder::new()
            .prefix("sisyphus_install_tmp_dir_")
            .tempdir_in(
                install_dir
                    .parent()
                    .context("Failed to get install dir's parent")?,
            )
            .context("Failed to create temporary directory")?;

        install_one(
            runtime_config,
            install_dir_existed,
            install_reason,
            package_id,
            PathBuf::from(&package.sky_path),
            install_dir,
            tempdir,
            &config,
            &conn,
            &install_conn,
        )?;
    }

    Ok(())
}

/// Verifies and stages one package, exchanges it into `install_dir`, and records
/// it in the database selected by `runtime_config` with `install_reason`.
/// `conn` supplies repository identities; `install_conn` supplies file ownership
/// records for conflict checks.
///
/// `tempdir` must share a filesystem with `install_dir`. `install_dir_existed`
/// describes the destination before the exchange. Database recording failures
/// trigger a best-effort swap back, removing a newly created destination.
/// Verification, staging, filesystem, and database errors propagate.
///
/// Panics if `sky_path` is not valid UTF-8 when recording the installation.
pub fn install_one(
    runtime_config: &RuntimeConfig,
    install_dir_existed: bool,
    install_reason: InstallReason,
    package_id: PackageIdentity,
    sky_path: PathBuf,
    install_dir: PathBuf,
    tempdir: TempDir,
    config: &SisyphusConfig,
    conn: &Connection,
    install_conn: &Connection,
) -> Result<()> {
    let state = PackageState::NotInstalled {
        package_id,
        sky_path: &sky_path,
        install_dir: &install_dir,
        tempdir,
        config,
        repo_conn: conn,
        install_conn,
    };
    let state = state.verify()?;
    let state = state.stage()?;
    let (_, metadata, staging_dir) = state.commit()?;

    let mut guard = SwapBackGuard::new(&install_dir, &staging_dir, install_dir_existed);

    insert_installed_package(
        runtime_config,
        &metadata,
        &package_id,
        install_dir.to_string_lossy().to_string(),
        sky_path.to_str().unwrap(),
        install_reason,
    )?;

    guard.disarm();
    Ok(())
}

static ASSUME_YES: AtomicBool = AtomicBool::new(false);

/// Skips interactive confirmation prompts when set to `true`. Intended for the
/// CLI's `--yes` flag and for non-interactive test runs.
pub fn set_assume_yes(assume_yes: bool) {
    ASSUME_YES.store(assume_yes, Ordering::Relaxed);
}

/// Lists `(name, version)` pairs with `indent` leading spaces per package and
/// asks for confirmation using `action` as the verb, defaulting to yes.
///
/// An empty list returns `true` without output or a prompt. When
/// [`set_assume_yes`] is enabled, a nonempty list is still printed but returns
/// `true` without prompting. Declining prints an abort message and returns `false`.
/// Prompt errors, including cancellation or interruption, are replaced with
/// `Invalid answer try again`.
pub fn confirmation_prompt(
    packages: Vec<(&str, &str)>,
    indent: usize,
    action: &str,
) -> Result<bool> {
    if packages.is_empty() {
        return Ok(true);
    }

    println!("Packages:");
    for (name, version) in packages {
        println!("{:indent$}{}-{}", "", name, version, indent = indent);
    }

    if ASSUME_YES.load(Ordering::Relaxed) {
        return Ok(true);
    }

    let ans = Confirm::new(&format!(
        "This will {} these packages, do you wish to proceed?",
        action
    ))
    .with_default(true)
    .prompt();

    match ans {
        Ok(true) => return Ok(true),
        Ok(false) => {
            println!("Transaction aborted");
            return Ok(false);
        }
        Err(_) => return Err(anyhow!("Invalid answer try again")),
    }
}
