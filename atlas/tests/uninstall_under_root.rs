mod fixtures;
use std::{fs::File, io::Write};

use anyhow::Result;
use fixtures::*;

#[test]
fn uninstall_only_removes_relevant_files() -> Result<()> {
    let workspace = Workspace::new()?;
    let root = workspace.root();

    let app_entries = vec![
        PayloadEntry::File {
            path: "usr/bin/tool".to_string(),
            mode: 0o755,
            contents: vec![],
        },
        PayloadEntry::File {
            path: "usr/share/man/man1/tool.1".to_string(),
            mode: 0o644,
            contents: vec![],
        },
    ];
    let mid_entries = vec![
        PayloadEntry::File {
            path: "usr/bin/mid".to_string(),
            mode: 0o755,
            contents: vec![],
        },
        PayloadEntry::File {
            path: "usr/share/man/man1/mid.1".to_string(),
            mode: 0o644,
            contents: vec![],
        },
    ];
    let unrelated_file = root.join("usr/bin/unrelated");
    File::create(&unrelated_file)?;

    workspace.publish_with_entries("app", "1.0.0", 1, &[], &app_entries)?;
    workspace.publish_with_entries("mid", "1.0.0", 1, &[], &mid_entries)?;

    workspace.install("app")?;
    workspace.install("mid")?;

    workspace.uninstall("app")?;

    for entry in &mid_entries {
        match &entry {
            &PayloadEntry::File { path, .. }
            | &PayloadEntry::Symlink { path, .. }
            | &PayloadEntry::Directory { path, .. } => {
                assert!(root.join(path).exists());
            }
        }
    }

    for entry in &app_entries {
        match &entry {
            &PayloadEntry::File { path, .. }
            | &PayloadEntry::Symlink { path, .. }
            | &PayloadEntry::Directory { path, .. } => {
                assert!(!root.join(path).exists());
            }
        }
    }
    assert!(unrelated_file.exists());

    Ok(())
}

#[test]
fn shared_directories_are_safe() -> Result<()> {
    let workspace = Workspace::new()?;
    let root = workspace.root();
    let app_entries = vec![
        PayloadEntry::File {
            path: "usr/bin/app".to_string(),
            mode: 0o755,
            contents: vec![],
        },
        PayloadEntry::File {
            path: "usr/share/smt".to_string(),
            mode: 0o644,
            contents: vec![],
        },
    ];
    let mid_entries = vec![PayloadEntry::File {
        path: "usr/bin/mid".to_string(),
        mode: 0o644,
        contents: vec![],
    }];

    workspace.publish_with_entries("app", "1.0.0", 1, &[], &app_entries)?;
    workspace.publish_with_entries("mid", "1.0.0", 1, &[], &mid_entries)?;

    workspace.install("app")?;
    workspace.install("mid")?;
    workspace.uninstall("app")?;

    assert!(!root.join("usr/share").exists());
    assert!(root.join("usr/bin").exists());

    for entry in &mid_entries {
        match &entry {
            &PayloadEntry::File { path, .. }
            | &PayloadEntry::Symlink { path, .. }
            | &PayloadEntry::Directory { path, .. } => {
                assert!(root.join(path).exists());
            }
        }
    }
    for entry in &app_entries {
        match &entry {
            &PayloadEntry::File { path, .. }
            | &PayloadEntry::Symlink { path, .. }
            | &PayloadEntry::Directory { path, .. } => {
                assert!(!root.join(path).exists());
            }
        }
    }
    Ok(())
}

#[test]
fn symlink_targets_are_safe() -> Result<()> {
    let workspace = Workspace::new()?;
    let root = workspace.root();
    let app_entries = vec![
        PayloadEntry::File {
            path: "usr/bin/app".to_string(),
            mode: 0o755,
            contents: vec![],
        },
        PayloadEntry::Symlink {
            path: "usr/share/anotherapp".to_string(),
            target: "usr/share/app".to_string(),
        },
    ];
    let usr_share_app = root.join("usr/share/app");
    File::create(&usr_share_app)?;
    workspace.publish_with_entries("app", "1.0.0", 1, &[], &app_entries)?;
    workspace.uninstall("app")?;
    assert!(usr_share_app.exists());

    Ok(())
}

#[test]
fn files_modified_after_installation_are_removed() -> Result<()> {
    let workspace = Workspace::new()?;
    let root = workspace.root();
    let app_entries = vec![PayloadEntry::File {
        path: "usr/bin/app".to_string(),
        mode: 0o755,
        contents: vec![],
    }];
    workspace.publish_with_entries("app", "1.0.0", 1, &[], &app_entries)?;
    workspace.install("app")?;

    File::open(root.join("usr/bin/app"))?.write_all("hello_world".as_bytes())?;

    workspace.uninstall("app")?;
    assert!(!root.join("usr/bin/app").exists());

    Ok(())
}
