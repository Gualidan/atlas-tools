mod fixtures;

use anyhow::Result;
use atlas::install::db::{InstallReason, ensure_install_schema, update_installed_package};
use common::types::package::PackageIdentity;
use fixtures::{
    ARCH, generate_keypair, metadata_for, seed_repo_db, tempdir, write_signed_sky_with_file,
};
use rusqlite::{Connection, params, types::Value};
use std::path::PathBuf;
use tempfile::TempDir;

// Exercise the database operation independently of config loading and swapping
// directories. Real SQLite constraints are part of the transaction contract.
struct Database {
    _tmp: TempDir,
    conn: Connection,
    repo: Connection,
    sky: PathBuf,
}

fn rows(conn: &Connection, sql: &str) -> Result<Vec<Vec<Value>>> {
    let mut stmt = conn.prepare(sql)?;
    let columns = stmt.column_count();
    Ok(stmt
        .query_map([], |row| (0..columns).map(|i| row.get(i)).collect())?
        .collect::<rusqlite::Result<_>>()?)
}

fn snapshot(conn: &Connection) -> Result<Vec<Vec<Vec<Value>>>> {
    [
        "SELECT rowid, * FROM installed_packages ORDER BY name, architecture",
        "SELECT * FROM owned_files ORDER BY path, architecture",
        "SELECT * FROM package_dependencies ORDER BY package_name, architecture, dependency_name",
    ]
    .into_iter()
    .map(|sql| rows(conn, sql))
    .collect()
}

impl Database {
    fn new() -> Result<Self> {
        let tmp = tempdir();
        let conn = Connection::open_in_memory()?;
        ensure_install_schema(&conn)?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             INSERT INTO installed_packages
                (name, version, release, architecture, sky_path, install_dir, installed_at, install_reason)
             VALUES
                ('foo', '1.0.0', 1, 'x86_64', 'old.sky', '/local/foo', '2000-01-01', 'dependency'),
                ('old-dep', '1.0.0', 1, 'x86_64', 'dep.sky', '/local/old-dep', '2000-01-01', 'dependency'),
                ('new-dep', '1.0.0', 1, 'x86_64', 'dep.sky', '/local/new-dep', '2000-01-01', 'dependency'),
                ('consumer', '1.0.0', 1, 'x86_64', 'consumer.sky', '/local/consumer', '2000-01-01', 'explicit'),
                ('foo', '1.0.0', 1, 'arm', 'arm.sky', '/arm/foo', '2000-01-01', 'explicit'),
                ('old-dep', '1.0.0', 1, 'arm', 'arm-dep.sky', '/arm/old-dep', '2000-01-01', 'dependency');
             INSERT INTO owned_files (path, package_name, architecture) VALUES
                ('old/file.txt', 'foo', 'x86_64'),
                ('old/file.txt', 'foo', 'arm'),
                ('consumer/file.txt', 'consumer', 'x86_64');
             INSERT INTO package_dependencies (package_name, architecture, dependency_name) VALUES
                ('foo', 'x86_64', 'old-dep'),
                ('foo', 'arm', 'old-dep'),
                ('consumer', 'x86_64', 'foo');",
        )?;
        let (keypair, _) = generate_keypair()?;
        let sky = write_signed_sky_with_file(
            &tmp,
            &keypair,
            "new.sky",
            "foo",
            "2.0.0",
            7,
            b"new\n",
            vec![],
            "new",
            "file.txt",
        )?;
        let repo_path = tmp.path().join("repo.db");
        seed_repo_db(&repo_path, &[("new-dep", "1.0.0", 1, "dep.sky")]);
        let repo = Connection::open(repo_path)?;
        Ok(Self {
            _tmp: tmp,
            conn,
            repo,
            sky,
        })
    }

    fn update(&mut self, deps: &[&str]) -> Result<()> {
        update_installed_package(
            &mut self.conn,
            &self.repo,
            &metadata_for(
                "foo",
                "2.0.0",
                7,
                deps.iter().map(|dep| dep.to_string()).collect(),
            ),
            &PackageIdentity {
                name: "foo",
                version: "2.0.0",
                release: 7,
                architecture: ARCH,
            },
            self.sky.to_str().unwrap(),
            "/local/foo",
        )
    }

    fn assert_rollback(&mut self, deps: &[&str], expected: &str) -> Result<()> {
        let before = snapshot(&self.conn)?;
        let err = self.update(deps).expect_err("update must fail");
        assert!(
            format!("{err:#}").contains(expected),
            "unexpected error: {err:#}"
        );
        assert_eq!(
            snapshot(&self.conn)?,
            before,
            "all ledger tables must roll back"
        );
        assert!(
            self.conn.is_autocommit(),
            "failure must release the transaction"
        );
        Ok(())
    }
}

