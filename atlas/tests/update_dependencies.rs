mod fixtures;

use anyhow::Result;
use fixtures::Workspace;
use rusqlite::{Connection, params};

#[test]
fn updating_a_package_upgrades_its_direct_dependency_in_place() -> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish("lib", "1.0.0", 1, &[])?;
    workspace.publish("app", "1.0.0", 1, &["lib"])?;
    workspace.install("app")?;

    workspace.set_metadata("app", "2000-01-01", "explicit")?;
    workspace.set_metadata("lib", "2000-01-02", "dependency")?;

    let app_dir_before = workspace.row("app")?.install_dir;
    let lib_dir_before = workspace.row("lib")?.install_dir;

    workspace.publish("lib", "2.0.0", 7, &[])?;
    workspace.publish("app", "2.0.0", 7, &["lib"])?;

    workspace.update("app")?;

    let app = workspace.assert_installed("app", "2.0.0", 7)?;
    let lib = workspace.assert_installed("lib", "2.0.0", 7)?;

    assert_eq!(app.install_dir, app_dir_before, "app install directory");
    assert_eq!(lib.install_dir, lib_dir_before, "lib install directory");
    assert_eq!(app.installed_at, "2000-01-01", "app install timestamp");
    assert_eq!(lib.installed_at, "2000-01-02", "lib install timestamp");
    assert_eq!(app.reason, "explicit", "app install reason");
    assert_eq!(lib.reason, "dependency", "lib install reason");
    assert_eq!(workspace.deps("app")?, vec!["lib".to_string()]);
    assert_eq!(
        workspace.deps("lib")?,
        Vec::<String>::new(),
        "lib's deps should be empty"
    );
    workspace.assert_no_staging_dirs()?;

    Ok(())
}

#[test]
fn dependencies_are_resolved_recursively() -> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish("base", "1.0.0", 1, &[])?;
    workspace.publish("mid", "1.0.0", 1, &["base"])?;
    workspace.publish("app", "1.0.0", 1, &["mid"])?;

    workspace.install("app")?;

    workspace.set_metadata("app", "2000-01-01", "explicit")?;
    workspace.set_metadata("mid", "2000-01-01", "dependency")?;
    workspace.set_metadata("base", "2000-01-01", "dependency")?;

    workspace.publish("base", "2.0.0", 8, &[])?;
    workspace.publish("mid", "2.0.0", 8, &["base"])?;
    workspace.publish("app", "2.0.0", 8, &["mid"])?;

    workspace.update("app")?;

    workspace.assert_installed("app", "2.0.0", 8)?;
    workspace.assert_installed("mid", "2.0.0", 8)?;
    workspace.assert_installed("base", "2.0.0", 8)?;

    assert!(workspace.deps("app")?.contains(&"mid".to_string()));
    assert!(workspace.deps("mid")?.contains(&"base".to_string()));
    assert_eq!(
        workspace.deps("base")?,
        Vec::<String>::new(),
        "base's deps should be empty"
    );

    Ok(())
}

#[test]
fn packages_are_updated_in_order() -> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish("base", "1.0.0", 1, &[])?;
    workspace.publish("left", "1.0.0", 1, &["base"])?;
    workspace.publish("right", "1.0.0", 1, &["base"])?;
    workspace.publish("app", "1.0.0", 1, &["left", "right"])?;

    workspace.install("app")?;

    workspace.set_metadata("app", "2000-01-01", "explicit")?;
    workspace.set_metadata("left", "2000-01-01", "dependency")?;
    workspace.set_metadata("right", "2000-01-01", "dependency")?;
    workspace.set_metadata("base", "2000-01-01", "dependency")?;

    workspace.publish("base", "2.0.0", 8, &[])?;
    workspace.publish("left", "2.0.0", 8, &["base"])?;
    workspace.publish("right", "2.0.0", 8, &["base"])?;
    workspace.publish("app", "2.0.0", 8, &["left", "right"])?;

    workspace.update("app")?;

    let log = workspace.get_update_log()?;

    assert_eq!(log.len(), 4);

    let names: Vec<_> = log.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names.iter().filter(|&&name| name == "base").count(), 1);
    assert!(names.contains(&"base"));
    assert!(names.contains(&"left"));
    assert!(names.contains(&"right"));
    assert!(names.contains(&"app"));

    let base_index = names.iter().position(|&name| name == "base").unwrap();
    let left_index = names.iter().position(|&name| name == "left").unwrap();
    let right_index = names.iter().position(|&name| name == "right").unwrap();
    let app_index = names.iter().position(|&name| name == "app").unwrap();

    assert!(base_index < left_index);
    assert!(base_index < right_index);
    assert!(app_index > base_index);
    assert!(app_index > left_index);
    assert!(app_index > right_index);

    workspace.assert_installed("app", "2.0.0", 8)?;
    workspace.assert_installed("left", "2.0.0", 8)?;
    workspace.assert_installed("right", "2.0.0", 8)?;
    workspace.assert_installed("base", "2.0.0", 8)?;

    assert!(workspace.deps("left")?.contains(&"base".to_string()));
    assert!(workspace.deps("right")?.contains(&"base".to_string()));
    assert_eq!(
        workspace.deps("base")?,
        Vec::<String>::new(),
        "base's deps should be empty"
    );

    Ok(())
}

