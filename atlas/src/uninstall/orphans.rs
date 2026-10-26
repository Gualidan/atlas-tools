use std::{env::consts::ARCH, path::PathBuf};

use anyhow::{Context, Result};
use common::types::package::Architectures;
use rusqlite::{Connection, params};

use crate::{
    install::{db::ensure_install_schema, install::print_packages_with_indent},
    types::{orphan_packages::OrphanPackage, runtime_config::RuntimeConfig},
    uninstall::uninstall::uninstall,
};

pub fn uninstall_orphans(runtime_config: &RuntimeConfig) -> Result<()> {
    let orphans = get_orphaned_packages(runtime_config)?;

    let packages: Vec<(&str, &str)> = orphans
        .iter()
        .map(|orphan| (orphan.name.as_str(), orphan.version.as_str()))
        .collect();

    print_packages_with_indent(packages, 4);

    for orphan in orphans {
        println!("Uninstalling {}", orphan.name);
        uninstall(runtime_config, &orphan.name)?;
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
