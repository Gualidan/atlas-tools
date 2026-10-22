use std::collections::HashSet;

use anyhow::{Context, Result, anyhow};
use common::types::package::Architectures;
use rusqlite::{Connection, OptionalExtension, params};

pub struct ResolvedPackage {
    name: String,
    version: String,
    release: u32,
    sky_path: String,
}

pub struct ResolveState {
    pub resolved: HashSet<String>,
    stack: Vec<String>,
    resolved_packages: Vec<ResolvedPackage>,
}
pub fn resolve_install_order(
    conn: &Connection,
    name: String,
    arch: Architectures,
) -> Result<Vec<ResolvedPackage>> {
    let mut resolve_state = ResolveState {
        resolved: HashSet::new(),
        stack: Vec::new(),
        resolved_packages: Vec::new(),
    };

    let mut stmt = conn
        .prepare("SELECT deps FROM packages WHERE name = ? AND architecture = ?")
        .context("Failed to prepare database statement")?;
    let Some(dep): Option<String> = stmt
        .query_row(params![name, arch.to_string()], |row| row.get(0))
        .optional()?
    else {
        return Err(anyhow!("Package {name} not found"));
    };

    let deps: Vec<&str> = dep.split("-").collect();

    for dep in deps {
        if resolve_state.resolved.contains(dep) {
            return Ok(resolve_state.resolved_packages);
        }
        if resolve_state.stack.contains(&dep.to_string()) {
            return Err(anyhow!("Dependency cycle detected"));
        }

        let mut stmt  = conn.prepare("SELECT version, release, sky_path, deps FROM packages WHERE name = ? AND architecture = ?").context("Failed to prepare database query")?;
        let Some((version, release, sky_path, architecture)): Option<(
            String,
            u32,
            String,
            String,
        )> = stmt
            .query_row(params![dep, arch.to_string()], |row| {
                let version: String = row.get(0)?;
                let release: u32 = row.get(1)?;
                let sky_path: String = row.get(2)?;
                let architecture: String = row.get(3)?;

                Ok((version, release, sky_path, architecture))
            })
            .optional()?
        else {
            return Err(anyhow!("Dependency {dep} not found"));
        };

        resolve_state.stack.push(dep.to_string());
    }

    Ok(resolve_state.resolved_packages)
}
