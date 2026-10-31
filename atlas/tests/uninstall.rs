mod fixtures;

use anyhow::Result;
use atlas::{install::install::install, uninstall::uninstall::uninstall};
use fixtures::{
    ConfigEnv, count, generate_keypair, runtime_config, seed_repo_db, tempdir, write_signed_sky,
    write_signed_sky_in_dir,
};
use rusqlite::Connection;

use crate::fixtures::seed_repo_db_with_deps;

#[test]
fn uninstall_package_with_no_dependents_succeeds() -> Result<()> {
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

    install(&"foo".to_string(), &config, &tmp.path().join("root")).expect("install should succeed");

    uninstall(&config, &"foo".to_string(), false).expect("uninstall should succeed");

    let conn = Connection::open(&config.install_db)?;
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM installed_packages WHERE name = ?1",
            &[&"foo"],
        ),
        0,
        "installed_packages row should be removed"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM owned_files WHERE package_name = ?1",
            &[&"foo"],
        ),
        0,
        "owned_files rows should be removed"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM package_dependencies WHERE package_name = ?1 OR dependency_name = ?1",
            &[&"foo"],
        ),
        0,
        "package_dependencies rows should be removed"
    );
    assert!(
        !tmp.path().join("root").join("foo-1.0.0-1").exists(),
        "install_dir should be removed from disk"
    );

    Ok(())
}

#[test]
fn uninstall_package_with_dependent_fails() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;

    let a_sky = write_signed_sky(&tmp, &keypair, "a.sky", "A", "1.0.0", 1, b"a\n", vec![])?;
    let b_sky = write_signed_sky_in_dir(
        &tmp,
        &keypair,
        "b.sky",
        "B",
        "1.0.0",
        1,
        b"b\n",
        vec!["A".to_string()],
        "bdir",
    )?;
    seed_repo_db(
        &config.repo_db,
        &[
            ("A", "1.0.0", 1, a_sky.to_str().unwrap()),
            ("B", "1.0.0", 1, b_sky.to_str().unwrap()),
        ],
    );

    let root = tmp.path().join("root");
    install(&"A".to_string(), &config, &root).expect("installing A should succeed");
    install(&"B".to_string(), &config, &root).expect("installing B should succeed");

    let result = uninstall(&config, &"A".to_string(), false);
    let err = result.expect_err("uninstalling A should fail because B depends on it");
    let chain = format!("{err:#}");
    assert!(chain.contains("depends on"), "error chain was: {chain}");
    assert!(chain.contains('A'), "error chain should mention A: {chain}");
    assert!(
        chain.contains('B'),
        "error chain should mention B as the dependent: {chain}"
    );

    let conn = Connection::open(&config.install_db)?;
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM installed_packages WHERE name = ?1",
            &[&"A"],
        ),
        1,
        "A should still be installed after failed uninstall"
    );
    assert!(
        root.join("A-1.0.0-1").exists(),
        "A's install_dir should still exist on disk after failed uninstall"
    );

    Ok(())
}

#[test]
fn uninstall_dependent_also_uninstalls_non_explicitly_installed_dependencies() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;

    let a_sky = write_signed_sky(&tmp, &keypair, "a.sky", "A", "1.0.0", 1, b"a\n", vec![])?;
    let b_sky = write_signed_sky_in_dir(
        &tmp,
        &keypair,
        "b.sky",
        "B",
        "1.0.0",
        1,
        b"b\n",
        vec!["A".to_string()],
        "bdir",
    )?;
    seed_repo_db_with_deps(
        &config.repo_db,
        &[
            ("A", "1.0.0", 1, a_sky.to_str().unwrap(), &[]),
            ("B", "1.0.0", 1, b_sky.to_str().unwrap(), &["A"]),
        ],
    );

    let root = tmp.path().join("root");

    install(&"B".to_string(), &config, &root).expect("installing B should succeed");

    uninstall(&config, &"B".to_string(), false).expect("uninstalling B should succeed");

    let conn = Connection::open(&config.install_db)?;
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM installed_packages", &[]),
        0,
        "installed_packages should be empty"
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM package_dependencies", &[]),
        0,
        "package_dependencies should be empty"
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM owned_files", &[]),
        0,
        "owned_files should be empty"
    );
    assert!(
        !root.join("A-1.0.0-1").exists(),
        "A's install_dir should be gone"
    );
    assert!(
        !root.join("B-1.0.0-1").exists(),
        "B's install_dir should be gone"
    );

    Ok(())
}

#[test]
fn uninstall_non_installed_package_fails() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (_keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;

    let result = uninstall(&config, &"ghost".to_string(), false);
    let err = result.expect_err("uninstalling a non-installed package should fail");
    let chain = format!("{err:#}");
    assert!(chain.contains("not found"), "error chain was: {chain}");
    assert!(chain.contains("ghost"), "error chain was: {chain}");

    Ok(())
}

#[test]
fn uninstall_keeps_explicitly_installed_dependencies() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;
    let sky_path_a = write_signed_sky(
        &tmp,
        &keypair,
        "baz.sky",
        "baz",
        "1.0.0",
        1,
        b"baz\n",
        vec![],
    )?;
    let sky_path_b = write_signed_sky_in_dir(
        &tmp,
        &keypair,
        "bar.sky",
        "bar",
        "1.0.0",
        1,
        b"bar\n",
        vec!["baz".to_string()],
        "bardir",
    )?;
    seed_repo_db(
        &config.repo_db,
        &[("baz", "1.0.0", 1, sky_path_a.to_str().unwrap())],
    );
    seed_repo_db(
        &config.repo_db,
        &[("bar", "1.0.0", 1, sky_path_b.to_str().unwrap())],
    );

    let root = tmp.path().join("root");

    install(&"baz".to_string(), &config, &root).expect("install should succeed");
    install(&"bar".to_string(), &config, &root).expect("install should succeed");

    uninstall(&config, &"bar".to_string(), false).expect("uninstall should succeed");

    let conn = Connection::open(&config.install_db)?;

    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM installed_packages WHERE name = ?1",
            &[&"baz"]
        ),
        1,
        "baz should still be installed because it was explicitley installed by the user"
    );

    // The universe of this test is exactly {A, B}, so the tables must be
    // completely empty afterwards. Blanket checks catch partial removals
    // that a targeted check would miss.
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM installed_packages", &[]),
        1,
        "installed_packages should contain baz"
    );

    println!(
        "{}",
        count(&conn, "SELECT COUNT(*) FROM package_dependencies", &[])
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM package_dependencies", &[]),
        0,
        "package_dependencies should be empty"
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM owned_files", &[]),
        1,
        "owned_files should contain baz's files"
    );

    // Disk must match the database — this is the check for your PR #5 bug class
    // (right DB rows deleted, wrong/stale directories left behind).
    assert!(
        root.join("baz-1.0.0-1").exists(),
        "baz's install_dir should be kept"
    );
    assert!(
        !root.join("bar-1.0.0-1").exists(),
        "bar's install_dir should be removed from disk"
    );

    Ok(())
}
