//! Integration tests for `sandbox_install::install_group_into_root`.
//!
//! Each test owns a [`fixtures::Fixture`] holding the process-wide
//! environment lock, so the tests run effectively serialized.

mod fixtures;

use std::fs;

use anyhow::Result;
use atlas::{
    cli::confirmation::{assume_yes, set_assume_yes},
    types::runtime_config::RuntimeConfig,
};
use fixtures::Fixture;
use sandbox_install::install_group_into_root;

/// 1. Files from a group's `.sky` packages exist under `install_root`.
#[test]
fn group_files_land_under_install_root() -> Result<()> {
    let fixture = Fixture::new()?;
    fixture.publish_in_groups("alpha", "1.0.0", 1, &[], &["tools"])?;
    fixture.publish_in_groups("beta", "2.0.0", 1, &[], &["tools"])?;

    install_group_into_root("tools", &fixture.repo_db(), fixture.root())?;

    assert_eq!(
        fs::read(fixture.root().join("alpha/1.0.0.txt"))?,
        b"alpha 1.0.0\n",
        "alpha payload"
    );
    assert_eq!(
        fs::read(fixture.root().join("beta/2.0.0.txt"))?,
        b"beta 2.0.0\n",
        "beta payload"
    );

    // Staging happens under install_root/var/lib/atlas/tmp; no staging
    // directories may survive the call.
    let staging = fixture.root().join("var/lib/atlas/tmp");
    assert!(
        fs::read_dir(&staging)?.next().is_none(),
        "staging directory {} must be empty after the install",
        staging.display()
    );
    Ok(())
}

/// Case 2: the dependency closure installs before the group members. A group
/// member that fails verification must not prevent — or undo — the
/// installation of the dependency closure that resolves before it.
///
/// The install database is a throwaway file inside a scratch directory that
/// dies with the call, so install reasons and commit order are not
/// observable from outside; this test pins the observable half of the
/// dependency-first contract instead.
#[test]
fn dependency_closure_installs_before_group_members() -> Result<()> {
    let fixture = Fixture::new()?;
    // `lib` is in the dependency closure of both members but is not a group
    // member, so resolution always pushes it first, whichever member is
    // visited first.
    fixture.publish_in_groups("lib", "1.0.0", 1, &[], &[])?;
    fixture.publish_in_groups("app", "1.0.0", 1, &["lib"], &["tools"])?;
    fixture.publish_broken_in_groups("broken", "1.0.0", 1, &["lib"], &["tools"])?;

    let result = install_group_into_root("tools", &fixture.repo_db(), fixture.root());

    assert!(
        result.is_err(),
        "the broken member must fail signature verification"
    );
    assert_eq!(
        fs::read(fixture.root().join("lib/1.0.0.txt"))?,
        b"lib 1.0.0\n",
        "the dependency closure must be installed even though a group member failed"
    );
    assert!(
        !fixture.root().join("broken/1.0.0.txt").exists(),
        "the failing member must not leave files behind"
    );
    Ok(())
}

/// Case 3: non-interactive — the call succeeds outside a TTY with assume-yes
/// initially off (no prompt is attempted), and restores the caller's
/// assume-yes value on success.
#[test]
fn non_interactive_without_tty_and_restores_assume_yes() -> Result<()> {
    let fixture = Fixture::new()?;
    fixture.publish_in_groups("alpha", "1.0.0", 1, &[], &["tools"])?;
    set_assume_yes(false);

    install_group_into_root("tools", &fixture.repo_db(), fixture.root())?;

    assert_eq!(
        fs::read(fixture.root().join("alpha/1.0.0.txt"))?,
        b"alpha 1.0.0\n"
    );
    assert!(
        !assume_yes(),
        "assume-yes must be restored to the caller's value after the call"
    );
    Ok(())
}

/// Case 4: the host install DB (the one `RuntimeConfig::build` derives from
/// the user's data directory) is never opened for writing: if it exists its
/// bytes are unchanged, and if it does not exist it is not created.
#[test]
fn host_install_db_is_never_touched() -> Result<()> {
    let fixture = Fixture::new()?;
    fixture.publish_in_groups("alpha", "1.0.0", 1, &[], &["tools"])?;

    let host_db = RuntimeConfig::build()?.install_db;
    let before = if host_db.exists() {
        Some(fs::read(&host_db)?)
    } else {
        None
    };

    install_group_into_root("tools", &fixture.repo_db(), fixture.root())?;

    match before {
        Some(before) => assert_eq!(
            fs::read(&host_db)?,
            before,
            "host install db at {} must be byte-identical",
            host_db.display()
        ),
        None => assert!(
            !host_db.exists(),
            "host install db at {} must not be created",
            host_db.display()
        ),
    }
    Ok(())
}

/// Case 5: unknown group → error naming the group, and assume-yes restored on
/// the error path too.
#[test]
fn unknown_group_names_the_group() -> Result<()> {
    let fixture = Fixture::new()?;
    fixture.publish_in_groups("alpha", "1.0.0", 1, &[], &["other"])?;
    set_assume_yes(false);

    let err = install_group_into_root("nonexistent-group", &fixture.repo_db(), fixture.root())
        .expect_err("an unknown group must be rejected");
    let chain = format!("{err:#}");

    assert!(
        chain.contains("nonexistent-group"),
        "error chain was: {chain}"
    );
    assert!(
        !assume_yes(),
        "assume-yes must be restored to the caller's value on the error path"
    );
    Ok(())
}

/// 6. An empty repository database → the resolution error is propagated.
#[test]
fn empty_repo_db_propagates_resolution_error() -> Result<()> {
    let fixture = Fixture::new()?;

    let err = install_group_into_root("tools", &fixture.repo_db(), fixture.root())
        .expect_err("an empty repository database must fail resolution");
    let chain = format!("{err:#}");

    assert!(
        chain.contains("Failed to resolve group"),
        "error chain was: {chain}"
    );
    Ok(())
}
