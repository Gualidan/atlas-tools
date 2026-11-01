mod fixtures;

use anyhow::Result;
use atlas::{cli::cli::UpdateArgs, update::update::update};
use fixtures::Workspace;

use crate::fixtures::PayloadEntry;

#[test]
fn update_replaces_only_the_packages_files() -> Result<()> {
    let workspace = Workspace::new()?;
    let root = workspace.root();

    let entries = vec![
        PayloadEntry::File {
            path: "usr/bin/app".to_string(),
            mode: 0o755,
            contents: vec![],
        },
        PayloadEntry::File {
            path: "usr/share/banana".to_string(),
            mode: 0o755,
            contents: vec![],
        },
    ];
    let updated_entries = vec![
        PayloadEntry::File {
            path: "usr/bin/app".to_string(),
            mode: 0o755,
            contents: vec![],
        },
        PayloadEntry::File {
            path: "usr/share/banana2".to_string(),
            mode: 0o755,
            contents: vec![],
        },
    ];
    let unrelated_entries = vec![PayloadEntry::File {
        path: "usr/bin/unrelated".to_string(),
        mode: 0o755,
        contents: vec![],
    }];

    workspace.publish_with_entries("app", "1.0.0", 1, &[], &entries)?;
    workspace.install("app")?;
    workspace.publish_with_entries("unrelated", "1.0.0", 1, &[], &unrelated_entries)?;
    workspace.install("unrelated")?;

    assert!(root.join("usr/bin/app").exists());
    assert!(root.join("usr/share/banana").exists());
    assert!(root.join("usr/bin/unrelated").exists());

    workspace.publish_with_entries("app", "2.0.0", 2, &[], &updated_entries)?;
    workspace.update("app")?;

    assert!(root.join("usr/bin/app").exists());
    assert!(root.join("usr/share/banana2").exists());
    assert!(root.join("usr/bin/unrelated").exists());
    assert!(!root.join("usr/bin/banana").exists());

    Ok(())
}

#[test]
fn update_removes_obsolete_files() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = Workspace::new()?;
    let root = workspace.root();
    let entries = vec![
        PayloadEntry::File {
            path: "usr/bin/app".to_string(),
            mode: 0o755,
            contents: vec![],
        },
        PayloadEntry::File {
            path: "usr/share/banana".to_string(),
            mode: 0o755,
            contents: vec![],
        },
    ];
    let updated_entries = vec![PayloadEntry::File {
        path: "usr/bin/app".to_string(),
        mode: 0o755,
        contents: vec![],
    }];

    workspace.publish_with_entries("app", "1.0.0", 1, &[], &entries)?;
    workspace.install("app")?;

    assert!(root.join("usr/bin/app").exists());
    assert!(root.join("usr/share/banana").exists());

    workspace.publish_with_entries("app", "2.0.0", 2, &[], &updated_entries)?;
    workspace.update("app")?;

    assert!(root.join("usr/bin/app").exists());
    assert!(!root.join("usr/bin/banana").exists());

    Ok(())
}

#[test]
fn failed_update_restores_the_previous_state() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = Workspace::new()?;
    let original_entries = [PayloadEntry::file("usr/bin/app", b"old version")];
    workspace.publish_with_entries("app", "1.0.0", 1, &[], &original_entries)?;
    workspace.install("app")?;

    let original = workspace.row("app")?;
    let original_paths = workspace.owned_paths("app")?;
    let updated_entries = [
        PayloadEntry::file("usr/bin/app", b"new version"),
        PayloadEntry::file("usr/bin/new", b"new file"),
    ];
    workspace.publish_with_entries("app", "2.0.0", 2, &[], &updated_entries)?;
    workspace.install_conn()?.execute_batch(
        "CREATE TRIGGER reject_update_file BEFORE INSERT ON owned_files
         WHEN NEW.path = 'usr/bin/new'
         BEGIN SELECT RAISE(ABORT, 'injected database failure'); END;",
    )?;

    let err = workspace
        .update("app")
        .expect_err("the injected ownership-record failure must fail the update");
    assert!(
        format!("{err:#}").contains("injected database failure"),
        "unexpected database error: {err:#}"
    );

    assert_eq!(
        std::fs::read(original.install_dir.join("usr/bin/app"))?,
        b"old version"
    );
    assert!(!original.install_dir.join("usr/bin/new").exists());
    let after = workspace.row("app")?;
    assert_eq!(after.version, original.version);
    assert_eq!(after.release, original.release);
    assert_eq!(after.sky_path, original.sky_path);
    assert_eq!(after.install_dir, original.install_dir);
    assert_eq!(workspace.owned_paths("app")?, original_paths);

    Ok(())
}

#[test]
fn failed_later_package_update_keeps_earlier_updates_committed() -> Result<()> {
    let workspace = Workspace::new()?;

    let alpha_old = [PayloadEntry::file("usr/bin/alpha", b"alpha old")];
    let zeta_old = [PayloadEntry::file("usr/bin/zeta", b"zeta old")];
    workspace.publish_with_entries("alpha", "1.0.0", 1, &[], &alpha_old)?;
    workspace.publish_with_entries("zeta", "1.0.0", 1, &[], &zeta_old)?;
    workspace.install("alpha")?;
    workspace.install("zeta")?;
    let alpha_install_dir = workspace.row("alpha")?.install_dir;
    let zeta_install_dir = workspace.row("zeta")?.install_dir;
    let zeta_owned_paths = workspace.owned_paths("zeta")?;

    let alpha_new = [PayloadEntry::file("usr/bin/alpha", b"alpha new")];
    let zeta_new = [PayloadEntry::file("usr/bin/zeta", b"zeta new")];
    workspace.publish_with_entries("alpha", "2.0.0", 2, &[], &alpha_new)?;
    workspace.publish_with_entries("zeta", "2.0.0", 2, &[], &zeta_new)?;
    workspace.install_conn()?.execute_batch(
        "CREATE TRIGGER reject_zeta_update BEFORE INSERT ON owned_files
         WHEN NEW.package_name = 'zeta'
         BEGIN SELECT RAISE(ABORT, 'injected zeta database failure'); END;",
    )?;

    let err = update(&workspace.config, &UpdateArgs { package: None })
        .expect_err("the later package update must fail");
    assert!(
        format!("{err:#}").contains("injected zeta database failure"),
        "unexpected update error: {err:#}"
    );

    assert_eq!(
        std::fs::read(alpha_install_dir.join("usr/bin/alpha"))?,
        b"alpha new"
    );
    assert_eq!(
        std::fs::read(zeta_install_dir.join("usr/bin/zeta"))?,
        b"zeta old"
    );

    let alpha = workspace.row("alpha")?;
    assert_eq!(alpha.version, "2.0.0");
    assert_eq!(alpha.release, 2);
    let zeta = workspace.row("zeta")?;
    assert_eq!(zeta.version, "1.0.0");
    assert_eq!(zeta.release, 1);
    assert_eq!(workspace.owned_paths("zeta")?, zeta_owned_paths);

    Ok(())
}
