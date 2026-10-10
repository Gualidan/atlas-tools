use std::path::{Path, PathBuf};

use atlas::cli::db::insert_installed_package;
use common::types::{
    context::Context,
    metadata::{Build, Metadata},
    package::{Architectures, Package, Source},
    settings::Settings,
};
use rusqlite::{Connection, params};
use sisyphus::types::runtime_config::RuntimeConfig;
use tempfile::TempDir;

const ARCH: Architectures = Architectures::X86_64;

fn runtime_config(tmp: &TempDir) -> RuntimeConfig {
    RuntimeConfig {
        recipe_repo: tmp.path().join("recipes"),
        sky_repo: tmp.path().join("sky"),
        srcdir: tmp.path().join("src"),
        repo_db_path: tmp.path().join("repo.db"),
        fetch_cache: tmp.path().join("fetch"),
        cache_db_path: tmp.path().join("cache.db"),
        install_db_path: tmp.path().join("install.db"),
    }
}

fn seed_repo_db(path: &Path, packages: &[(&str, &str, u32, &str)]) {
    let conn = Connection::open(path).unwrap();
    conn.execute(
        "CREATE TABLE IF NOT EXISTS packages (name TEXT NOT NULL, version TEXT NOT NULL, release INTEGER NOT NULL, architecture TEXT NOT NULL, sky_path TEXT PRIMARY KEY, recipe_sha256 TEXT NOT NULL, built_at TEXT NOT NULL, builder TEXT NOT NULL, deps TEXT NOT NULL, makedeps TEXT NOT NULL)",
        [],
    )
    .unwrap();

    for (name, version, release, sky_path) in packages {
        conn.execute(
            "INSERT OR REPLACE INTO packages (name, version, release, architecture, sky_path, recipe_sha256, built_at, builder, deps, makedeps) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                *name,
                *version,
                *release,
                ARCH.to_string(),
                *sky_path,
                "deadbeef",
                "2026-01-01T00:00:00Z",
                "tester",
                "[]",
                "[]"
            ],
        )
        .unwrap();
    }
}

fn package(name: &str, deps: &[&str]) -> Package {
    Package {
        schema: 1,
        name: name.to_string(),
        version: "1.0.0".to_string(),
        release: 1,
        architecture: ARCH,
        source: Source {
            url: "https://example.invalid/src.tar.gz".to_string(),
            sha256: None,
        },
        makedeps: Vec::new(),
        deps: deps.iter().map(|dep| dep.to_string()).collect(),
        prepare: None,
        build: None,
        check: None,
        package: "true".to_string(),
    }
}

fn metadata(package: &Package) -> Metadata {
    Metadata {
        format: 1,
        name: package.name.clone(),
        version: package.version.clone(),
        release: package.release,
        architecture: package.architecture,
        deps: package.deps.clone(),
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

fn context<'a>(
    package: &'a Package,
    settings: &'a Settings,
    conn: &'a Connection,
    config: &RuntimeConfig,
) -> Context<'a> {
    Context {
        package,
        sky_path: PathBuf::from("/sky/root.sky"),
        sky_repo: config.sky_repo.clone(),
        install_dir: PathBuf::from("/"),
        temp_dir: PathBuf::from("/tmp"),
        settings,
        dependencies: package.deps.clone(),
        conn,
    }
}

fn count(conn: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> i64 {
    conn.query_row(sql, params, |row| row.get(0)).unwrap()
}

#[test]
fn insert_installed_package_records_root_deps_and_edges() {
    let tmp = TempDir::new().unwrap();
    let config = runtime_config(&tmp);

    seed_repo_db(
        &config.repo_db_path,
        &[
            ("root", "1.0.0", 1, "/sky/root.sky"),
            ("dep-a", "2.1.0", 3, "/sky/dep-a.sky"),
            ("dep-b", "0.4.0", 1, "/sky/dep-b.sky"),
        ],
    );

    let package = package("root", &["dep-a", "dep-b"]);
    let settings = Settings {
        priv_key_path: PathBuf::from("/tmp/key"),
        pub_key_path: PathBuf::from("/tmp/key.pub"),
    };
    let ctx_conn = Connection::open_in_memory().unwrap();

    let ctx = context(&package, &settings, &ctx_conn, &config);
    insert_installed_package(
        &config,
        metadata(&package),
        &package,
        ctx,
        "/usr".to_string(),
    )
    .expect("first install should succeed");

    let conn = Connection::open(&config.install_db_path).unwrap();

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
    let ctx2 = context(&package, &settings, &ctx_conn, &config);
    insert_installed_package(
        &config,
        metadata(&package),
        &package,
        ctx2,
        "/usr".to_string(),
    )
    .expect("second install should succeed");

    let conn = Connection::open(&config.install_db_path).unwrap();
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
}
