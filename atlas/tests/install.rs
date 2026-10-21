mod fixtures;

use anyhow::Result;
use atlas::{
    cli::cli::{InstallArgs, UninstallArgs},
    install::install::install,
    types::runtime_config::RuntimeConfig,
    uninstall::uninstall::uninstall,
};
use common::types::config::load_config;
use fixtures::{
    ConfigEnv, generate_keypair, runtime_config, seed_repo_db, tempdir, write_signed_sky,
    write_signed_sky_with_file, write_signed_sky_with_raw_entry, write_signed_sky_with_symlink,
};
use rusqlite::Connection;
use tar::EntryType;

fn install_args(package: &str) -> InstallArgs {
    InstallArgs {
        package: Some(package.to_string()),
    }
}

fn uninstall_args(package: &str) -> UninstallArgs {
    UninstallArgs {
        package: package.to_string(),
    }
}

fn count(conn: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> i64 {
    conn.query_row(sql, params, |row| row.get(0)).unwrap()
}

/// Asserts `install` fails with `crime` in its error chain and that the
/// transaction left nothing behind: the package's install dir must not exist
/// and the install database must have no rows for the package.
fn assert_install_rejects(
    config: &RuntimeConfig,
    root: &std::path::Path,
    package_name: &str,
    result: anyhow::Result<()>,
    crime: &str,
) {
    let err = result.expect_err("install should have been rejected");
    let chain = format!("{err:#}");
    assert!(chain.contains(crime), "error chain was: {chain}");

    assert!(
        !root.join(format!("{package_name}-1.0.0-1")).exists(),
        "nothing may land on disk for a rejected install"
    );

    let conn = Connection::open(&config.install_db).unwrap();
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM owned_files WHERE package_name = ?1",
            &[&package_name],
        ),
        0,
        "a rejected install must not leave owned_files rows behind"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM installed_packages WHERE name = ?1",
            &[&package_name],
        ),
        0,
        "a rejected install must not leave an installed_packages row behind"
    );
}

#[test]
fn install_rejects_setuid_file() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;

    let sky_path = write_signed_sky_with_raw_entry(
        &tmp,
        &keypair,
        "foo.sky",
        "foo",
        "1.0.0",
        1,
        "dir/evil.bin",
        0o4755,
        EntryType::file(),
        b"evil\n",
    )?;
    seed_repo_db(
        &config.repo_db,
        &[("foo", "1.0.0", 1, sky_path.to_str().unwrap())],
    );

    let root = tmp.path().join("root");
    let result = install(install_args("foo"), &config, root.clone());
    assert_install_rejects(&config, &root, "foo", result, "setuid");

    Ok(())
}

#[test]
fn install_rejects_setgid_file() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;

    let sky_path = write_signed_sky_with_raw_entry(
        &tmp,
        &keypair,
        "foo.sky",
        "foo",
        "1.0.0",
        1,
        "dir/evil.bin",
        0o2755,
        EntryType::file(),
        b"evil\n",
    )?;
    seed_repo_db(
        &config.repo_db,
        &[("foo", "1.0.0", 1, sky_path.to_str().unwrap())],
    );

    let root = tmp.path().join("root");
    let result = install(install_args("foo"), &config, root.clone());
    assert_install_rejects(&config, &root, "foo", result, "setgid");

    Ok(())
}

#[test]
fn install_rejects_absolute_path() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;

    let sky_path = write_signed_sky_with_raw_entry(
        &tmp,
        &keypair,
        "foo.sky",
        "foo",
        "1.0.0",
        1,
        "/etc/evil",
        0o644,
        EntryType::file(),
        b"evil\n",
    )?;
    seed_repo_db(
        &config.repo_db,
        &[("foo", "1.0.0", 1, sky_path.to_str().unwrap())],
    );

    let root = tmp.path().join("root");
    let result = install(install_args("foo"), &config, root.clone());
    assert_install_rejects(&config, &root, "foo", result, "absolute");

    Ok(())
}

#[test]
fn install_rejects_parent_dir_path() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;

    let sky_path = write_signed_sky_with_raw_entry(
        &tmp,
        &keypair,
        "foo.sky",
        "foo",
        "1.0.0",
        1,
        "dir/../../evil",
        0o644,
        EntryType::file(),
        b"evil\n",
    )?;
    seed_repo_db(
        &config.repo_db,
        &[("foo", "1.0.0", 1, sky_path.to_str().unwrap())],
    );

    let root = tmp.path().join("root");
    let result = install(install_args("foo"), &config, root.clone());
    assert_install_rejects(&config, &root, "foo", result, "ParentDir");

    Ok(())
}

