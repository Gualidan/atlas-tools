use anyhow::Result;
use atlas::install::{
    db::ensure_install_schema,
    resolve::{ResolvedPackage, resolve_install_order},
};
use common::{db::db::create_packages_table, types::package::Architectures};
use rusqlite::{Connection, params};

const ARCH: Architectures = Architectures::X86_64;

struct Databases {
    repo: Connection,
    installed: Connection,
}

impl Databases {
    fn new(packages: &[(&str, &[&str])]) -> Result<Self> {
        let mut repo = Connection::open_in_memory()?;
        create_packages_table(&mut repo)?;
        let installed = Connection::open_in_memory()?;
        ensure_install_schema(&installed)?;
        for (name, deps) in packages {
            repo.execute(
                "INSERT INTO packages
                 (name, version, release, architecture, sky_path, recipe_sha256, built_at, builder, deps, makedeps)
                 VALUES (?, '2.0.0', 7, ?, ?, 'hash', '2026-01-01', 'tester', ?, '[]')",
                params![
                    name,
                    ARCH.to_string(),
                    format!("/repo/{name}.sky"),
                    serde_saphyr::to_string(deps)?
                ],
            )?;
        }
        Ok(Self { repo, installed })
    }

    fn install(&self, name: &str, version: &str, reason: &str) -> Result<()> {
        self.installed.execute(
            "INSERT INTO installed_packages
             (name, version, release, architecture, sky_path, install_dir, install_reason)
             VALUES (?, ?, 1, ?, 'old.sky', ?, ?)",
            params![
                name,
                version,
                ARCH.to_string(),
                format!("/installed/{name}"),
                reason
            ],
        )?;
        Ok(())
    }

    fn resolve(&self, name: &str) -> Result<Vec<ResolvedPackage>> {
        resolve_install_order(&self.repo, &self.installed, name.into(), ARCH)
    }

    fn names(&self, name: &str) -> Result<Vec<String>> {
        Ok(self
            .resolve(name)?
            .into_iter()
            .map(|pkg| pkg.name)
            .collect())
    }
}

#[test]
fn installed_dependencies_are_skipped_regardless_of_version_or_install_reason() -> Result<()> {
    for version in ["1.0.0", "2.0.0", "3.0.0"] {
        for reason in ["explicit", "dependency"] {
            let db = Databases::new(&[("app", &["library"]), ("library", &[])])?;
            db.install("library", version, reason)?;
            let installed_changes = db.installed.total_changes();
            let repo_changes = db.repo.total_changes();

            let plan = db.resolve("app")?;

            assert_eq!(plan.len(), 1, "installed {version}, reason {reason}");
            let package = &plan[0];
            assert_eq!(package.name, "app");
            assert_eq!(package.version, "2.0.0");
            assert_eq!(package.release, 7);
            assert_eq!(package.architecture, ARCH);
            assert_eq!(package.sky_path, "/repo/app.sky");
            assert_eq!(db.installed.total_changes(), installed_changes);
            assert_eq!(db.repo.total_changes(), repo_changes);
        }
    }
    Ok(())
}

#[test]
fn installed_dependency_subtrees_are_not_traversed() -> Result<()> {
    for deps in ["[missing]", "[library]", "invalid: [yaml"] {
        let db = Databases::new(&[("app", &["library"]), ("library", &[])])?;
        db.install("library", "1.0.0", "dependency")?;
        db.repo.execute(
            "UPDATE packages SET deps = ? WHERE name = 'library'",
            [deps],
        )?;

        assert_eq!(db.names("app")?, ["app"], "{deps}");
    }
    Ok(())
}

#[test]
fn an_installed_transitive_dependency_is_pruned_from_every_branch() -> Result<()> {
    let db = Databases::new(&[
        ("app", &["left", "right", "library"]),
        ("left", &["library", "new-leaf"]),
        ("right", &["library", "new-leaf"]),
        ("library", &["unused-leaf"]),
        ("unused-leaf", &[]),
        ("new-leaf", &[]),
    ])?;
    db.install("library", "1.0.0", "dependency")?;

    // Keep dependency order and deduplicate the remaining shared dependency.
    assert_eq!(db.names("app")?, ["new-leaf", "left", "right", "app"]);
    Ok(())
}

#[test]
fn an_explicitly_requested_installed_root_is_still_resolved() -> Result<()> {
    let db = Databases::new(&[("app", &["library"]), ("library", &[])])?;
    db.install("app", "1.0.0", "explicit")?;

    assert_eq!(db.names("app")?, ["library", "app"]);
    db.install("library", "1.0.0", "dependency")?;
    assert_eq!(db.names("app")?, ["app"]);
    Ok(())
}

#[test]
fn pruning_an_installed_sibling_does_not_hide_a_missing_dependency() -> Result<()> {
    let db = Databases::new(&[("app", &["installed", "missing"]), ("installed", &[])])?;
    db.install("installed", "1.0.0", "dependency")?;

    let err = db
        .resolve("app")
        .expect_err("the uninstalled dependency is required");

    assert_eq!(
        err.to_string(),
        "Dependency `missing` of package `app` not found"
    );
    Ok(())
}

#[test]
fn pruning_an_installed_sibling_does_not_hide_an_uninstalled_cycle() -> Result<()> {
    let db = Databases::new(&[
        ("app", &["installed", "left"]),
        ("installed", &[]),
        ("left", &["right"]),
        ("right", &["left"]),
    ])?;
    db.install("installed", "1.0.0", "dependency")?;

    let err = db
        .resolve("app")
        .expect_err("the remaining cycle must be rejected");

    assert_eq!(err.to_string(), "dependency cycle: left -> right -> left");
    Ok(())
}

#[test]
fn quoted_yaml_dependencies_are_decoded_before_installed_lookup() -> Result<()> {
    let db = Databases::new(&[("app", &[]), ("library", &[])])?;
    // Escaped YAML scalars require an owned String during deserialization.
    db.repo.execute(
        "UPDATE packages SET deps = ? WHERE name = 'app'",
        [r#"["\u006cibrary"]"#],
    )?;

    assert_eq!(db.names("app")?, ["library", "app"]);
    db.install("library", "1.0.0", "dependency")?;
    assert_eq!(db.names("app")?, ["app"]);
    Ok(())
}

#[test]
fn an_unreadable_install_ledger_is_an_error_instead_of_reinstalling_dependencies() -> Result<()> {
    let db = Databases::new(&[("app", &["library"]), ("library", &[])])?;
    db.installed.execute("DROP TABLE installed_packages", [])?;

    let err = db
        .resolve("app")
        .expect_err("installed state must be readable");

    assert_eq!(err.to_string(), "Failed to prepare database query");
    assert!(format!("{err:#}").contains("no such table: installed_packages"));
    Ok(())
}
