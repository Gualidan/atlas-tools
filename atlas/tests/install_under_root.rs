mod fixtures;

use std::{
    collections::BTreeMap,
    fs::{self, File},
    path::{Path, PathBuf},
};

use anyhow::Result;
use atlas::install::install::install;
use common::functions::sky_verify::data_to_sign;
use fixtures::*;
use ring::signature::Ed25519KeyPair;
use rusqlite::Connection;
use tar::EntryType;
use tempfile::TempDir;

#[derive(Debug, PartialEq, Eq)]
enum TreeEntry {
    Directory,
    File(Vec<u8>),
    Symlink(PathBuf),
    Other,
}

fn snapshot_tree(root: &Path) -> Result<BTreeMap<PathBuf, TreeEntry>> {
    let mut snapshot = BTreeMap::new();
    for entry in walkdir::WalkDir::new(root).min_depth(1) {
        let entry = entry?;
        let relative_path = entry.path().strip_prefix(root)?.to_path_buf();
        let entry_type = entry.file_type();
        let snapshot_entry = if entry_type.is_dir() {
            TreeEntry::Directory
        } else if entry_type.is_symlink() {
            TreeEntry::Symlink(fs::read_link(entry.path())?)
        } else if entry_type.is_file() {
            TreeEntry::File(fs::read(entry.path())?)
        } else {
            TreeEntry::Other
        };
        snapshot.insert(relative_path, snapshot_entry);
    }
    Ok(snapshot)
}

fn write_sky_for_case(
    tmp: &TempDir,
    keypair: &Ed25519KeyPair,
    package_name: &str,
    payload: &[u8],
    manifest: &str,
    sign_correctly: bool,
) -> Result<PathBuf> {
    let metadata = serde_saphyr::to_string(&metadata_for(package_name, "1.0.0", 1, vec![]))?;
    let signature = if sign_correctly {
        let data_to_sign = data_to_sign(metadata.as_bytes(), payload, manifest.as_bytes())?;
        keypair.sign(&data_to_sign).as_ref().to_vec()
    } else {
        vec![0; 64]
    };
    let sky_path = tmp.path().join(format!("{package_name}.sky"));
    write_sky(
        &sky_path,
        payload,
        metadata.as_bytes(),
        manifest,
        &signature,
    )?;
    Ok(sky_path)
}

#[test]
fn installs_at_target_root_path() -> Result<()> {
    let workspace = Workspace::new()?;
    let root = workspace.root();
    let entries = vec![PayloadEntry::File {
        path: "usr/bin/tool".to_string(),
        mode: 0o755,
        contents: vec![],
    }];

    workspace.publish_with_entries("app", "1.0.0", 1, &[], &entries)?;

    workspace.install("app")?;

    assert!(root.join("usr/bin/tool").exists());
    assert!(!root.join("var/lib/atlas/local/app-1.0.0-1").exists());

    Ok(())
}

#[test]
fn multiple_packages_coexist() -> Result<()> {
    let workspace = Workspace::new()?;
    let root = workspace.root();
    let app_entries = vec![PayloadEntry::File {
        path: "usr/bin/tool".to_string(),
        mode: 0o755,
        contents: vec![],
    }];
    let app2_entry = vec![PayloadEntry::File {
        path: "usr/bin/tool2".to_string(),
        mode: 0o755,
        contents: vec![],
    }];

    workspace.publish_with_entries("app", "1.0.0", 1, &[], &app_entries)?;
    workspace.publish_with_entries("app2", "1.0.0", 1, &[], &app2_entry)?;

    workspace.install("app")?;
    workspace.install("app2")?;

    assert!(root.join("usr/bin/tool").exists());
    assert!(root.join("usr/bin/tool2").exists());
    assert!(!root.join("var/lib/atlas/local/app-1.0.0-1").exists());
    assert!(!root.join("var/lib/atlas/local/app2-1.0.0-1").exists());

    Ok(())
}

