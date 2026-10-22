use anyhow::Result;
use common::types::{
    metadata::{Build, Metadata},
    package::{Architectures, Source},
};
use rusqlite::{Connection, params};
use sisyphus::{
    package::db::insert_package,
    types::{package::Package, runtime_config::RuntimeConfig},
};
use tempfile::tempdir;

fn package(name: &str, version: &str, architecture: Architectures) -> Package {
    Package {
        schema: 1,
        name: name.to_string(),
        version: version.to_string(),
        release: 1,
        architecture,
        source: Source {
            url: "https://example.invalid/src.tar.gz".to_string(),
            sha256: "unused".to_string(),
        },
        makedeps: vec![],
        deps: vec![],
        prepare: None,
        build: None,
        check: None,
        package: "true".to_string(),
    }
}

fn metadata() -> Metadata {
    Metadata {
        format: 1,
        name: "unused".to_string(),
        version: "0.0.0".to_string(),
        release: 1,
        architecture: Architectures::X86_64,
        deps: vec![],
        source: Source {
            url: "https://example.invalid/src.tar.gz".to_string(),
            sha256: "unused".to_string(),
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
fn republishing_replaces_the_previous_row() -> Result<()> {
    let tmp = tempdir()?;
    let config = RuntimeConfig::build_at(&tmp.path().to_path_buf())?;
    let foo_1_9 = package("foo", "1.9.0", Architectures::X86_64);
    let foo_1_10 = package("foo", "1.10.0", Architectures::X86_64);
    let a_sky = tmp.path().join("a.sky");
    let b_sky = tmp.path().join("b.sky");

    insert_package(&config, &foo_1_9, a_sky.to_str().unwrap(), &metadata())?;
    insert_package(&config, &foo_1_10, b_sky.to_str().unwrap(), &metadata())?;

    let conn = Connection::open(&config.repo_db_path)?;
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM packages WHERE name = ?1",
            &[&"foo"]
        ),
        1,
        "a republished package must occupy exactly one repository row"
    );

    let (version, sky_path): (String, String) = conn.query_row(
        "SELECT version, sky_path FROM packages WHERE name = ?1 AND architecture = ?2",
        params!["foo", Architectures::X86_64.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(version, "1.10.0", "the newest publish must win");
    assert_eq!(sky_path, b_sky.to_str().unwrap(), "the newest sky must win");

    Ok(())
}

#[test]
fn publishing_the_same_name_for_two_architectures_keeps_both_rows() -> Result<()> {
    let tmp = tempdir()?;
    let config = RuntimeConfig::build_at(&tmp.path().to_path_buf())?;
    let foo_x86 = package("foo", "1.0.0", Architectures::X86_64);
    let foo_arm = package("foo", "1.0.0", Architectures::ARM);
    let x86_sky = tmp.path().join("x86.sky");
    let arm_sky = tmp.path().join("arm.sky");

    insert_package(&config, &foo_x86, x86_sky.to_str().unwrap(), &metadata())?;
    insert_package(&config, &foo_arm, arm_sky.to_str().unwrap(), &metadata())?;

    let conn = Connection::open(&config.repo_db_path)?;
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM packages WHERE name = ?1",
            &[&"foo"]
        ),
        2,
        "the same package must be publishable for each architecture"
    );

    for architecture in [Architectures::X86_64, Architectures::ARM] {
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM packages WHERE name = ?1 AND architecture = ?2",
                &[&"foo", &architecture.to_string()],
            ),
            1,
            "missing row for architecture `{architecture}`"
        );
    }

    Ok(())
}