#[test]
fn already_up_to_date_is_a_no_op() -> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish("app", "1.0.0", 1, &[])?;
    workspace.install("app")?;

    let app_99 = workspace.publish("app", "1.0.0", 99, &[])?;
    std::fs::remove_file(app_99)?;

    workspace.update("app")?;

    workspace.assert_installed("app", "1.0.0", 1)?;
    workspace.assert_no_staging_dirs()?;
    Ok(())
}

#[test]
fn installed_package_declares_a_new_dependency_and_updating_fails_and_tells_to_reinstall()
-> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish("app", "1.0.0", 1, &[])?;

    workspace.install("app")?;

    workspace.set_metadata("app", "2000-01-01", "explicit")?;

    workspace.publish("base", "1.0.0", 1, &[])?;
    workspace.publish("app", "2.0.0", 2, &["base"])?;

    let err = workspace.update("app").expect_err(
        "updating should error because a new not yet installed dep was declared in the new version",
    );

    assert_eq!(
        err.to_string(),
        "New `app` version declares `base` as a dependency but it is not yet installed, try reinstalling `app`"
    );

    workspace.assert_installed("app", "1.0.0", 1)?;
    workspace.assert_no_staging_dirs()?;
    workspace.assert_not_installed("base");

    Ok(())
}

#[test]
fn missing_transitive_dep_errors() -> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish("mid", "1.0.0", 1, &[])?;
    workspace.publish("app", "1.0.0", 1, &["mid"])?;

    workspace.install("app")?;

    workspace.set_metadata("app", "2000-01-01", "explicit")?;
    workspace.set_metadata("mid", "2000-01-01", "dependency")?;

    workspace.publish("base", "1.0.0", 1, &[])?;
    workspace.publish("mid", "2.0.0", 2, &["base"])?;
    workspace.publish("app", "2.0.0", 2, &["mid"])?;

    let err = workspace.update("app").expect_err("updating should error because a new not yet installed dependency was added to one of the new packages");

    assert_eq!(
        err.to_string(),
        "New `mid` version declares `base` as a dependency but it is not yet installed, try reinstalling `mid`"
    );

    workspace.assert_installed("app", "1.0.0", 1)?;
    workspace.assert_no_staging_dirs()?;
    workspace.assert_not_installed("base");

    Ok(())
}

#[test]
fn dep_absent_from_the_repository_fails() -> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish("base", "1.0.0", 1, &[])?;
    workspace.publish("app", "1.0.0", 1, &["base"])?;

    workspace.install("app")?;

    workspace.set_metadata("app", "2000-01-01", "explicit")?;
    workspace.set_metadata("base", "2000-01-01", "dependency")?;

    let conn = workspace.repo_conn()?;
    workspace.publish("base", "2.0.0", 2, &[])?;

    conn.execute("DELETE FROM packages WHERE name = ?", params!["base"])?;

    workspace.publish("app", "2.0.0", 2, &["base"])?;

    let err = workspace
        .update("app")
        .expect_err("update should error because the repository rows for base no longer exist");

    assert_eq!(
        err.to_string(),
        "Dependency of `app` `base` not found in repository"
    );

    workspace.assert_installed("app", "1.0.0", 1)?;
    workspace.assert_installed("base", "1.0.0", 1)?;
    workspace.assert_no_staging_dirs()?;

    Ok(())
}