#[test]
fn unmanaged_files_are_protected() -> Result<()> {
    let workspace = Workspace::new()?;
    let conn = workspace.install_conn()?;
    let root = workspace.root();
    let entries = vec![PayloadEntry::File {
        path: "usr/bin/tool".to_string(),
        mode: 0o755,
        contents: vec![],
    }];
    let unrelated_conflicting_file = root.join("usr/bin/tool");
    File::create(&unrelated_conflicting_file)?;
    owned_paths(&conn, "app").expect_err("database should be empty");

    workspace.publish_with_entries("app", "1.0.0", 1, &[], &entries)?;
    let err = workspace
        .install("app")
        .expect_err("install should fail because of a conflicting file");
    assert_eq!(
        err.to_string(),
        "Failed to install `app`: conflicting file `usr/bin/tool` already exists"
    );

    assert!(unrelated_conflicting_file.exists());
    assert!(!root.join("var/lib/atlas/local/app-1.0.0-1").exists());
    owned_paths(&conn, "app").expect_err("database should still be empty");

    Ok(())
}

#[test]
fn conflicting_package_files_are_protected() -> Result<()> {
    let workspace = Workspace::new()?;
    let root = workspace.root();
    let conn = workspace.install_conn()?;
    let entries = vec![PayloadEntry::File {
        path: "usr/bin/tool".to_string(),
        mode: 0o755,
        contents: vec![],
    }];

    workspace.publish_with_entries("app", "1.0.0", 1, &[], &entries)?;
    workspace.publish_with_entries("base", "1.0.0", 1, &[], &entries)?;
    workspace.install("app")?;
    let app_owned_files = owned_paths(&conn, "app")?;

    let err = workspace
        .install("base")
        .expect_err("install should fail because of a conflicting file");
    assert_eq!(
        err.to_string(),
        "Failed to install `base`: conflicting file of package `app` already exists"
    );

    assert!(root.join("usr/bin/tool").exists());
    assert!(!root.join("var/lib/atlas/local/app-1.0.0-1").exists());
    assert_eq!(app_owned_files, owned_paths(&conn, "app")?);

    Ok(())
}

#[test]
#[allow(clippy::too_many_lines)]
fn invalid_packages_do_not_modify_target_or_install_records() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, public_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &public_key)?;
    let root = tmp.path().join("root");
    fs::create_dir(&root)?;
    fs::write(root.join("preserved.txt"), b"existing target data")?;

    let entries = [PayloadEntry::file("usr/bin/invalid", b"payload")];
    let (payload, valid_manifest) = build_payload_and_manifest(&entries)?;

    let invalid_signature = write_sky_for_case(
        &tmp,
        &keypair,
        "invalid-signature",
        &payload,
        &valid_manifest,
        false,
    )?;
    let malformed_manifest = write_sky_for_case(
        &tmp,
        &keypair,
        "malformed-manifest",
        &payload,
        "not a valid manifest record\n",
        true,
    )?;
    let unsafe_path = write_signed_sky_with_raw_entry(
        &tmp,
        &keypair,
        "unsafe-path.sky",
        "unsafe-path",
        "1.0.0",
        1,
        "../escape",
        0o644,
        EntryType::file(),
        b"unsafe",
    )?;
    let invalid_payload = write_signed_sky_with_raw_entry(
        &tmp,
        &keypair,
        "invalid-payload.sky",
        "invalid-payload",
        "1.0.0",
        1,
        "usr/bin/pipe",
        0o644,
        EntryType::fifo(),
        b"",
    )?;

    seed_repo_db(
        &config.repo_db,
        &[
            (
                "invalid-signature",
                "1.0.0",
                1,
                invalid_signature.to_str().unwrap(),
            ),
            (
                "malformed-manifest",
                "1.0.0",
                1,
                malformed_manifest.to_str().unwrap(),
            ),
            ("unsafe-path", "1.0.0", 1, unsafe_path.to_str().unwrap()),
            (
                "invalid-payload",
                "1.0.0",
                1,
                invalid_payload.to_str().unwrap(),
            ),
        ],
    );

    let target_before = snapshot_tree(&root)?;

    for package in [
        "invalid-signature",
        "malformed-manifest",
        "unsafe-path",
        "invalid-payload",
    ] {
        let _err = install(&package.to_string(), &config, &root)
            .expect_err("invalid package must be rejected");
        assert_eq!(
            snapshot_tree(&root)?,
            target_before,
            "{package} changed target files"
        );

        let conn = Connection::open(&config.install_db)?;
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM installed_packages WHERE name = ?1",
                &[&package],
            ),
            0,
            "{package} must not leave an installed package record"
        );
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM owned_files WHERE package_name = ?1",
                &[&package],
            ),
            0,
            "{package} must not leave owned-file records"
        );
    }

    Ok(())
}