#[test]
fn install_rejects_fifo_entry() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;

    let sky_path = write_signed_sky_with_raw_entry(
        &tmp,
        &keypair,
        "foo.sky",
        "foo",
        "1.0.0",
        1,
        "dir/pipe",
        0o644,
        EntryType::fifo(),
        b"",
    )?;
    seed_repo_db(
        &config.repo_db,
        &[("foo", "1.0.0", 1, sky_path.to_str().unwrap())],
    );

    let root = tmp.path().join("root");
    let result = install(install_args("foo"), &config, root.clone());
    assert_install_rejects(&config, &root, "foo", result, "Fifo");

    Ok(())
}

/// Insurance: the raw fixture builder must be able to produce a sky that
/// installs cleanly, otherwise the rejection tests above could all be passing
/// because the fixture itself is broken rather than because a gate fired.
#[test]
fn raw_fixture_with_clean_entry_installs() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;

    let sky_path = write_signed_sky_with_raw_entry(
        &tmp,
        &keypair,
        "foo.sky",
        "foo",
        "1.0.0",
        1,
        "dir/file.txt",
        0o644,
        EntryType::file(),
        b"benign\n",
    )?;
    seed_repo_db(
        &config.repo_db,
        &[("foo", "1.0.0", 1, sky_path.to_str().unwrap())],
    );

    let root = tmp.path().join("root");
    install(install_args("foo"), &config, root.clone())
        .expect("the clean raw fixture must install successfully");

    let conn = Connection::open(&config.install_db)?;
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM installed_packages WHERE name = ?1",
            &[&"foo"],
        ),
        1,
        "the clean install must be recorded"
    );

    Ok(())
}

#[test]
fn fresh_install_populates_owned_files() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;

    let sky_path = write_signed_sky(
        &tmp,
        &keypair,
        "foo.sky",
        "foo",
        "1.0.0",
        1,
        b"foo\n",
        vec![],
    )?;
    seed_repo_db(
        &config.repo_db,
        &[("foo", "1.0.0", 1, sky_path.to_str().unwrap())],
    );

    install(install_args("foo"), &config, tmp.path().join("root"))
        .expect("fresh install should succeed");

    let conn = Connection::open(&config.install_db)?;
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM owned_files WHERE package_name = ?1",
            &[&"foo"],
        ),
        1,
        "expected one owned_files row per manifest file"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM installed_packages WHERE name = ?1",
            &[&"foo"],
        ),
        1,
        "expected exactly one installed_packages row"
    );

    Ok(())
}

#[test]
fn install_with_file_conflict_fails() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;

    let a_sky = write_signed_sky(&tmp, &keypair, "a.sky", "A", "1.0.0", 1, b"a\n", vec![])?;
    let b_sky = write_signed_sky(&tmp, &keypair, "b.sky", "B", "1.0.0", 1, b"b\n", vec![])?;
    seed_repo_db(
        &config.repo_db,
        &[
            ("A", "1.0.0", 1, a_sky.to_str().unwrap()),
            ("B", "1.0.0", 1, b_sky.to_str().unwrap()),
        ],
    );

    let root = tmp.path().join("root");
    install(install_args("A"), &config, root.clone()).expect("installing A should succeed");

    let result = install(install_args("B"), &config, root);
    let err = result.expect_err("installing B should fail on the file conflict");
    let chain = format!("{err:#}");
    assert!(chain.contains("already owns"), "error chain was: {chain}");
    assert!(chain.contains("dir/file.txt"), "error chain was: {chain}");
    assert!(chain.contains("A"), "error chain was: {chain}");

    Ok(())
}

#[test]
fn idempotent_install_does_not_conflict_with_itself() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;

    let sky_path = write_signed_sky(
        &tmp,
        &keypair,
        "foo.sky",
        "foo",
        "1.0.0",
        1,
        b"foo\n",
        vec![],
    )?;
    seed_repo_db(
        &config.repo_db,
        &[("foo", "1.0.0", 1, sky_path.to_str().unwrap())],
    );

    let root = tmp.path().join("root");
    install(install_args("foo"), &config, root.clone()).expect("first install should succeed");
    install(install_args("foo"), &config, root).expect("second install should succeed");

    let conn = Connection::open(&config.install_db)?;
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM owned_files WHERE package_name = ?1",
            &[&"foo"],
        ),
        1,
        "re-installing must not duplicate owned_files rows"
    );

    Ok(())
}

