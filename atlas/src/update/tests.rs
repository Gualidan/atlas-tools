use super::*;
use crate::install::db::ensure_install_schema;
use common::db::db::create_packages_table;

struct Databases {
    installed: Connection,
    repo: Connection,
}

impl Databases {
    fn new() -> Result<Self> {
        let installed = Connection::open_in_memory()?;
        ensure_install_schema(&installed)?;
        let mut repo = Connection::open_in_memory()?;
        create_packages_table(&mut repo)?;
        Ok(Self { installed, repo })
    }

    fn install(&self, name: &str, version: &str, architecture: &str) -> Result<()> {
        self.installed.execute(
            "INSERT INTO installed_packages
             (name, version, release, architecture, sky_path, install_dir, install_reason)
             VALUES (?, ?, 1, ?, 'old.sky', ?, 'explicit')",
            params![name, version, architecture, format!("/installed/{name}")],
        )?;
        Ok(())
    }

    fn publish(&self, name: &str, version: &str, architecture: &str) -> Result<()> {
        self.repo.execute(
            "INSERT INTO packages
             (name, version, release, architecture, sky_path, recipe_sha256, built_at, builder, deps, makedeps)
             VALUES (?, ?, 7, ?, ?, 'hash', '2026-01-01', 'tester', '[]', '[]')",
            params![name, version, architecture, format!("/repo/{name}-{architecture}.sky")],
        )?;
        Ok(())
    }

    fn resolve(&self, name: Option<&str>) -> Result<Resolution> {
        resolve_update_order(&self.installed, &self.repo, name)
    }
}

