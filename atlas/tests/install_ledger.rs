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
    ARCH, ConfigEnv, count, generate_keypair, runtime_config, seed_repo_db, tempdir,
    write_signed_sky, write_signed_sky_in_dir,
};
use ring::signature::Ed25519KeyPair;
use rusqlite::{Connection, params};
use std::path::PathBuf;
use tempfile::TempDir;

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

fn seed_foo_with_uninstalled_bar(
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
    seed_repo_db(
        &config.repo_db,
        &[
            ("foo", "1.0.0", 1, foo_sky.to_str().unwrap()),
            ("bar", "1.0.0", 1, bar_sky.to_str().unwrap()),
        ],
    );
    Ok((foo_sky, bar_sky))
}

#[test]
fn installing_a_package_does_not_install_its_dependencies() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;
    let (foo_sky, _bar_sky) = seed_foo_with_uninstalled_bar(&tmp, &config, &keypair)?;

    install(install_args("foo"), &config, tmp.path().join("root"))
        .expect("installing foo should succeed even though bar is not installed");

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
        0,
        "bar must not get a phantom installed row"
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM installed_packages", &[]),
        1,
        "only the explicitly installed package may be in the ledger"
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
fn uninstalling_an_uninstalled_dependency_reports_not_found() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;
    let settings = load_config()?;
    seed_foo_with_uninstalled_bar(&tmp, &config, &keypair)?;

    let root = tmp.path().join("root");
    install(install_args("foo"), &config, root.clone()).expect("installing foo should succeed");

    let err = uninstall(&config, uninstall_args("bar"), &settings)
        .expect_err("bar was never installed, so uninstalling it must fail");
    let chain = format!("{err:#}");
    assert!(chain.contains("not found"), "error chain was: {chain}");
    assert!(
        !chain.contains("depends on"),
        "bar's phantom row must not trigger the dependent guard: {chain}"
    );

    uninstall(&config, uninstall_args("foo"), &settings)
        .expect("uninstalling foo should succeed");

    let err = uninstall(&config, uninstall_args("bar"), &settings)
        .expect_err("bar still was never installed");
    let chain = format!("{err:#}");
    assert!(chain.contains("not found"), "error chain was: {chain}");

    let conn = Connection::open(&config.install_db)?;
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM installed_packages", &[]),
        0,
        "foo's uninstall must not leave bar's phantom row behind"
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
    install(install_args("A"), &config, root.clone()).expect("installing A should succeed");

    let a_dir = root.join("A-1.0.0-1");
    let a_file = a_dir.join("dir").join("file.txt");
    assert_eq!(
        std::fs::read_to_string(&a_file)?,
        "a\n",
        "A's file must be installed before the failing install"
    );

    let err = install(install_args("B"), &config, root.clone())
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
