mod fixtures;

use anyhow::Result;

use crate::fixtures::Workspace;

#[test]
fn updating_a_group_resolves_every_installed_member_and_their_newer_dependencies() -> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish_in_groups("another", "1.0.0", 1, &[], &[])?;
    workspace.publish_in_groups("foo", "1.0.0", 1, &[], &["default"])?;
    workspace.publish_in_groups("bar", "1.0.0", 1, &["another"], &["default"])?;
    workspace.publish_in_groups("baz", "1.0.0", 1, &["another"], &["default"])?;

    workspace.install_group("default")?;

    workspace.assert_installed("foo", "1.0.0", 1)?;
    workspace.assert_installed("bar", "1.0.0", 1)?;
    workspace.assert_installed("baz", "1.0.0", 1)?;
    workspace.assert_installed("another", "1.0.0", 1)?;

    workspace.assert_deps("foo", &[])?;
    workspace.assert_deps("bar", &["another"])?;
    workspace.assert_deps("baz", &["another"])?;

    workspace.assert_no_dangling_dependencies()?;
    workspace.assert_no_staging_dirs()?;

    workspace.publish_in_groups("another", "2.0.0", 2, &[], &[])?;
    workspace.publish_in_groups("foo", "2.0.0", 2, &[], &["default"])?;
    workspace.publish_in_groups("bar", "2.0.0", 2, &["another"], &["default"])?;
    workspace.publish_in_groups("baz", "2.0.0", 2, &["another"], &["default"])?;

    workspace.update_groups("default")?;

    workspace.assert_installed("foo", "2.0.0", 2)?;
    workspace.assert_installed("bar", "2.0.0", 2)?;
    workspace.assert_installed("baz", "2.0.0", 2)?;
    workspace.assert_installed("another", "2.0.0", 2)?;

    workspace.assert_deps("foo", &[])?;
    workspace.assert_deps("bar", &["another"])?;
    workspace.assert_deps("baz", &["another"])?;
    workspace.assert_deps("another", &[])?;

    workspace.assert_no_dangling_dependencies()?;
    workspace.assert_no_staging_dirs()?;

    Ok(())
}

#[test]
fn a_group_member_at_the_latest_version_is_skipped_while_newer_members_are_updated() -> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish_in_groups("another", "1.0.0", 1, &[], &[])?;
    workspace.publish_in_groups("foo", "1.0.0", 1, &[], &["default"])?;
    workspace.publish_in_groups("bar", "1.0.0", 1, &["another"], &["default"])?;
    workspace.publish_in_groups("baz", "1.0.0", 1, &["another"], &["default"])?;

    workspace.install_group("default")?;

    workspace.assert_installed("foo", "1.0.0", 1)?;
    workspace.assert_installed("bar", "1.0.0", 1)?;
    workspace.assert_installed("baz", "1.0.0", 1)?;
    workspace.assert_installed("another", "1.0.0", 1)?;

    workspace.assert_deps("foo", &[])?;
    workspace.assert_deps("bar", &["another"])?;
    workspace.assert_deps("baz", &["another"])?;

    workspace.assert_no_dangling_dependencies()?;
    workspace.assert_no_staging_dirs()?;

    workspace.publish_in_groups("another", "2.0.0", 2, &[], &[])?;
    workspace.publish_in_groups("foo", "1.0.0", 2, &[], &["default"])?;
    workspace.publish_in_groups("bar", "2.0.0", 2, &["another"], &["default"])?;
    workspace.publish_in_groups("baz", "2.0.0", 2, &["another"], &["default"])?;

    workspace.update_groups("default")?;

    workspace.assert_installed("foo", "1.0.0", 1)?;
    workspace.assert_installed("bar", "2.0.0", 2)?;
    workspace.assert_installed("baz", "2.0.0", 2)?;
    workspace.assert_installed("another", "2.0.0", 2)?;

    workspace.assert_deps("foo", &[])?;
    workspace.assert_deps("bar", &["another"])?;
    workspace.assert_deps("baz", &["another"])?;
    workspace.assert_deps("another", &[])?;

    workspace.assert_no_dangling_dependencies()?;
    workspace.assert_no_staging_dirs()?;

    Ok(())
}

#[test]
fn a_group_member_missing_from_the_install_database_rejects_the_plan() -> Result<()> {
    let ws = Workspace::new()?;

    ws.publish_in_groups("foo", "1.0.0", 1, &[], &["default"])?;
    ws.publish_in_groups("bar", "1.0.0", 1, &[], &["default"])?;

    ws.install_group("default")?;

    ws.assert_installed("foo", "1.0.0", 1)?;
    ws.assert_installed("bar", "1.0.0", 1)?;

    ws.assert_no_dangling_dependencies()?;
    ws.assert_no_staging_dirs()?;

    ws.uninstall("foo")?;

    ws.assert_installed("bar", "1.0.0", 1)?;
    assert!(ws.assert_installed("foo", "1.0.0", 1).is_err());

    ws.assert_no_dangling_dependencies()?;
    ws.assert_no_staging_dirs()?;

    ws.publish_in_groups("foo", "2.0.0", 2, &[], &["default"])?;
    ws.publish_in_groups("bar", "2.0.0", 2, &[], &["default"])?;

    let err = ws
        .update_groups("default")
        .expect_err("a group member missing from the install database must reject the plan");

    let chain = format!("{err:#}");
    assert!(
        chain.contains("Package `foo` from group `default` is not installed"),
        "unexpected error: {chain}"
    );

    Ok(())
}