#[test]
fn old_dependency_edges_are_replaced() -> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish("old_base", "1.0.0", 1, &[])?;
    workspace.publish("new_base", "2.0.0", 2, &[])?;
    workspace.publish("app", "1.0.0", 1, &["old_base"])?;

    workspace.install("app")?;
    workspace.install("new_base")?;

    let missing_sky = workspace.publish("old_base", "2.0.0", 2, &[])?;
    workspace.publish("app", "2.0.0", 2, &["new_base"])?;

    std::fs::remove_file(missing_sky)?;

    workspace.update("app")?;

    workspace.assert_installed("app", "2.0.0", 2)?;
    workspace.assert_installed("old_base", "1.0.0", 1)?;
    workspace.assert_installed("new_base", "2.0.0", 2)?;
    assert_eq!(workspace.deps("app")?, vec!["new_base".to_string()]);
    assert!(!workspace.deps("app")?.contains(&"old_base".to_string()));
    workspace.assert_no_staging_dirs()?;

    Ok(())
}

#[test]
fn deps_are_never_downgraded_and_update_becomes_a_no_op() -> Result<()> {
    let workspace = Workspace::new()?;

    let lib_sky = workspace.publish("lib", "3.0.0", 1, &[])?;
    workspace.publish("app", "1.0.0", 1, &["lib"])?;

    workspace.install("app")?;

    let repo = Connection::open(&workspace.config.repo_db)?;
    repo.execute(
        "DELETE FROM packages WHERE name = ? AND version = ?",
        params!["lib", "3.0.0"],
    )?;
    std::fs::remove_file(lib_sky)?;

    workspace.publish("lib", "2.0.0", 1, &[])?;
    workspace.publish("app", "2.0.0", 2, &["lib"])?;

    workspace.update("app").expect_err(
        "update should fail because a package downgrade possibility results in an error",
    );

    workspace.assert_installed("app", "1.0.0", 1)?;
    workspace.assert_installed("lib", "3.0.0", 1)?;
    assert_eq!(workspace.deps("app")?, vec!["lib".to_string()]);
    assert_eq!(workspace.deps("lib")?, Vec::<String>::new());

    workspace.assert_no_staging_dirs()?;

    Ok(())
}

#[test]
fn metadata_and_incoming_edges_survive() -> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish("lib", "1.0.0", 1, &[])?;
    workspace.publish("app", "1.0.0", 1, &["lib"])?;
    workspace.publish("consumer", "1.0.0", 1, &["lib"])?;

    workspace.install("app")?;
    workspace.install("consumer")?;

    workspace.set_metadata("lib", "2001-01-01", "explicit")?;
    workspace.set_metadata("app", "2000-01-01", "explicit")?;
    workspace.set_metadata("consumer", "2000-01-02", "dependency")?;

    let lib_before = workspace.row("lib")?;
    let app_before = workspace.row("app")?;
    let consumer_before = workspace.row("consumer")?;

    workspace.publish("app", "2.0.0", 2, &["lib"])?;

    workspace.update("app")?;

    let app = workspace.assert_installed("app", "2.0.0", 2)?;
    let lib = workspace.assert_installed("lib", "1.0.0", 1)?;
    let consumer = workspace.assert_installed("consumer", "1.0.0", 1)?;

    assert_eq!(lib_before.install_dir, lib.install_dir, "lib install dir");
    assert_eq!(
        lib_before.installed_at, lib.installed_at,
        "lib installed_at"
    );
    assert_eq!(lib_before.reason, lib.reason, "lib install_reason");

    assert_eq!(app_before.install_dir, app.install_dir, "app install dir");
    assert_eq!(
        consumer_before.install_dir, consumer.install_dir,
        "consumer install dir"
    );

    assert_eq!(workspace.deps("app")?, vec!["lib".to_string()]);
    assert_eq!(workspace.deps("consumer")?, vec!["lib".to_string()]);
    assert_eq!(workspace.deps("lib")?, Vec::<String>::new());

    workspace.assert_no_staging_dirs()?;

    Ok(())
}
