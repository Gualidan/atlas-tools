mod fixtures;

use std::{
    env, fs, io,
    process::{Command, Stdio},
};

use anyhow::Result;
use atlas::{
    cli::cli::UpdateArgs,
    install::{
        db::ensure_install_schema,
        install::{confirmation_prompt, install, set_assume_yes},
    },
    types::runtime_config::RuntimeConfig,
    uninstall::{orphans::uninstall_orphans, uninstall::uninstall},
    update::update::update,
};
use fixtures::{
    ARCH, ConfigEnv, count, generate_keypair, runtime_config, seed_repo_db, seed_repo_db_with_deps,
    tempdir,
};
use rusqlite::{Connection, params};

/// Run one test in its own process with closed stdin and captured stdout. This
/// guarantees no terminal is required, isolates ASSUME_YES, and lets the parent
/// check the package preview without relying on libtest's output capture.
fn without_terminal(
    test_name: &str,
    body: impl FnOnce() -> Result<()>,
    check_output: impl FnOnce(&str),
) -> Result<()> {
    if env::var_os("ATLAS_CONFIRMATION_TEST_CHILD").as_deref() == Some(test_name.as_ref()) {
        return body();
    }

    let output = Command::new(env::current_exe()?)
        .args(["--exact", test_name, "--nocapture", "--color", "never"])
        .env("ATLAS_CONFIRMATION_TEST_CHILD", test_name)
        .stdin(Stdio::null())
        .output()?;
    let stdout = String::from_utf8(output.stdout)?;
    assert!(
        output.status.success(),
        "{test_name}:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("1 passed; 0 failed;"),
        "the selected child test must run: {stdout}"
    );
    check_output(&stdout);
    Ok(())
}

fn seed_installed(config: &RuntimeConfig, name: &str, reason: &str) -> Result<()> {
    let conn = Connection::open(&config.install_db)?;
    ensure_install_schema(&conn)?;
    let dir = config.install_db.parent().unwrap().join(name);
    fs::create_dir(&dir)?;
    fs::write(dir.join("sentinel"), b"original")?;
    conn.execute(
        "INSERT INTO installed_packages
         (name, version, release, architecture, sky_path, install_dir, install_reason)
         VALUES (?, '1.0.0', 1, ?, 'old.sky', ?, ?)",
        params![name, ARCH.to_string(), dir.to_str().unwrap(), reason],
    )?;
    conn.execute(
        "INSERT INTO owned_files VALUES (?, ?, ?)",
        params![format!("{name}/sentinel"), name, ARCH.to_string()],
    )?;
    Ok(())
}

fn assert_installed_unchanged(config: &RuntimeConfig, name: &str) -> Result<()> {
    let conn = Connection::open(&config.install_db)?;
    let (version, dir): (String, String) = conn.query_row(
        "SELECT version, install_dir FROM installed_packages WHERE name = ?",
        [name],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(version, "1.0.0");
    assert_eq!(
        fs::read(std::path::Path::new(&dir).join("sentinel"))?,
        b"original"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM owned_files WHERE package_name = ?",
            &[&name]
        ),
        1
    );
    Ok(())
}

#[test]
fn empty_package_list_needs_no_terminal_or_assume_yes() -> Result<()> {
    without_terminal(
        "empty_package_list_needs_no_terminal_or_assume_yes",
        || {
            set_assume_yes(false);
            assert!(confirmation_prompt(vec![], 4, "install")?);
            Ok(())
        },
        |stdout| assert!(!stdout.contains("Packages:")),
    )
}

#[test]
fn assume_yes_prints_names_versions_and_indentation_and_can_be_disabled() -> Result<()> {
    without_terminal(
        "assume_yes_prints_names_versions_and_indentation_and_can_be_disabled",
        || {
            // A fresh process must require confirmation by default.
            let err = confirmation_prompt(vec![("foo", "1.0.0")], 0, "install")
                .expect_err("a nonempty plan requires a terminal by default");
            assert_eq!(err.to_string(), "Invalid answer try again");

            set_assume_yes(true);
            assert!(confirmation_prompt(vec![("foo", "1.0.0")], 0, "install")?);
            assert!(confirmation_prompt(
                vec![("bar", "2.0.0-rc.1"), ("baz", "3.0.0")],
                4,
                "update"
            )?);

            set_assume_yes(false);
            let err = confirmation_prompt(vec![("foo", "1.0.0")], 4, "uninstall")
                .expect_err("disabling assume_yes must restore terminal confirmation");
            assert_eq!(err.to_string(), "Invalid answer try again");
            Ok(())
        },
        |stdout| {
            assert!(stdout.contains("Packages:\nfoo-1.0.0\n"), "{stdout}");
            assert!(
                stdout.contains("Packages:\n    bar-2.0.0-rc.1\n    baz-3.0.0\n"),
                "{stdout}"
            );
            assert!(
                !stdout.contains("Transaction aborted"),
                "terminal errors are not declined answers"
            );
        },
    )
}