#[test]
fn update_replaces_version_files_and_outgoing_dependencies_in_place() -> Result<()> {
    let mut db = Database::new()?;
    let unaffected = [
        "SELECT rowid, * FROM installed_packages WHERE name != 'foo' OR architecture != 'x86_64' ORDER BY name, architecture",
        "SELECT * FROM owned_files WHERE package_name != 'foo' OR architecture != 'x86_64' ORDER BY path, architecture",
        "SELECT * FROM package_dependencies WHERE package_name != 'foo' OR architecture != 'x86_64' ORDER BY package_name, architecture",
    ];
    let before: Vec<_> = unaffected
        .iter()
        .map(|sql| rows(&db.conn, sql))
        .collect::<Result<_>>()?;
    let rowid: i64 = db.conn.query_row(
        "SELECT rowid FROM installed_packages WHERE name = 'foo' AND architecture = 'x86_64'",
        [],
        |row| row.get(0),
    )?;

    db.update(&["new-dep"])?;

    let record: (i64, String, u32, String, String, String, String) = db.conn.query_row(
        "SELECT rowid, version, release, sky_path, install_dir, installed_at, install_reason
         FROM installed_packages WHERE name = 'foo' AND architecture = 'x86_64'",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        },
    )?;
    assert_eq!(
        record,
        (
            rowid,
            "2.0.0".into(),
            7,
            db.sky.to_str().unwrap().into(),
            "/local/foo".into(),
            "2000-01-01".into(),
            "dependency".into()
        )
    );
    assert_eq!(
        rows(
            &db.conn,
            "SELECT path FROM owned_files WHERE package_name = 'foo' AND architecture = 'x86_64'"
        )?,
        vec![vec![Value::Text("new/file.txt".into())]]
    );
    assert_eq!(
        rows(
            &db.conn,
            "SELECT dependency_name FROM package_dependencies WHERE package_name = 'foo' AND architecture = 'x86_64'"
        )?,
        vec![vec![Value::Text("new-dep".into())]]
    );
    let after: Vec<_> = unaffected
        .iter()
        .map(|sql| rows(&db.conn, sql))
        .collect::<Result<_>>()?;
    assert_eq!(
        after, before,
        "other packages, architectures and incoming edges must survive"
    );
    assert!(rows(&db.conn, "PRAGMA foreign_key_check")?.is_empty());
    Ok(())
}

#[test]
fn update_can_drop_all_dependencies_and_is_repeatable() -> Result<()> {
    let mut db = Database::new()?;
    db.update(&[])?;
    assert!(rows(&db.conn, "SELECT * FROM package_dependencies WHERE package_name = 'foo' AND architecture = 'x86_64'")?.is_empty());
    let before = snapshot(&db.conn)?;
    db.update(&[])?;
    assert_eq!(snapshot(&db.conn)?, before);
    Ok(())
}

#[test]
fn repeated_dependency_names_create_one_edge() -> Result<()> {
    let mut db = Database::new()?;
    db.update(&["new-dep", "new-dep"])?;
    assert_eq!(rows(&db.conn, "SELECT dependency_name FROM package_dependencies WHERE package_name = 'foo' AND architecture = 'x86_64'")?.len(), 1);
    Ok(())
}

