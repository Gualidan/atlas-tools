mod fixtures;

use anyhow::Result;
use fixtures::Workspace;

#[test]
fn uninstall_package_keeps_dependency_that_is_still_needed_by_another_package() -> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish("mid", "1.0.0", 1, &[])?;
    workspace.publish("app", "1.0.0", 1, &["mid"])?;
    workspace.publish("base", "1.0.0", 1, &["mid"])?;

    workspace.install("app")?;
    workspace.install("base")?;

    workspace.uninstall("app")?;

    workspace.assert_not_installed("app");
    workspace.assert_installed("base", "1.0.0", 1)?;
    workspace.assert_installed("mid", "1.0.0", 1)?;
    workspace.assert_deps("base", &["mid"])?;
    workspace.assert_deps("mid", &[])?;

    Ok(())
}
