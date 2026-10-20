mod fixtures;

use anyhow::Result;
use atlas::{cli::cli::InstallArgs, install::install::install};
use fixtures::{
    ConfigEnv, generate_keypair, runtime_config, seed_repo_db, tempdir, write_signed_sky,
};
use rusqlite::Connection;

fn install_args(package: &str) -> InstallArgs {
    InstallArgs {
        package: Some(package.to_string()),
    }
}

fn count(conn: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> i64 {
    conn.query_row(sql, params, |row| row.get(0)).unwrap()
}

#[test]
fn fresh_install_populates_owned_files() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, pub_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &pub_key)?;

    let sky_path = write_signed_sky(&tmp, &keypair, "foo.sky", "foo", "1.0.0", 1, b"foo\n", vec![])?;
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
        2,
        "expected one owned_files row per manifest path"
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

    let sky_path = write_signed_sky(&tmp, &keypair, "foo.sky", "foo", "1.0.0", 1, b"foo\n", vec![])?;
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
        2,
        "re-installing must not duplicate owned_files rows"
    );

    Ok(())
}