#[test]
fn install_prompt_error_lists_dependencies_and_leaves_no_installation() -> Result<()> {
    without_terminal(
        "install_prompt_error_lists_dependencies_and_leaves_no_installation",
        || {
            let tmp = tempdir();
            let config = runtime_config(&tmp)?;
            let (_, key) = generate_keypair()?;
            let _env = ConfigEnv::new(&tmp, &key)?;
            set_assume_yes(false);
            seed_repo_db_with_deps(
                &config.repo_db,
                &[
                    ("foo", "2.0.0", 1, "missing-foo.sky", &["bar"]),
                    ("bar", "1.0.0", 1, "missing-bar.sky", &[]),
                ],
            );
            let root = tmp.path().join("root");

            let err = install(&"foo".into(), &config, root.clone())
                .expect_err("terminal failure must stop the install before reading archives");

            assert_eq!(err.to_string(), "Invalid answer try again");
            assert!(!root.exists());
            assert!(!config.install_db.exists());
            Ok(())
        },
        |stdout| {
            assert!(
                stdout.contains("Packages:\n    bar-1.0.0\n    foo-2.0.0\n"),
                "{stdout}"
            );
        },
    )
}

#[test]
fn uninstall_prompt_error_preserves_files_and_ledger() -> Result<()> {
    without_terminal(
        "uninstall_prompt_error_preserves_files_and_ledger",
        || {
            let tmp = tempdir();
            let config = runtime_config(&tmp)?;
            seed_installed(&config, "foo", "explicit")?;

            let err = uninstall(&config, &"foo".into(), true)
                .expect_err("terminal failure must stop removal");

            assert_eq!(err.to_string(), "Invalid answer try again");
            assert_installed_unchanged(&config, "foo")
        },
        |stdout| assert!(stdout.contains("Packages:\n    foo-1.0.0\n"), "{stdout}"),
    )
}

#[test]
fn uninstall_can_skip_its_prompt_without_global_assume_yes() -> Result<()> {
    without_terminal(
        "uninstall_can_skip_its_prompt_without_global_assume_yes",
        || {
            let tmp = tempdir();
            let config = runtime_config(&tmp)?;
            seed_installed(&config, "foo", "explicit")?;

            uninstall(&config, &"foo".into(), false)?;

            assert!(!tmp.path().join("foo").exists());
            let conn = Connection::open(&config.install_db)?;
            assert_eq!(
                count(&conn, "SELECT COUNT(*) FROM installed_packages", &[]),
                0
            );
            assert_eq!(count(&conn, "SELECT COUNT(*) FROM owned_files", &[]), 0);
            Ok(())
        },
        |stdout| assert!(!stdout.contains("Packages:")),
    )
}

#[test]
fn clean_with_no_orphans_does_not_require_confirmation() -> Result<()> {
    without_terminal(
        "clean_with_no_orphans_does_not_require_confirmation",
        || {
            let tmp = tempdir();
            let config = runtime_config(&tmp)?;
            seed_installed(&config, "explicit", "explicit")?;

            uninstall_orphans(&config)?;

            assert_installed_unchanged(&config, "explicit")
        },
        |stdout| {
            assert!(
                stdout.contains("No orphaned packages to remove"),
                "{stdout}"
            );
            assert!(!stdout.contains("Packages:"));
        },
    )
}

#[test]
fn clean_prompt_error_lists_all_orphans_and_removes_nothing() -> Result<()> {
    without_terminal(
        "clean_prompt_error_lists_all_orphans_and_removes_nothing",
        || {
            let tmp = tempdir();
            let config = runtime_config(&tmp)?;
            seed_installed(&config, "alpha", "dependency")?;
            seed_installed(&config, "zeta", "dependency")?;
            seed_installed(&config, "explicit", "explicit")?;

            let err = uninstall_orphans(&config)
                .expect_err("terminal failure must stop the entire cleanup");

            assert_eq!(err.to_string(), "Invalid answer try again");
            for name in ["alpha", "zeta", "explicit"] {
                assert_installed_unchanged(&config, name)?;
            }
            Ok(())
        },
        |stdout| {
            assert_eq!(stdout.matches("Packages:").count(), 1, "{stdout}");
            assert!(stdout.contains("    alpha-1.0.0\n"), "{stdout}");
            assert!(stdout.contains("    zeta-1.0.0\n"), "{stdout}");
            assert!(!stdout.contains("    explicit-1.0.0\n"), "{stdout}");
            assert!(!stdout.contains("Uninstalling"), "{stdout}");
        },
    )
}