#[test]
fn missing_archive_rolls_back_deleted_files_and_edges() -> Result<()> {
    let mut db = Database::new()?;
    std::fs::remove_file(&db.sky)?;
    db.assert_rollback(&[], "Failed to read sky file")
}

#[test]
fn malformed_manifest_rolls_back_the_package_record() -> Result<()> {
    let mut db = Database::new()?;
    fixtures::write_sky(&db.sky, &[], b"unused", "invalid manifest\n", &[])?;
    db.assert_rollback(&[], "Failed to parse manifest's lines")
}

#[test]
fn duplicate_manifest_paths_roll_back_earlier_file_inserts() -> Result<()> {
    let mut db = Database::new()?;
    fixtures::write_sky(
        &db.sky,
        &[],
        b"unused",
        "f\t0644\thash\tnew/file.txt\nf\t0644\thash\tnew/file.txt\n",
        &[],
    )?;
    db.assert_rollback(&[], "Failed to insert owned files")
}

#[test]
fn update_cannot_take_another_packages_owned_path() -> Result<()> {
    let mut db = Database::new()?;
    db.conn.execute(
        "INSERT INTO owned_files VALUES ('new/file.txt', 'consumer', 'x86_64')",
        [],
    )?;
    db.assert_rollback(&[], "Failed to insert owned files")
}

#[test]
fn missing_repository_dependency_rolls_back_after_a_valid_edge() -> Result<()> {
    let mut db = Database::new()?;
    db.assert_rollback(
        &["new-dep", "missing-dep"],
        "Dependency `missing-dep` not found",
    )?;
    // A failed transaction must not prevent a later valid update.
    db.update(&["new-dep"])
}

#[test]
fn repository_dependency_must_match_the_package_architecture() -> Result<()> {
    let mut db = Database::new()?;
    db.repo.execute(
        "UPDATE packages SET architecture = 'arm' WHERE name = 'new-dep'",
        [],
    )?;
    db.assert_rollback(&["new-dep"], "Dependency `new-dep` not found")
}

#[test]
fn dependency_present_only_in_repository_cannot_create_a_dangling_edge() -> Result<()> {
    let mut db = Database::new()?;
    db.conn
        .execute("DELETE FROM installed_packages WHERE name = 'new-dep'", [])?;
    db.assert_rollback(&["new-dep"], "FOREIGN KEY constraint failed")
}

#[test]
fn update_enables_foreign_key_enforcement_on_the_supplied_connection() -> Result<()> {
    let mut db = Database::new()?;
    db.conn.execute_batch(
        "DELETE FROM installed_packages WHERE name = 'new-dep'; PRAGMA foreign_keys = OFF;",
    )?;
    db.assert_rollback(&["new-dep"], "FOREIGN KEY constraint failed")?;
    assert_eq!(
        db.conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i32>(0))?,
        1
    );
    Ok(())
}

#[test]
fn self_dependency_is_rejected_atomically() -> Result<()> {
    let mut db = Database::new()?;
    db.repo.execute("UPDATE packages SET name = 'foo'", [])?;
    db.assert_rollback(&["foo"], "CHECK constraint failed")
}

#[test]
fn symlinks_are_owned_but_directories_are_not() -> Result<()> {
    let mut db = Database::new()?;
    fixtures::write_sky(
        &db.sky,
        &[],
        b"unused",
        "d\t0755\t-\tbin\nl\t0777\t../lib/tool\tbin/tool\n",
        &[],
    )?;
    db.update(&[])?;
    let owner: (String, String) = db.conn.query_row(
        "SELECT path, package_name FROM owned_files WHERE package_name = ? AND architecture = ?",
        params!["foo", ARCH.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(owner, ("bin/tool".into(), "foo".into()));
    Ok(())
}

#[test]
fn install_reason_display_preserves_the_database_values() {
    for (reason, expected) in [
        (InstallReason::Dependency, "dependency"),
        (InstallReason::Explicit, "explicit"),
    ] {
        assert_eq!(format!("{reason}"), expected);
        assert_eq!(reason.to_string(), expected);
    }
}
