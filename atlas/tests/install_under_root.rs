mod fixtures;

use std::{
    collections::BTreeMap,
    fs::{self, File},
    os::unix::fs::symlink,
    path::{Path, PathBuf},
};

use anyhow::Result;
use atlas::{
    cli::cli::UpdateArgs, install::db::ensure_install_schema, install::install::install,
    types::runtime_config::RuntimeConfig, uninstall::uninstall::uninstall, update::update::update,
};
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

fn config_with_install_db(workspace: &Workspace, install_db: PathBuf) -> RuntimeConfig {
    RuntimeConfig {
        repo_db: workspace.config.repo_db.clone(),
        install_db,
        sky_repo: workspace.config.sky_repo.clone(),
    }
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

#[test]
fn installed_state_is_independent_between_target_roots() -> Result<()> {
    let workspace = Workspace::new()?;
    let root_one = workspace.create_target_root("root-one")?;
    let root_two = workspace.create_target_root("root-two")?;
    let install_db_one = root_one.join("var/lib/atlas/installed.db");
    let install_db_two = root_two.join("var/lib/atlas/installed.db");
    let config_one = config_with_install_db(&workspace, install_db_one.clone());
    let config_two = config_with_install_db(&workspace, install_db_two.clone());

    workspace.publish_with_entries(
        "app",
        "1.0.0",
        1,
        &[],
        &[PayloadEntry::file("usr/bin/tool", b"version one")],
    )?;
    install(&"app".to_string(), &config_one, &root_one)?;
    install(&"app".to_string(), &config_two, &root_two)?;

    workspace.publish_with_entries(
        "app",
        "2.0.0",
        2,
        &[],
        &[PayloadEntry::file("usr/bin/tool", b"version two")],
    )?;
    update(
        &config_one,
        &UpdateArgs {
            package: Some("app".into()),
        },
    )?;

    assert_eq!(fs::read(root_one.join("usr/bin/tool"))?, b"version two");
    assert_eq!(fs::read(root_two.join("usr/bin/tool"))?, b"version one");
    assert_eq!(
        Connection::open(&install_db_one)?.query_row(
            "SELECT version FROM installed_packages WHERE name = 'app'",
            [],
            |row| row.get::<_, String>(0),
        )?,
        "2.0.0"
    );
    assert_eq!(
        Connection::open(&install_db_two)?.query_row(
            "SELECT version FROM installed_packages WHERE name = 'app'",
            [],
            |row| row.get::<_, String>(0),
        )?,
        "1.0.0"
    );

    uninstall(&config_one, &"app".to_string(), false)?;
    assert!(!root_one.join("usr/bin/tool").exists());
    assert_eq!(fs::read(root_two.join("usr/bin/tool"))?, b"version one");
    assert_eq!(
        count(
            &Connection::open(&install_db_two)?,
            "SELECT COUNT(*) FROM installed_packages WHERE name = 'app'",
            &[],
        ),
        1
    );

    Ok(())
}

#[test]
fn system_install_records_are_shared_between_user_configs() -> Result<()> {
    let workspace = Workspace::new()?;
    let system_root = workspace.create_target_root("system-root")?;
    let shared_install_db = system_root.join("var/lib/atlas/installed.db");
    let user_two_repo = workspace
        .create_target_root("user-two-data")?
        .join("repo.db");

    let user_one_config = config_with_install_db(&workspace, shared_install_db.clone());
    let user_two_config = RuntimeConfig {
        repo_db: user_two_repo.clone(),
        install_db: shared_install_db.clone(),
        sky_repo: workspace.config.sky_repo.clone(),
    };
    workspace.publish_with_entries(
        "shared-app",
        "1.0.0",
        1,
        &[],
        &[PayloadEntry::file("usr/bin/shared-tool", b"shared")],
    )?;
    fs::copy(&workspace.config.repo_db, &user_two_repo)?;

    install(&"shared-app".to_string(), &user_one_config, &system_root)?;
    install(&"shared-app".to_string(), &user_two_config, &system_root)?;

    let conn = Connection::open(&shared_install_db)?;
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM installed_packages WHERE name = 'shared-app'",
            &[],
        ),
        1,
        "both invocations must use the same system install record"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM owned_files WHERE package_name = 'shared-app'",
            &[],
        ),
        1
    );
    assert_eq!(
        fs::read(system_root.join("usr/bin/shared-tool"))?,
        b"shared"
    );

    Ok(())
}