#[test]
fn fresh_install_rolls_back_completely_when_the_database_insert_fails() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;

    // The metadata declares a dependency that was never seeded into the repo
    // database, so the install database insert fails *after* the payload has
    // already been swapped into place.
    let sky_path = write_signed_sky(
        &tmp,
        &keypair,
        "foo.sky",
        "foo",
        "1.0.0",
        1,
        b"foo\n",
        vec!["missing-dep".to_string()],
    )?;
    seed_repo_db(
        &config.repo_db,
        &[("foo", "1.0.0", 1, sky_path.to_str().unwrap())],
    );

    let root = tmp.path().join("root");
    let err = install(install_args("foo"), &config, root.clone())
        .expect_err("install should fail because the dependency is missing");
    let chain = format!("{err:#}");
    assert!(
        chain.contains("missing-dep"),
        "the error should name the missing dependency: {chain}"
    );

    assert!(
        !root.join("foo-1.0.0-1").exists(),
        "the install dir should have been rolled back completely"
    );

    let conn = Connection::open(&config.install_db)?;
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM installed_packages WHERE name = ?1",
            &[&"foo"],
        ),
        0,
        "the failed install must not leave an installed_packages row behind"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM owned_files WHERE package_name = ?1",
            &[&"foo"],
        ),
        0,
        "the failed install must not leave owned_files rows behind"
    );

    Ok(())
}

#[test]
fn reinstall_rollback_preserves_the_previous_version() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;

    let good_sky = write_signed_sky(
        &tmp,
        &keypair,
        "good.sky",
        "foo",
        "1.0.0",
        1,
        b"v1\n",
        vec![],
    )?;
    seed_repo_db(
        &config.repo_db,
        &[("foo", "1.0.0", 1, good_sky.to_str().unwrap())],
    );

    let root = tmp.path().join("root");
    install(install_args("foo"), &config, root.clone()).expect("first install should succeed");

    let install_dir = root.join("foo-1.0.0-1");
    let installed_file = install_dir.join("dir").join("file.txt");
    assert_eq!(
        std::fs::read_to_string(&installed_file)?,
        "v1\n",
        "the first install should have written the payload to disk"
    );

    // Clear the seeded row before re-seeding: the packages table UNIQUEs
    // (name, architecture) and seed_repo_db's INSERT OR REPLACE already
    // overwrites the row, so the failing sky ends up the package's only
    // candidate either way — the delete just makes that intent explicit.
    let repo_conn = Connection::open(&config.repo_db)?;
    repo_conn.execute("DELETE FROM packages WHERE name = ?1", [&"foo"])?;

    let bad_sky = write_signed_sky(
        &tmp,
        &keypair,
        "bad.sky",
        "foo",
        "1.0.0",
        1,
        b"v2\n",
        vec!["missing-dep".to_string()],
    )?;
    seed_repo_db(
        &config.repo_db,
        &[("foo", "1.0.0", 1, bad_sky.to_str().unwrap())],
    );

    let err = install(install_args("foo"), &config, root.clone())
        .expect_err("the second install should fail because the dependency is missing");
    let chain = format!("{err:#}");
    assert!(
        chain.contains("missing-dep"),
        "the error should name the missing dependency: {chain}"
    );

    assert!(
        install_dir.exists(),
        "the previous installation must still be on disk after the rollback"
    );
    assert_eq!(
        std::fs::read_to_string(&installed_file)?,
        "v1\n",
        "the rollback must restore the previous version's content"
    );

    let conn = Connection::open(&config.install_db)?;
    let (version, release): (String, u32) = conn.query_row(
        "SELECT version, release FROM installed_packages WHERE name = ?1",
        [&"foo"],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(version, "1.0.0", "the recorded version must be untouched");
    assert_eq!(release, 1, "the recorded release must be untouched");
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM owned_files WHERE package_name = ?1",
            &[&"foo"],
        ),
        1,
        "the failed reinstall must not have changed the owned files"
    );

    Ok(())
}

