mod fixtures;

use anyhow::Result;
use atlas::install::db::insert_installed_package;
use common::types::{
    metadata::{Build, Metadata},
    package::{PackageIdentity, Source},
};
use fixtures::{ARCH, generate_keypair, runtime_config, seed_repo_db, tempdir, write_signed_sky};
use rusqlite::Connection;

fn package_identity(name: &str, deps: &[&str]) -> (PackageIdentity, Vec<String>) {
    let identity = PackageIdentity {
        name: name.to_string(),
        version: "1.0.0".to_string(),
        release: 1,
        architecture: ARCH,
    };
    let deps: Vec<String> = deps.iter().map(|dep| dep.to_string()).collect();
    (identity, deps)
}

fn metadata(identity: &PackageIdentity, deps: &[String]) -> Metadata {
    Metadata {
        format: 1,
        name: identity.name.clone(),
        version: identity.version.clone(),
        release: identity.release,
        architecture: identity.architecture,
        deps: deps.to_vec(),
        source: Source {
            url: "https://example.invalid/src.tar.gz".to_string(),
            sha256: None,
        },
        build: Build {
            recipe_sha256: "deadbeef".to_string(),
            built_at: "2026-01-01T00:00:00Z".to_string(),
            builder: "tester".to_string(),
        },
    }
}

fn count(conn: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> i64 {
    conn.query_row(sql, params, |row| row.get(0)).unwrap()
}

#[test]
fn insert_installed_package_records_root_deps_and_edges() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, _pub_key) = generate_keypair()?;

    let root_sky = write_signed_sky(&tmp, &keypair, "root.sky", "root", "1.0.0", 1, b"root\n", vec![])?;
    let dep_a_sky = write_signed_sky(&tmp, &keypair, "dep-a.sky", "dep-a", "2.1.0", 3, b"dep-a\n", vec![])?;
    let dep_b_sky = write_signed_sky(&tmp, &keypair, "dep-b.sky", "dep-b", "0.4.0", 1, b"dep-b\n", vec![])?;

    seed_repo_db(
        &config.repo_db,
        &[
            ("root", "1.0.0", 1, root_sky.to_str().unwrap()),
            ("dep-a", "2.1.0", 3, dep_a_sky.to_str().unwrap()),
            ("dep-b", "0.4.0", 1, dep_b_sky.to_str().unwrap()),
        ],
    );

    let (package, deps) = package_identity("root", &["dep-a", "dep-b"]);

    insert_installed_package(
        &config,
        metadata(&package, &deps),
        &package,
        "/usr".to_string(),
    )
    .expect("first install should succeed");

    let conn = Connection::open(&config.install_db).unwrap();

    // Root row exists.
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM installed_packages WHERE name = ?1 AND architecture = ?2",
            &[&"root", &ARCH.to_string()],
        ),
        1,
        "root installed row missing"
    );

    // Dependency rows exist.
    for dep in ["dep-a", "dep-b"] {
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM installed_packages WHERE name = ?1 AND architecture = ?2",
                &[&dep, &ARCH.to_string()],
            ),
            1,
            "installed row for dependency `{dep}` missing"
        );
    }
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM installed_packages", &[]),
        3,
        "unexpected number of installed packages"
    );

    // Edge rows exist.
    for dep in ["dep-a", "dep-b"] {
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM package_dependencies WHERE package_name = ?1 AND dependency_name = ?2 AND architecture = ?3",
                &[&"root", &dep, &ARCH.to_string()],
            ),
            1,
            "edge root -> `{dep}` missing"
        );
    }
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM package_dependencies", &[]),
        2,
        "unexpected number of dependency edges"
    );

    // Idempotence: a second run must still succeed and leave row counts stable
    // thanks to `INSERT OR REPLACE`.
    insert_installed_package(
        &config,
        metadata(&package, &deps),
        &package,
        "/usr".to_string(),
    )
    .expect("second install should succeed");

    let conn = Connection::open(&config.install_db).unwrap();
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM installed_packages", &[]),
        3,
        "idempotent run changed installed package count"
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM package_dependencies", &[]),
        2,
        "idempotent run changed dependency edge count"
    );
    Ok(())
}
