//! Integration tests for the sandbox `/usr` assembly step in
//! `sisyphus::build::build`: every build sandbox gets the published
//! `base-devel` group (hardcoded constant name) plus its dependency closure
//! installed into the sandbox `/usr` root, alongside the extracted
//! dependency artifacts, before any phase script runs.
//!
//! Each test owns a [`fixtures::Fixture`] holding the process-wide
//! environment lock, so the tests run effectively serialized.

mod base_devel_group_fixture;
mod fixtures;

use std::{fs, path::PathBuf};

use anyhow::Result;
use base_devel_group_fixture::Fixture;
use common::types::package::Architectures;
use fixtures::package;
use tempfile::{Builder as TempBuilder, TempDir};

/// A throwaway sandbox `/usr` root with the shape `build_package` gives
/// `extract_artifacts` today: an unnamed-but-named-prefix tempdir whose
/// lifetime the caller owns.
fn usr_root() -> Result<TempDir> {
    Ok(TempBuilder::new().prefix("sandbox_usr_").tempdir()?)
}

/// Case 1: the sandbox usr assembly installs the published `base-devel` group
/// into the usr root — with no dependency artifacts at all, each group
/// member's payload file exists under the usr root with the right contents,
/// and the crate's staging directory (`usr_root/var/lib/atlas/tmp`) is left
/// existing but empty.
#[test]
fn group_members_land_in_usr_root() -> Result<()> {
    let fixture = Fixture::new()?;
    fixture.publish_in_groups("alpha", "1.0.0", 1, &[], &["base-devel"])?;
    fixture.publish_in_groups("beta", "2.0.0", 1, &[], &["base-devel"])?;
    let usr = usr_root()?;

    sisyphus::build::build::assemble_sandbox_usr(&[], fixture.repo_db(), &usr)?;

    assert_eq!(
        fs::read(usr.path().join("alpha/1.0.0.txt"))?,
        b"alpha 1.0.0\n",
        "alpha group-member payload must land in the sandbox usr root"
    );
    assert_eq!(
        fs::read(usr.path().join("beta/2.0.0.txt"))?,
        b"beta 2.0.0\n",
        "beta group-member payload must land in the sandbox usr root"
    );

    // The install staging directory under the usr root must exist and hold
    // no leftovers after the group install.
    let staging = usr.path().join("var/lib/atlas/tmp");
    assert!(
        fs::read_dir(&staging)?.next().is_none(),
        "staging directory {} must be empty after the group install",
        staging.display()
    );
    Ok(())
}

/// Case 2: the sandbox usr assembly keeps dependency artifacts alongside the
/// group's files — a real signed `.sky` that is NOT a group member is
/// extracted into the usr root via the artifact list, and after the
/// assembly both the artifact's payload file and every group member's
/// payload file coexist under the usr root with distinct, correct contents.
#[test]
fn artifacts_and_group_files_coexist_in_usr_root() -> Result<()> {
    let fixture = Fixture::new()?;
    // `dep` is only an artifact input, never a group member; the group
    // members must not clobber it and it must not clobber them.
    fixture.publish_in_groups("dep", "0.9.0", 1, &[], &[])?;
    fixture.publish_in_groups("alpha", "1.0.0", 1, &[], &["base-devel"])?;
    fixture.publish_in_groups("beta", "2.0.0", 1, &[], &["base-devel"])?;

    // Re-publish `dep` outside the repo layout to get a standalone artifact
    // path: the artifact list carries raw `.sky` paths, like the artifacts
    // `build_package` passes to `extract_artifacts` today.
    let dep_artifact = fixture.publish_in_groups("dep", "0.9.0", 1, &[], &[])?;
    assert!(
        dep_artifact.exists(),
        "the artifact .sky must exist at its repo-layout path"
    );
    let usr = usr_root()?;

    sisyphus::build::build::assemble_sandbox_usr(
        &[PathBuf::from(&dep_artifact)],
        fixture.repo_db(),
        &usr,
    )?;

    assert_eq!(
        fs::read(usr.path().join("dep/0.9.0.txt"))?,
        b"dep 0.9.0\n",
        "the dependency artifact payload must land in the sandbox usr root"
    );
    assert_eq!(
        fs::read(usr.path().join("alpha/1.0.0.txt"))?,
        b"alpha 1.0.0\n",
        "the group member payload must survive the artifact extraction"
    );
    assert_eq!(
        fs::read(usr.path().join("beta/2.0.0.txt"))?,
        b"beta 2.0.0\n",
        "the second group member payload must survive the artifact extraction"
    );
    Ok(())
}

/// Case 3: a missing or unresolvable `base-devel` group fails the usr
/// assembly with an error whose chain names the group — with a repository DB
/// holding only a package in another group, `assemble_sandbox_usr` must
/// return an error containing `base-devel`, and — with no artifacts to
/// extract — the usr root must receive no group files at all.
#[test]
fn missing_group_fails_usr_assembly_naming_the_group() -> Result<()> {
    let fixture = Fixture::new()?;
    // The DB resolves, but only for a package outside `base-devel`.
    fixture.publish_in_groups("alpha", "1.0.0", 1, &[], &["other"])?;
    let usr = usr_root()?;

    let err = sisyphus::build::build::assemble_sandbox_usr(&[], fixture.repo_db(), &usr)
        .expect_err("a missing base-devel group must fail the usr assembly");
    let chain = format!("{err:#}");

    assert!(
        chain.contains("base-devel"),
        "error chain must name the group; chain was: {chain}"
    );

    // With an empty artifact list the only thing that may exist under the
    // usr root is the directory `extract_artifacts` creates (`usr`), which
    // must itself be empty: no group member payload may have landed.
    let entries: Vec<_> = fs::read_dir(usr.path())?
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .map(|e| e.file_name())
        .collect();
    assert!(
        entries.iter().all(|name| name == "usr"),
        "usr root must hold no group or artifact files on failure; got {entries:?}"
    );
    assert!(
        fs::read_dir(usr.path().join("usr"))?.next().is_none(),
        "the extracted usr directory must be empty when no artifacts were given"
    );
    Ok(())
}

/// Case 4: `build_package` aborts with the `base-devel` group-install error
/// before any phase script runs — with the group missing from the runtime
/// repo DB, the returned error chain must name `base-devel` and must NOT
/// contain bubblewrap / phase-failure wording, proving the abort happens
/// ahead of the prepare/build/check/package phases.
#[test]
fn build_package_aborts_before_phases_when_group_missing() -> Result<()> {
    let fixture = Fixture::new()?;
    fixture.publish_in_groups("alpha", "1.0.0", 1, &[], &["other"])?;
    let pkg = package("todefine", "1.0.0", Architectures::X86_64, vec![]);
    let srcdir = TempBuilder::new().prefix("srcdir_").tempdir()?;

    // Note the chosen contract shape: `repo_db: &Path` as the first
    // parameter, mirroring `assemble_sandbox_usr`.
    let err = sisyphus::build::build::build_package(&fixture.repo_db(), &pkg, srcdir.path(), &[])
        .expect_err("a missing base-devel group must abort the build before any phase");
    let chain = format!("{err:#}");

    assert!(
        chain.contains("base-devel"),
        "error chain must name the group; chain was: {chain}"
    );
    assert!(
        !chain.contains("Failed to run package phase") && !chain.contains("bwrap"),
        "the abort must happen before any phase runs; chain was: {chain}"
    );
    Ok(())
}
