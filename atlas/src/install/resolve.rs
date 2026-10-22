use std::collections::HashSet;

use anyhow::{Context, Result, anyhow};
use common::types::package::Architectures;
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Debug)]
pub struct ResolvedPackage {
    pub name: String,
    pub version: String,
    pub release: u32,
    pub architecture: Architectures,
    pub sky_path: String,
}

pub fn resolve_install_order(
    conn: &Connection,
    name: String,
    arch: Architectures,
) -> Result<Vec<ResolvedPackage>> {
    let mut stack: Vec<String> = Vec::new();
    let mut resolved: HashSet<String> = HashSet::new();
    let mut resolved_packages: Vec<ResolvedPackage> = Vec::new();

    visit(
        conn,
        name,
        &mut stack,
        &mut resolved,
        &mut resolved_packages,
        arch,
    )?;

    Ok(resolved_packages)
}

pub fn visit<'a>(
    conn: &Connection,
    name: String,
    stack: &mut Vec<String>,
    resolved: &mut HashSet<String>,
    resolved_packages: &'a mut Vec<ResolvedPackage>,
    arch: Architectures,
) -> Result<()> {
    if resolved.contains(&name) {
        return Ok(());
    }
    if stack.contains(&name) {
        return Err(anyhow!("Dependency cycle detected"));
    }

    let mut stmt  = conn.prepare("SELECT version, release, sky_path, deps FROM packages WHERE name = ? AND architecture = ?").context("Failed to prepare database query")?;
    let Some((version, release, sky_path, deps)): Option<(String, u32, String, String)> = stmt
        .query_row(params![name, arch.to_string()], |row| {
            let version: String = row.get(0)?;
            let release: u32 = row.get(1)?;
            let sky_path: String = row.get(2)?;
            let deps: String = row.get(3)?;

            Ok((version, release, sky_path, deps))
        })
        .optional()?
    else {
        return Err(anyhow!("Package {name} not found"));
    };

    let deps: Vec<&str> = serde_saphyr::from_str(&deps).context("Failed to deserialize YAML")?;

    stack.push(name.to_string());

    for dep in deps {
        visit(
            conn,
            dep.to_string(),
            stack,
            resolved,
            resolved_packages,
            arch,
        )?;
    }

    stack.pop();
    resolved.insert(name.to_string());
    resolved_packages.push(ResolvedPackage {
        name,
        version,
        release,
        architecture: arch,
        sky_path,
    });

    Ok(())
}
