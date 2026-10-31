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

    assert!(workspace.row("app").is_err());
    assert!(workspace.row("base").is_ok());
    assert!(workspace.row("mid").is_ok());

    Ok(())
}

#[test]
fn uninstall_package_keeps_transitive_dependency_that_is_still_needed_elsewhere() -> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish("base", "1.0.0", 1, &[])?;
    workspace.publish("unrelated", "1.0.0", 1, &["base"])?;
    workspace.publish("mid", "1.0.0", 1, &["base"])?;
    workspace.publish("app", "1.0.0", 1, &["mid"])?;

    workspace.install("app")?;
    workspace.install("unrelated")?;

    workspace.uninstall("app")?;

    workspace.assert_not_installed("app");
    workspace.assert_not_installed("mid");
    workspace.assert_installed("unrelated", "1.0.0", 1)?;
    workspace.assert_installed("base", "1.0.0", 1)?;
    workspace.assert_deps("unrelated", &["base"])?;
    workspace.assert_deps("base", &[])?;

    Ok(())
}

#[test]
fn uninstall_package_with_shared_dependency_does_not_fail_even_if_dependency_has_external_dependent()
-> Result<()> {
    let workspace = Workspace::new()?;
    workspace.publish("base", "1.0.0", 1, &[])?;
    workspace.publish("app", "1.0.0", 1, &["base"])?;
    workspace.publish("unrelated", "1.0.0", 1, &["base"])?;

    workspace.install("app")?;
    workspace.install("unrelated")?;

    workspace.uninstall("app")?;

    workspace.assert_not_installed("app");
    workspace.assert_installed("unrelated", "1.0.0", 1)?;
    workspace.assert_installed("base", "1.0.0", 1)?;
    workspace.assert_deps("unrelated", &["base"])?;
    workspace.assert_deps("base", &[])?;

    Ok(())
}

#[test]
fn uninstall_package_chain_skips_only_relevant_shared_dependency() -> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish("base", "1.0.0", 1, &[])?;
    workspace.publish("mid", "1.0.0", 1, &["base"])?;
    workspace.publish("app", "1.0.0", 1, &["mid"])?;
    workspace.publish("unrelated", "1.0.0", 1, &["base"])?;

    workspace.install("app")?;
    workspace.install("unrelated")?;

    workspace.uninstall("app")?;

    workspace.assert_not_installed("app");
    workspace.assert_not_installed("mid");
    workspace.assert_installed("unrelated", "1.0.0", 1)?;
    workspace.assert_installed("base", "1.0.0", 1)?;
    workspace.assert_deps("unrelated", &["base"])?;
    workspace.assert_deps("base", &[])?;

    Ok(())
}

#[test]
fn uninstall_package_keeps_user_explicitly_installed_dependency() -> Result<()> {
    let workspace = Workspace::new()?;
    workspace.publish("base", "1.0.0", 1, &[])?;
    workspace.publish("app", "1.0.0", 1, &["base"])?;

    workspace.install("base")?;
    workspace.install("app")?;

    workspace.uninstall("app")?;

    workspace.assert_not_installed("app");
    workspace.assert_installed("base", "1.0.0", 1)?;
    workspace.assert_deps("base", &[])?;

    Ok(())
}
