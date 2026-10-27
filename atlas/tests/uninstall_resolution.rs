use std::{
    collections::BTreeSet,
    env,
    io::{Read, Seek, SeekFrom},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use atlas::{install::db::ensure_install_schema, uninstall::resolve::packages_to_uninstall};
use common::types::package::{Architectures, PackageIdentity};
use rusqlite::{Connection, params};

struct Database(Connection);

impl Database {
    fn new(packages: &[(&str, &[&str])]) -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        ensure_install_schema(&conn)?;
        // Supply the serialized column read by the resolver to isolate its
        // traversal and decoding. The production-schema regression below
        // deliberately omits this addition and must also pass.
        let has_deps: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM pragma_table_info('installed_packages') WHERE name = 'deps')",
            [],
            |row| row.get(0),
        )?;
        if !has_deps {
            conn.execute(
                "ALTER TABLE installed_packages ADD COLUMN deps TEXT NOT NULL DEFAULT '[]'",
                [],
            )?;
        }
        conn.execute_batch("PRAGMA foreign_keys = ON")?;
        for (name, deps) in packages {
            conn.execute(
                "INSERT INTO installed_packages
                 (name, version, release, architecture, sky_path, install_dir, install_reason, deps)
                 VALUES (?, '2.1.0-rc.1', 7, 'x86_64', 'unused.sky', ?, 'dependency', ?)",
                params![
                    name,
                    format!("/installed/{name}"),
                    serde_saphyr::to_string(deps)?
                ],
            )?;
        }
        for (name, deps) in packages {
            for dep in *deps {
                conn.execute(
                    "INSERT INTO package_dependencies VALUES (?, 'x86_64', ?)",
                    params![name, dep],
                )?;
            }
        }
        Ok(Self(conn))
    }

    fn resolve(&self, name: &str) -> Result<Vec<(PackageIdentity, String)>> {
        packages_to_uninstall(&self.0, name.into())
    }

    fn assert_plan(&self, root: &str, expected: &[&str]) -> Result<()> {
        let changes_before = self.0.total_changes();
        let plan = self.resolve(root)?;
        let names: BTreeSet<_> = plan.iter().map(|(pkg, _)| pkg.name.as_str()).collect();
        assert_eq!(names, expected.iter().copied().collect());
        assert_eq!(plan.len(), expected.len(), "each package must appear once");
        for (package, install_dir) in &plan {
            assert_eq!(package.version, "2.1.0-rc.1");
            assert_eq!(package.release, 7);
            assert_eq!(package.architecture, Architectures::X86_64);
            assert_eq!(*install_dir, format!("/installed/{}", package.name));
        }
        assert_eq!(
            self.0.total_changes(),
            changes_before,
            "resolution is read only"
        );
        Ok(())
    }
}

struct TestChild(Child);

impl Drop for TestChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// Recursive regressions must fail individually even if traversal aborts on a
// stack overflow. Capture to a file to avoid a full pipe blocking the child.
fn isolated(test_name: &str, body: impl FnOnce() -> Result<()>) -> Result<()> {
    if env::var_os("ATLAS_RESOLUTION_TEST_CHILD").as_deref() == Some(test_name.as_ref()) {
        return body();
    }
    let mut log = tempfile::tempfile()?;
    let mut child = TestChild(
        Command::new(env::current_exe()?)
            .args(["--exact", test_name, "--nocapture", "--color", "never"])
            .env("ATLAS_RESOLUTION_TEST_CHILD", test_name)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log.try_clone()?)
            .spawn()?,
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.0.try_wait()? {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "{test_name}: traversal did not terminate"
        );
        thread::sleep(Duration::from_millis(5));
    };
    log.seek(SeekFrom::Start(0))?;
    let mut output = String::new();
    log.read_to_string(&mut output)?;
    assert!(status.success(), "{test_name}: {status}\n{output}");
    assert!(output.contains("1 passed; 0 failed;"), "{output}");
    Ok(())
}

#[test]
fn resolver_accepts_the_production_install_schema() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    ensure_install_schema(&conn)?;
    conn.execute(
        "INSERT INTO installed_packages
         (name, version, release, architecture, sky_path, install_dir, install_reason)
         VALUES ('app', '1.0.0', 1, 'x86_64', 'app.sky', '/installed/app', 'explicit')",
        [],
    )?;

    let plan = packages_to_uninstall(&conn, "app".into())?;

    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].0.name, "app");
    assert_eq!(plan[0].1, "/installed/app");
    Ok(())
}

#[test]
fn leaf_resolution_preserves_the_complete_identity_and_directory() -> Result<()> {
    let db = Database::new(&[("app", &[]), ("unrelated", &[])])?;
    db.assert_plan("app", &["app"])
}

#[test]
fn leaf_resolution_supports_both_architectures_and_release_boundaries() -> Result<()> {
    for (architecture, expected) in [
        ("x86_64", Architectures::X86_64),
        ("arm", Architectures::ARM),
    ] {
        for release in [0, u32::MAX] {
            let db = Database::new(&[("app", &[])])?;
            db.0.execute(
                "UPDATE installed_packages SET architecture = ?, release = ?",
                params![architecture, release],
            )?;
            let plan = db.resolve("app")?;
            assert_eq!(plan.len(), 1);
            assert_eq!(plan[0].0.architecture, expected);
            assert_eq!(plan[0].0.release, release);
        }
    }
    Ok(())
}

