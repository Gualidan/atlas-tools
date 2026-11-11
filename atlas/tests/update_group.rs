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
