use std::{env::consts::ARCH, path::PathBuf};

use anyhow::{Context, Result};
use common::types::package::Architectures;
use rusqlite::{Connection, params};

use crate::{
    cli::confirmation::{Action, confirmation_prompt},
    install::db::ensure_install_schema,
    types::{orphan_packages::OrphanPackage, runtime_config::RuntimeConfig},
    uninstall::uninstall::uninstall,
};

/// Removes installed packages for the host architecture that are not reachable
/// from explicitly installed packages through recorded dependencies.
///
/// Requests confirmation once unless `set_assume_yes` is enabled, then removes
/// packages without individual prompts. An empty list or a declined prompt
/// succeeds without removing packages. Propagates lookup, confirmation, and
/// uninstall errors; stops at the first failure, retaining earlier removals.
pub fn uninstall_orphans(runtime_config: &RuntimeConfig) -> Result<()> {
    let orphans = get_orphaned_packages(runtime_config)?;

    if orphans.is_empty() {
        println!("No orphaned packages to remove");
        return Ok(());
    }

    let packages: Vec<(&str, &str)> = orphans
        .iter()
        .map(|orphan| (orphan.name.as_str(), orphan.version.as_str()))
        .collect();

    if !confirmation_prompt(packages, 4, &Action::Uninstall)? {
        return Ok(());
    }

    for orphan in orphans {
        println!("Uninstalling {}", orphan.name);
        uninstall(runtime_config, &orphan.name, false)?;
    }

    Ok(())
}

pub fn get_orphaned_packages(runtime_config: &RuntimeConfig) -> Result<Vec<OrphanPackage>> {
    let conn = Connection::open(&runtime_config.install_db)
        .context("Failed to open connection to install database")?;
    ensure_install_schema(&conn)?;

    let mut stmt = conn
        .prepare(
            "
            WITH RECURSIVE keep(name, architecture) AS (
                SELECT name, architecture
                FROM installed_packages
                WHERE install_reason = 'explicit'

                UNION

                SELECT pd.dependency_name, pd.architecture
                FROM package_dependencies pd
                JOIN keep k ON pd.package_name = k.name AND pd.architecture = k.architecture
            )
            SELECT p.name, p.architecture, p.install_dir, p.version, p.release
            FROM installed_packages p
            WHERE p.architecture = ?
            AND NOT EXISTS (
                SELECT 1 FROM keep k
                WHERE k.name = p.name AND k.architecture = p.architecture
            );
",
        )
        .context("Failed to prepare database query")?;
    let orphans: Vec<OrphanPackage> = stmt
        .query_map(params![ARCH], |row| {
            Ok(OrphanPackage {
                name: row.get::<_, String>(0)?,
                architecture: Architectures::try_from(row.get::<_, String>(1)?).unwrap(),
                install_dir: PathBuf::from(row.get::<_, String>(2)?),
                version: row.get::<_, String>(3)?,
                release: row.get::<_, u32>(4)?,
            })
        })
        .context("Failed to list orphaned packages")?
        .collect::<Result<Vec<OrphanPackage>, _>>()?;

    Ok(orphans)
}