#[test]
fn package_names_are_bound_as_data() -> Result<()> {
    let name = "app' OR 1=1 --";
    let db = Database::new(&[(name, &[]), ("unrelated", &[])])?;
    db.assert_plan(name, &[name])
}

#[test]
fn a_direct_dependent_blocks_resolution_without_writes() -> Result<()> {
    let db = Database::new(&[("app", &["library"]), ("library", &[])])?;
    let changes_before = db.0.total_changes();

    let err = db
        .resolve("library")
        .expect_err("app still requires library");

    let message = err.to_string();
    assert!(message.contains("library"), "{message}");
    assert!(message.contains("app depends on it"), "{message}");
    assert_eq!(db.0.total_changes(), changes_before);
    Ok(())
}

#[test]
fn a_package_and_its_exclusive_dependency_are_resolved() -> Result<()> {
    isolated(
        "a_package_and_its_exclusive_dependency_are_resolved",
        || {
            Database::new(&[("app", &["library"]), ("library", &[])])?
                .assert_plan("app", &["app", "library"])
        },
    )
}

#[test]
fn every_sibling_and_transitive_dependency_is_resolved_once() -> Result<()> {
    isolated(
        "every_sibling_and_transitive_dependency_is_resolved_once",
        || {
            Database::new(&[
                ("app", &["left", "right"]),
                ("left", &["leaf"]),
                ("right", &[]),
                ("leaf", &[]),
                ("unrelated", &[]),
            ])?
            .assert_plan("app", &["app", "left", "right", "leaf"])
        },
    )
}

#[test]
fn an_external_dependent_of_a_transitive_dependency_blocks_the_plan() -> Result<()> {
    isolated(
        "an_external_dependent_of_a_transitive_dependency_blocks_the_plan",
        || {
            let db = Database::new(&[
                ("app", &["middle"]),
                ("middle", &["shared"]),
                ("shared", &[]),
                ("neighbor", &["shared"]),
            ])?;
            let changes_before = db.0.total_changes();
            let err = db.resolve("app").expect_err("neighbor still needs shared");
            let message = err.to_string();
            assert!(message.contains("shared"), "{message}");
            assert!(message.contains("neighbor"), "{message}");
            assert_eq!(db.0.total_changes(), changes_before);
            Ok(())
        },
    )
}

#[test]
fn a_missing_dependency_is_reported_instead_of_recursing_forever() -> Result<()> {
    isolated(
        "a_missing_dependency_is_reported_instead_of_recursing_forever",
        || {
            let db = Database::new(&[("app", &[])])?;
            db.0.execute("UPDATE installed_packages SET deps = '[missing]'", [])?;
            assert!(
                db.resolve("app").is_err(),
                "a broken ledger must fail resolution"
            );
            Ok(())
        },
    )
}

#[test]
fn a_cycle_is_rejected_without_recursing_forever() -> Result<()> {
    isolated("a_cycle_is_rejected_without_recursing_forever", || {
        let db = Database::new(&[
            ("app", &["left"]),
            ("left", &["right"]),
            ("right", &["left"]),
        ])?;
        assert!(
            db.resolve("app").is_err(),
            "a dependency cycle cannot be removed safely"
        );
        Ok(())
    })
}

#[test]
fn malformed_dependency_yaml_is_reported_with_context() -> Result<()> {
    for deps in ["[unfinished", "not: a-list"] {
        let db = Database::new(&[("app", &[])])?;
        db.0.execute("UPDATE installed_packages SET deps = ?", [deps])?;
        let err = db
            .resolve("app")
            .expect_err("invalid dependencies must be rejected");
        assert!(err.to_string().contains("Failed to deserialize"), "{err:#}");
    }
    Ok(())
}

#[test]
fn unsupported_architectures_are_rejected() -> Result<()> {
    let db = Database::new(&[("app", &[])])?;
    db.0.execute(
        "UPDATE installed_packages SET architecture = 'unsupported'",
        [],
    )?;
    let err = db
        .resolve("app")
        .expect_err("unknown architecture must not produce an identity");
    assert!(
        err.to_string().contains("Unsupported architecture"),
        "{err:#}"
    );
    Ok(())
}

#[test]
fn invalid_release_numbers_are_rejected() -> Result<()> {
    for release in [-1_i64, i64::from(u32::MAX) + 1] {
        let db = Database::new(&[("app", &[])])?;
        db.0.execute("UPDATE installed_packages SET release = ?", [release])?;
        let err = db.resolve("app").expect_err("release must fit in u32");
        assert!(format!("{err:#}").contains("out of range"), "{err:#}");
    }
    Ok(())
}

#[test]
fn an_absent_package_returns_a_lookup_error() -> Result<()> {
    let db = Database::new(&[])?;
    let err = db
        .resolve("missing")
        .expect_err("an absent package must fail");
    assert!(matches!(
        err.downcast_ref::<rusqlite::Error>(),
        Some(rusqlite::Error::QueryReturnedNoRows)
    ));
    Ok(())
}

#[test]
fn missing_install_tables_propagate_database_errors() -> Result<()> {
    for table in ["package_dependencies", "installed_packages"] {
        let db = Database::new(&[])?;
        db.0.execute(&format!("DROP TABLE {table}"), [])?;
        let err = db
            .resolve("app")
            .expect_err("unreadable ledger must not produce a plan");
        assert_eq!(err.to_string(), "Failed to prepare database query");
        assert!(
            format!("{err:#}").contains(&format!("no such table: {table}")),
            "{err:#}"
        );
    }
    Ok(())
}
