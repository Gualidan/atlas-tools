mod fixtures;

use anyhow::Result;
use atlas::{
    install::install::install, types::runtime_config::RuntimeConfig,
    uninstall::uninstall::uninstall,
};
use fixtures::{
    ARCH, ConfigEnv, count, generate_keypair, runtime_config, seed_repo_db, seed_repo_db_with_deps,
    tempdir, write_signed_sky, write_signed_sky_in_dir,
};
use ring::signature::Ed25519KeyPair;
use rusqlite::{Connection, params};
use std::path::PathBuf;
use tempfile::TempDir;

fn seed_foo_depends_on_bar(
    tmp: &TempDir,
    config: &RuntimeConfig,
    keypair: &Ed25519KeyPair,
) -> Result<(PathBuf, PathBuf)> {
    let bar_sky = write_signed_sky_in_dir(
        tmp,
        keypair,
        "bar.sky",
        "bar",
        "1.0.0",
        1,
        b"bar\n",
        vec![],
        "bardir",
    )?;
    let foo_sky = write_signed_sky_in_dir(
        tmp,
        keypair,
        "foo.sky",
        "foo",
        "1.0.0",
        1,
        b"foo\n",
        vec!["bar".to_string()],
        "foodir",
    )?;
    seed_repo_db_with_deps(
        &config.repo_db,
        &[
            ("foo", "1.0.0", 1, foo_sky.to_str().unwrap(), &["bar"]),
            ("bar", "1.0.0", 1, bar_sky.to_str().unwrap(), &[]),
        ],
    );
    Ok((foo_sky, bar_sky))
}

#[test]
fn installing_a_package_records_edges_backed_by_ledger_rows() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;
    let (foo_sky, _bar_sky) = seed_foo_depends_on_bar(&tmp, &config, &keypair)?;

    install(&"foo".to_string(), &config, &tmp.path().join("root"))
        .expect("installing foo should install bar first and then foo");

    let conn = Connection::open(&config.install_db)?;

    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM installed_packages WHERE name = ?1 AND architecture = ?2",
            &[&"foo", &ARCH.to_string()],
        ),
        1,
        "foo must have its own installed row"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM installed_packages WHERE name = ?1 AND architecture = ?2",
            &[&"bar", &ARCH.to_string()],
        ),
        1,
        "bar must be installed before foo so the edge can reference it"
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM installed_packages", &[]),
        2,
        "only foo and bar may be in the ledger"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM package_dependencies WHERE package_name = ?1 AND dependency_name = ?2 AND architecture = ?3",
            &[&"foo", &"bar", &ARCH.to_string()],
        ),
        1,
        "the foo -> bar edge must be recorded"
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM package_dependencies", &[]),
        1,
        "exactly one dependency edge is expected"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM package_dependencies WHERE NOT EXISTS (
            SELECT 1 FROM installed_packages
            WHERE name = package_dependencies.dependency_name
              AND architecture = package_dependencies.architecture
        )",
            &[],
        ),
        0,
        "every recorded edge must point at an installed dependency"
    );

    let ledger_sky_path: String = conn.query_row(
        "SELECT sky_path FROM installed_packages WHERE name = ?1 AND architecture = ?2",
        params!["foo", ARCH.to_string()],
        |row| row.get(0),
    )?;
    assert_eq!(
        ledger_sky_path,
        foo_sky.to_str().unwrap(),
        "the ledger must point at the sky that was installed"
    );

    Ok(())
}

#[test]
fn uninstalling_a_never_installed_package_reports_not_found() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;
    seed_foo_depends_on_bar(&tmp, &config, &keypair)?;

    let err = uninstall(&config, &"bar".to_string(), false)
        .expect_err("bar was never installed, so uninstalling it must fail");
    let chain = format!("{err:#}");
    assert!(chain.contains("not found"), "error chain was: {chain}");
    assert!(
        !chain.contains("depends on"),
        "a package with no ledger row must not trigger the dependent guard: {chain}"
    );

    let err =
        uninstall(&config, &"foo".to_string(), false).expect_err("foo was never installed either");
    let chain = format!("{err:#}");
    assert!(chain.contains("not found"), "error chain was: {chain}");

    let conn = Connection::open(&config.install_db)?;
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM installed_packages", &[]),
        0,
        "failed uninstalls must not have created ledger rows"
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM package_dependencies", &[]),
        0,
        "failed uninstalls must not have created dependency edges"
    );

    Ok(())
}

/// Pins the dependency-side foreign key: the resolver consults only the repo's
/// `deps` column, so a repo that lies — "foo has no deps" while foo's sky
/// declares `bar` — reaches the ledger with bar absent. The FK on
/// `package_dependencies(dependency_name, architecture)` must veto the edge
/// and roll the whole install back.
#[test]
fn install_fails_and_rolls_back_when_the_repo_lies_about_dependencies() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;

    let bar_sky = write_signed_sky_in_dir(
        &tmp,
        &keypair,
        "bar.sky",
        "bar",
        "1.0.0",
        1,
        b"bar\n",
        vec![],
        "bardir",
    )?;
    let foo_sky = write_signed_sky_in_dir(
        &tmp,
        &keypair,
        "foo.sky",
        "foo",
        "1.0.0",
        1,
        b"foo\n",
        vec!["bar".to_string()],
        "foodir",
    )?;
    // The repo DB claims foo has no dependencies, contradicting foo's sky.
    seed_repo_db_with_deps(
        &config.repo_db,
        &[
            ("foo", "1.0.0", 1, foo_sky.to_str().unwrap(), &[]),
            ("bar", "1.0.0", 1, bar_sky.to_str().unwrap(), &[]),
        ],
    );

    let root = tmp.path().join("root");
    install(&"foo".to_string(), &config, &root).expect_err(
        "installing foo must fail: the foo -> bar edge cannot reference bar's ledger row",
    );

    assert!(
        !root.join("foo-1.0.0-1").exists(),
        "the failed install must not leave foo's directory behind"
    );

    let conn = Connection::open(&config.install_db)?;
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM installed_packages", &[]),
        0,
        "the failed install must not leave ledger rows"
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM package_dependencies", &[]),
        0,
        "the failed install must not leave dependency edges"
    );

    Ok(())
}

#[test]
fn failed_fresh_install_preserves_the_existing_root_and_cleans_up() -> Result<()> {
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
        vec!["missing".to_string()],
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

    let a_dir = root.join("A-1.0.0-1");
    let a_file = a_dir.join("dir").join("file.txt");
    assert_eq!(
        std::fs::read_to_string(&a_file)?,
        "a\n",
        "A's file must be installed before the failing install"
    );

    let err = install(&"B".to_string(), &config, &root)
        .expect_err("installing B must fail because its dependency is missing");
    let chain = format!("{err:#}");
    assert!(chain.contains("not found"), "error chain was: {chain}");

    assert!(
        !root.join("B-1.0.0-1").exists(),
        "the failed install must not leave B's placeholder directory behind"
    );
    assert!(a_dir.exists(), "A's install directory must survive");
    assert_eq!(
        std::fs::read_to_string(&a_file)?,
        "a\n",
        "A's installed file must be untouched"
    );

    Ok(())
}