#[test]
fn two_packages_can_share_a_directory() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;

    let a_sky = write_signed_sky_with_file(
        &tmp,
        &keypair,
        "a.sky",
        "A",
        "1.0.0",
        1,
        b"a\n",
        vec![],
        "usr",
        "a.txt",
    )?;
    let b_sky = write_signed_sky_with_file(
        &tmp,
        &keypair,
        "b.sky",
        "B",
        "1.0.0",
        1,
        b"b\n",
        vec![],
        "usr",
        "b.txt",
    )?;
    seed_repo_db(
        &config.repo_db,
        &[
            ("A", "1.0.0", 1, a_sky.to_str().unwrap()),
            ("B", "1.0.0", 1, b_sky.to_str().unwrap()),
        ],
    );

    let root = tmp.path().join("root");
    install(install_args("A"), &config, root.clone()).expect("installing A should succeed");
    install(install_args("B"), &config, root.clone())
        .expect("installing B should succeed even though it shares a directory with A");

    let conn = Connection::open(&config.install_db)?;
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM owned_files WHERE path = ?1 AND package_name = ?2",
            &[&"usr/a.txt", &"A"],
        ),
        1,
        "A should own usr/a.txt"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM owned_files WHERE path = ?1 AND package_name = ?2",
            &[&"usr/b.txt", &"B"],
        ),
        1,
        "B should own usr/b.txt"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM owned_files WHERE path = ?1",
            &[&"usr"],
        ),
        0,
        "the shared directory must not be recorded as an owned path"
    );

    Ok(())
}

#[test]
fn symlinks_are_recorded_as_owned_files() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;

    let sky_path = write_signed_sky_with_symlink(
        &tmp,
        &keypair,
        "sym.sky",
        "sym",
        "1.0.0",
        1,
        "link",
        "target.txt",
        vec![],
        "dir",
    )?;
    seed_repo_db(
        &config.repo_db,
        &[("sym", "1.0.0", 1, sky_path.to_str().unwrap())],
    );

    let root = tmp.path().join("root");
    install(install_args("sym"), &config, root.clone()).expect("install should succeed");

    let conn = Connection::open(&config.install_db)?;
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM owned_files WHERE package_name = ?1",
            &[&"sym"],
        ),
        1,
        "the symlink should be recorded as an owned file"
    );
    let path: String = conn.query_row(
        "SELECT path FROM owned_files WHERE package_name = ?1",
        [&"sym"],
        |row| row.get(0),
    )?;
    assert_eq!(
        path, "dir/link",
        "the owned path should be the symlink's path"
    );

    Ok(())
}

#[test]
fn uninstall_after_co_install_leaves_the_neighbor_intact() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;
    let settings = load_config()?;

    let a_sky = write_signed_sky_with_file(
        &tmp,
        &keypair,
        "a.sky",
        "A",
        "1.0.0",
        1,
        b"a\n",
        vec![],
        "usr",
        "a.txt",
    )?;
    let b_sky = write_signed_sky_with_file(
        &tmp,
        &keypair,
        "b.sky",
        "B",
        "1.0.0",
        1,
        b"b\n",
        vec![],
        "usr",
        "b.txt",
    )?;
    seed_repo_db(
        &config.repo_db,
        &[
            ("A", "1.0.0", 1, a_sky.to_str().unwrap()),
            ("B", "1.0.0", 1, b_sky.to_str().unwrap()),
        ],
    );

    let root = tmp.path().join("root");
    install(install_args("A"), &config, root.clone()).expect("installing A should succeed");
    install(install_args("B"), &config, root.clone()).expect("installing B should succeed");

    uninstall(&config, uninstall_args("A"), &settings).expect("uninstalling A should succeed");

    let conn = Connection::open(&config.install_db)?;
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM owned_files WHERE package_name = ?1",
            &[&"A"],
        ),
        0,
        "A's owned files should be removed"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM owned_files WHERE package_name = ?1 AND path = ?2",
            &[&"B", &"usr/b.txt"],
        ),
        1,
        "B's owned files must be untouched"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM installed_packages WHERE name = ?1",
            &[&"B"],
        ),
        1,
        "B must still be recorded as installed"
    );

    assert!(
        !root.join("A-1.0.0-1").exists(),
        "A's install dir should be gone"
    );
    let b_dir = root.join("B-1.0.0-1");
    assert!(b_dir.exists(), "B's install dir should still exist");
    assert_eq!(
        std::fs::read_to_string(b_dir.join("usr").join("b.txt"))?,
        "b\n",
        "B's files must still be on disk"
    );

    Ok(())
}