#[test]
fn package_cannot_overwrite_atlas_state_under_target_root() -> Result<()> {
    let workspace = Workspace::new()?;
    let root = workspace.root();
    let state_db = root.join("var/lib/atlas/installed.db");
    fs::create_dir_all(state_db.parent().unwrap())?;
    let conn = Connection::open(&state_db)?;
    ensure_install_schema(&conn)?;
    conn.execute_batch(
        "CREATE TABLE state_sentinel (value TEXT NOT NULL);
         INSERT INTO state_sentinel VALUES ('preserve me');",
    )?;
    drop(conn);

    let config = config_with_install_db(&workspace, state_db.clone());
    workspace.publish_with_entries(
        "state-overwriter",
        "1.0.0",
        1,
        &[],
        &[PayloadEntry::file(
            "var/lib/atlas/installed.db",
            b"package payload must not replace Atlas state",
        )],
    )?;

    let err = install(
        &"state-overwriter".to_string(),
        &config,
        &root.to_path_buf(),
    )
    .expect_err("Atlas state paths must be reserved");
    assert!(
        format!("{err:#}").to_lowercase().contains("conflict")
            || format!("{err:#}").to_lowercase().contains("reserved"),
        "unexpected state-path error: {err:#}"
    );
    let conn = Connection::open(state_db)?;
    let sentinel: String =
        conn.query_row("SELECT value FROM state_sentinel", [], |row| row.get(0))?;
    assert_eq!(sentinel, "preserve me");
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM installed_packages WHERE name = 'state-overwriter'",
            &[],
        ),
        0
    );

    Ok(())
}

#[test]
fn target_root_escape_attempts_leave_outside_paths_unchanged() -> Result<()> {
    let tmp = tempdir();
    let config = runtime_config(&tmp)?;
    let (keypair, public_key) = generate_keypair()?;
    let _env = ConfigEnv::new(&tmp, &public_key)?;
    let root = tmp.path().join("root");
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&root)?;
    fs::create_dir_all(&outside)?;
    fs::write(outside.join("sentinel"), b"outside data")?;

    let traversal_sky = write_signed_sky_with_raw_entry(
        &tmp,
        &keypair,
        "traversal.sky",
        "traversal",
        "1.0.0",
        1,
        "../escape",
        0o644,
        EntryType::file(),
        b"must not escape",
    )?;
    let absolute_target = tmp.path().join("absolute-output");
    let absolute_sky = write_signed_sky_with_raw_entry(
        &tmp,
        &keypair,
        "absolute.sky",
        "absolute",
        "1.0.0",
        1,
        absolute_target.to_str().unwrap(),
        0o644,
        EntryType::file(),
        b"must not escape",
    )?;
    let symlink_sky = write_signed_sky_with_entries(
        &tmp,
        &keypair,
        "symlink-parent.sky",
        "symlink-parent",
        "1.0.0",
        1,
        vec![],
        &[PayloadEntry::file("usr/bin/tool", b"must stay in root")],
    )?;
    seed_repo_db(
        &config.repo_db,
        &[
            ("traversal", "1.0.0", 1, traversal_sky.to_str().unwrap()),
            ("absolute", "1.0.0", 1, absolute_sky.to_str().unwrap()),
            ("symlink-parent", "1.0.0", 1, symlink_sky.to_str().unwrap()),
        ],
    );

    symlink(&outside, root.join("usr"))?;
    let target_before = snapshot_tree(&root)?;
    for package in ["traversal", "absolute", "symlink-parent"] {
        let _err = install(&package.to_string(), &config, &root)
            .expect_err("target-root escapes must be rejected");
        assert_eq!(
            snapshot_tree(&root)?,
            target_before,
            "{package} changed root"
        );
        assert_eq!(fs::read(outside.join("sentinel"))?, b"outside data");
        assert!(!outside.join("bin/tool").exists());
    }

    assert!(!tmp.path().join("escape").exists());
    assert!(!absolute_target.exists());
    assert_eq!(fs::read_link(root.join("usr"))?, outside);
    let conn = Connection::open(&config.install_db)?;
    for package in ["traversal", "absolute", "symlink-parent"] {
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM installed_packages WHERE name = ?1",
                &[&package],
            ),
            0,
            "{package} must not be recorded as installed"
        );
    }

    Ok(())
}