#[test]
fn clean_with_assume_yes_removes_orphans_and_keeps_explicit_packages() -> Result<()> {
    without_terminal(
        "clean_with_assume_yes_removes_orphans_and_keeps_explicit_packages",
        || {
            let tmp = tempdir();
            let config = runtime_config(&tmp)?;
            seed_installed(&config, "alpha", "dependency")?;
            seed_installed(&config, "zeta", "dependency")?;
            seed_installed(&config, "explicit", "explicit")?;
            set_assume_yes(true);

            uninstall_orphans(&config)?;

            for name in ["alpha", "zeta"] {
                assert!(!tmp.path().join(name).exists());
            }
            let conn = Connection::open(&config.install_db)?;
            assert_eq!(
                count(&conn, "SELECT COUNT(*) FROM installed_packages", &[]),
                1
            );
            assert_eq!(count(&conn, "SELECT COUNT(*) FROM owned_files", &[]), 1);
            assert_installed_unchanged(&config, "explicit")
        },
        |stdout| {
            assert_eq!(
                stdout.matches("Packages:").count(),
                1,
                "per-package prompts must be skipped: {stdout}"
            )
        },
    )
}

#[test]
fn current_single_and_bulk_updates_skip_confirmation() -> Result<()> {
    without_terminal(
        "current_single_and_bulk_updates_skip_confirmation",
        || {
            let tmp = tempdir();
            let config = runtime_config(&tmp)?;
            seed_installed(&config, "foo", "explicit")?;
            seed_repo_db(&config.repo_db, &[("foo", "1.0.0", 99, "missing.sky")]);

            update(
                &config,
                UpdateArgs {
                    package: Some("foo".into()),
                },
            )?;
            update(&config, UpdateArgs { package: None })?;

            assert_installed_unchanged(&config, "foo")
        },
        |stdout| {
            assert!(
                stdout.contains("Package foo is already up to date"),
                "{stdout}"
            );
            assert!(stdout.contains("All packages are up to date"), "{stdout}");
            assert!(!stdout.contains("Packages:"));
        },
    )
}

#[test]
fn update_prompt_error_lists_only_upgrades_and_leaves_files_untouched() -> Result<()> {
    without_terminal(
        "update_prompt_error_lists_only_upgrades_and_leaves_files_untouched",
        || {
            let tmp = tempdir();
            let config = runtime_config(&tmp)?;
            seed_installed(&config, "foo", "explicit")?;
            seed_installed(&config, "current", "explicit")?;
            seed_repo_db(
                &config.repo_db,
                &[
                    ("foo", "2.0.0", 7, "missing-foo.sky"),
                    ("current", "1.0.0", 99, "missing-current.sky"),
                    ("uninstalled", "3.0.0", 1, "missing-uninstalled.sky"),
                ],
            );
            let entries_before = fs::read_dir(tmp.path())?
                .map(|entry| entry.map(|entry| entry.file_name()))
                .collect::<io::Result<std::collections::BTreeSet<_>>>()?;

            for package in [Some("foo".into()), None] {
                let err = update(&config, UpdateArgs { package })
                    .expect_err("terminal failure must stop updates before staging");
                assert_eq!(err.to_string(), "Invalid answer try again");
            }

            assert_installed_unchanged(&config, "foo")?;
            assert_installed_unchanged(&config, "current")?;
            let entries_after = fs::read_dir(tmp.path())?
                .map(|entry| entry.map(|entry| entry.file_name()))
                .collect::<io::Result<std::collections::BTreeSet<_>>>()?;
            assert_eq!(
                entries_after, entries_before,
                "no staging directories may remain"
            );
            Ok(())
        },
        |stdout| {
            assert_eq!(
                stdout.matches("Packages:\n    foo-2.0.0\n").count(),
                2,
                "{stdout}"
            );
            assert!(!stdout.contains("    current-"), "{stdout}");
            assert!(!stdout.contains("    uninstalled-"), "{stdout}");
        },
    )
}