#[test]
fn single_candidate_uses_recorded_directory_and_repository_target() -> Result<()> {
    let db = Databases::new()?;
    db.install("foo", "1.0.0", ARCH)?;
    db.install("other", "1.0.0", ARCH)?;
    db.publish("foo", "2.0.0", ARCH)?;
    // Repository dependency metadata must not turn an update into an install.
    db.repo
        .execute("UPDATE packages SET deps = 'invalid: [yaml'", [])?;

    let resolution = db.resolve(Some("foo"))?;

    assert!(resolution.error.is_none());
    assert_eq!(resolution.candidates.len(), 1);
    let candidate = &resolution.candidates[0];
    assert_eq!(candidate.name, "foo");
    assert_eq!(candidate.version, "2.0.0");
    assert_eq!(candidate.release, 7);
    assert_eq!(candidate.install_dir, "/installed/foo");
    assert_eq!(candidate.sky_path, format!("/repo/foo-{ARCH}.sky"));
    // Resolution must leave the installed row untouched.
    let version: String = db.installed.query_row(
        "SELECT version FROM installed_packages WHERE name = 'foo'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(version, "1.0.0");
    Ok(())
}

#[test]
fn equal_versions_produce_no_single_or_bulk_candidates_despite_release_changes() -> Result<()> {
    let db = Databases::new()?;
    db.install("foo", "1.0.0", ARCH)?;
    db.publish("foo", "1.0.0", ARCH)?;
    for name in [Some("foo"), None] {
        let resolution = db.resolve(name)?;
        assert!(resolution.candidates.is_empty());
        assert!(resolution.error.is_none());
    }
    Ok(())
}

#[test]
fn bulk_resolution_sorts_real_upgrades_and_ignores_uninstalled_packages() -> Result<()> {
    let db = Databases::new()?;
    for name in ["zeta", "middle", "alpha"] {
        db.install(name, "1.0.0", ARCH)?;
        db.publish(name, if name == "middle" { "1.0.0" } else { "2.0.0" }, ARCH)?;
    }
    db.publish("uninstalled", "9.0.0", ARCH)?;

    let resolution = db.resolve(None)?;

    assert!(resolution.error.is_none());
    let names: Vec<_> = resolution
        .candidates
        .iter()
        .map(|row| row.name.as_str())
        .collect();
    assert_eq!(names, ["alpha", "zeta"]);
    for candidate in resolution.candidates {
        assert_eq!(candidate.version, "2.0.0");
        assert_eq!(candidate.release, 7);
        assert_eq!(
            candidate.install_dir,
            format!("/installed/{}", candidate.name)
        );
        assert_eq!(
            candidate.sky_path,
            format!("/repo/{}-{ARCH}.sky", candidate.name)
        );
    }
    Ok(())
}

#[test]
fn empty_installation_does_not_select_repository_packages() -> Result<()> {
    let db = Databases::new()?;
    db.publish("foo", "2.0.0", ARCH)?;
    let resolution = db.resolve(None)?;
    assert!(resolution.candidates.is_empty());
    assert!(resolution.error.is_none());
    Ok(())
}

#[test]
fn missing_or_foreign_installed_package_is_rejected_before_repository_lookup() -> Result<()> {
    for installed_arch in [None, Some("foreign")] {
        let db = Databases::new()?;
        if let Some(arch) = installed_arch {
            db.install("foo", "1.0.0", arch)?;
        }
        db.publish("foo", "2.0.0", ARCH)?;
        let err = db
            .resolve(Some("foo"))
            .err()
            .expect("host package must be installed");
        assert_eq!(
            err.to_string(),
            "Package foo not found in installed packages"
        );
    }
    Ok(())
}

#[test]
fn a_repository_row_for_another_architecture_cannot_supply_an_update() -> Result<()> {
    let db = Databases::new()?;
    db.install("foo", "1.0.0", ARCH)?;
    db.publish("foo", "2.0.0", "foreign")?;
    let err = db
        .resolve(Some("foo"))
        .err()
        .expect("host repository row is required");
    assert_eq!(err.to_string(), "Package foo not found");
    Ok(())
}

#[test]
fn bulk_lookup_keeps_using_the_host_repository_architecture() -> Result<()> {
    let db = Databases::new()?;
    db.install("foo", "1.0.0", "foreign")?;
    db.publish("foo", "2.0.0", ARCH)?;
    db.publish("foo", "9.0.0", "foreign")?;
    let resolution = db.resolve(None)?;
    assert!(resolution.error.is_none());
    assert_eq!(resolution.candidates.len(), 1);
    assert_eq!(resolution.candidates[0].version, "2.0.0");
    Ok(())
}

#[test]
fn single_downgrade_error_identifies_the_package_and_both_versions() -> Result<()> {
    let db = Databases::new()?;
    db.install("foo", "2.0.0", ARCH)?;
    db.publish("foo", "1.0.0", ARCH)?;
    let err = db.resolve(Some("foo")).err().expect("downgrade must fail");
    assert_eq!(
        err.to_string(),
        "Not allowed to downgrade foo: installed version is 2.0.0 while the repository's is 1.0.0"
    );
    Ok(())
}

#[test]
fn bulk_errors_retain_only_candidates_resolved_before_the_failure() -> Result<()> {
    for failure in [
        "missing",
        "downgrade",
        "invalid-installed",
        "invalid-repo",
        "invalid-release",
    ] {
        let db = Databases::new()?;
        for name in ["zeta", "middle", "alpha"] {
            db.install(name, "1.0.0", ARCH)?;
            db.publish(name, "2.0.0", ARCH)?;
        }
        let expected = match failure {
            "missing" => {
                db.repo
                    .execute("DELETE FROM packages WHERE name = 'middle'", [])?;
                "Package middle not found"
            }
            "downgrade" => {
                db.repo.execute(
                    "UPDATE packages SET version = '0.9.0' WHERE name = 'middle'",
                    [],
                )?;
                "Not allowed to downgrade middle"
            }
            "invalid-installed" => {
                db.installed.execute(
                    "UPDATE installed_packages SET version = 'bad' WHERE name = 'middle'",
                    [],
                )?;
                "Failed to parse version"
            }
            "invalid-repo" => {
                db.repo.execute(
                    "UPDATE packages SET version = 'bad' WHERE name = 'middle'",
                    [],
                )?;
                "Failed to parse version"
            }
            "invalid-release" => {
                db.repo
                    .execute("UPDATE packages SET release = -1 WHERE name = 'middle'", [])?;
                "out of range"
            }
            _ => unreachable!(),
        };

        let resolution = db.resolve(None)?;

        assert_eq!(resolution.candidates.len(), 1, "{failure}");
        assert_eq!(resolution.candidates[0].name, "alpha", "{failure}");
        let err = resolution.error.expect("failure must be deferred");
        assert!(format!("{err:#}").contains(expected), "{failure}: {err:#}");
    }
    Ok(())
}

#[test]
fn invalid_single_versions_keep_the_parse_context() -> Result<()> {
    for (installed, available) in [("bad", "2.0.0"), ("1.0.0", "bad")] {
        let db = Databases::new()?;
        db.install("foo", installed, ARCH)?;
        db.publish("foo", available, ARCH)?;
        let err = db
            .resolve(Some("foo"))
            .err()
            .expect("invalid semver must fail");
        assert_eq!(err.to_string(), "Failed to parse version number");
        assert!(
            err.source().is_some(),
            "the parser error should be retained"
        );
    }
    Ok(())
}

#[test]
fn semantic_version_comparison_handles_numeric_and_prerelease_boundaries() -> Result<()> {
    for (installed, available, expected) in [
        ("1.9.0", "1.10.0", Less),
        ("2.0.0-rc.9", "2.0.0-rc.10", Less),
        ("2.0.0-rc.1", "2.0.0", Less),
        ("2.0.0", "2.0.0-rc.1", Greater),
        ("2.0.0", "2.0.0", Equal),
    ] {
        assert_eq!(
            compare_versions(installed, available, "parse failed")?,
            expected,
            "{installed} versus {available}"
        );
    }
    Ok(())
}
