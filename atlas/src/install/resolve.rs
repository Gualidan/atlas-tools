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
        let start = stack.iter().position(|entry| entry == &name).unwrap_or(0);
        let mut cycle = stack[start..].to_vec();
        cycle.push(name.clone());
        return Err(anyhow!("dependency cycle: {}", cycle.join(" -> ")));
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
        return Err(match stack.last() {
            Some(parent) => anyhow!("Dependency `{name}` of package `{parent}` not found"),
            None => anyhow!("Package {name} not found"),
        });
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

#[cfg(test)]
mod tests {
    use super::*;
    use common::db::db::create_packages_table;

    const ARCH: Architectures = Architectures::X86_64;

    fn seed(conn: &mut Connection, packages: &[(&str, &[&str])]) {
        create_packages_table(conn).unwrap();

        for (name, deps) in packages {
            let deps: Vec<String> = deps.iter().map(|dep| dep.to_string()).collect();
            let deps = serde_saphyr::to_string(&deps).expect("failed to serialize deps to YAML");

            conn.execute(
                "INSERT INTO packages (name, version, release, architecture, sky_path, recipe_sha256, built_at, builder, deps, makedeps) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    *name,
                    "1.0.0",
                    1,
                    ARCH.to_string(),
                    format!("{name}.sky"),
                    "deadbeef",
                    "2026-01-01T00:00:00Z",
                    "tester",
                    deps,
                    "[]"
                ],
            )
            .unwrap();
        }
    }

    fn resolve_names(conn: &Connection, name: &str) -> Result<Vec<String>> {
        let packages = resolve_install_order(conn, name.to_string(), ARCH)?;
        Ok(packages.into_iter().map(|package| package.name).collect())
    }

    #[test]
    fn resolves_simple_dependency() {
        let mut conn = Connection::open_in_memory().unwrap();
        seed(&mut conn, &[("foo", &["bar"]), ("bar", &[])]);

        let names = resolve_names(&conn, "foo").unwrap();

        assert_eq!(names, ["bar", "foo"]);
    }

    #[test]
    fn resolves_transitive_chain() {
        let mut conn = Connection::open_in_memory().unwrap();
        seed(
            &mut conn,
            &[("foo", &["bar"]), ("bar", &["baz"]), ("baz", &[])],
        );

        let names = resolve_names(&conn, "foo").unwrap();

        assert_eq!(names, ["baz", "bar", "foo"]);
    }

    #[test]
    fn diamond_installs_shared_dep_once() {
        let mut conn = Connection::open_in_memory().unwrap();
        seed(
            &mut conn,
            &[("foo", &["bar", "baz"]), ("bar", &["baz"]), ("baz", &[])],
        );

        let names = resolve_names(&conn, "foo").unwrap();

        assert_eq!(names, ["baz", "bar", "foo"]);
        assert_eq!(
            names.iter().filter(|name| *name == "baz").count(),
            1,
            "a shared dependency must be resolved exactly once"
        );
    }

    #[test]
    fn cycles_are_named() {
        let mut conn = Connection::open_in_memory().unwrap();
        seed(&mut conn, &[("foo", &["bar"]), ("bar", &["foo"])]);

        let err = resolve_names(&conn, "foo").expect_err("the cycle must be rejected");
        let chain = format!("{err:#}");

        assert!(
            chain.contains("dependency cycle: foo -> bar -> foo"),
            "error chain was: {chain}"
        );
    }

    #[test]
    fn self_dependency_is_a_cycle() {
        let mut conn = Connection::open_in_memory().unwrap();
        seed(&mut conn, &[("foo", &["foo"])]);

        let err = resolve_names(&conn, "foo").expect_err("the self-cycle must be rejected");
        let chain = format!("{err:#}");

        assert!(
            chain.contains("dependency cycle: foo -> foo"),
            "error chain was: {chain}"
        );
    }

    #[test]
    fn missing_dep_names_parent_and_ghost() {
        let mut conn = Connection::open_in_memory().unwrap();
        seed(&mut conn, &[("foo", &["ghost"])]);

        let err = resolve_names(&conn, "foo").expect_err("the missing dependency must be rejected");
        let chain = format!("{err:#}");

        assert!(chain.contains("foo"), "error chain was: {chain}");
        assert!(chain.contains("ghost"), "error chain was: {chain}");
    }

    #[test]
    fn missing_root_keeps_message() {
        let mut conn = Connection::open_in_memory().unwrap();
        seed(&mut conn, &[]);

        let err = resolve_names(&conn, "nope").expect_err("the missing root must be rejected");

        assert_eq!(format!("{err}"), "Package nope not found");
    }
}
