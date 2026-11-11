mod fixtures;

use anyhow::{Context, Result};
use atlas::install::install::install;
use fixtures::{PayloadEntry, Workspace, install_state_snapshot, snapshot_tree};

fn position(order: &[String], name: &str) -> usize {
    order
        .iter()
        .position(|entry| entry == name)
        .unwrap_or_else(|| panic!("{name} is missing from install order {order:?}"))
}

#[test]
fn installing_a_group_resolves_every_member_and_their_uninstalled_dependencies() -> Result<()> {
    let workspace = Workspace::new()?;

    // `shared-ui` is a dependency of group members but is not itself in the
    // group. `toolkit` is shared by both members and is in the group.
    workspace.publish("shared-ui", "1.0.0", 1, &[])?;
    workspace.publish_in_groups("toolkit", "1.0.0", 1, &["shared-ui"], &["desktop"])?;
    workspace.publish_in_groups("editor", "1.0.0", 1, &["toolkit"], &["desktop"])?;
    workspace.publish_in_groups(
        "terminal",
        "1.0.0",
        1,
        &["shared-ui", "toolkit"],
        &["desktop"],
    )?;

    // `Workspace` runs with --yes semantics, so the single combined plan is
    // accepted without a terminal.
    workspace.install_group("desktop")?;

    for name in ["shared-ui", "toolkit", "editor", "terminal"] {
        workspace.assert_installed(name, "1.0.0", 1)?;
    }

    let order = workspace.install_order()?;
    assert_eq!(order.len(), 4, "each package must appear once: {order:?}");
    assert!(
        position(&order, "shared-ui") < position(&order, "toolkit"),
        "the shared dependency must be installed before toolkit: {order:?}"
    );
    assert!(
        position(&order, "toolkit") < position(&order, "editor"),
        "toolkit must be installed before editor: {order:?}"
    );
    assert!(
        position(&order, "toolkit") < position(&order, "terminal"),
        "toolkit must be installed before terminal: {order:?}"
    );

    for member in ["toolkit", "editor", "terminal"] {
        assert_eq!(
            workspace.row(member)?.reason,
            "explicit",
            "{member} install reason"
        );
    }
    assert_eq!(
        workspace.row("shared-ui")?.reason,
        "dependency",
        "shared-ui install reason"
    );

    workspace.assert_deps("shared-ui", &[])?;
    workspace.assert_deps("toolkit", &["shared-ui"])?;
    workspace.assert_deps("editor", &["toolkit"])?;
    workspace.assert_deps("terminal", &["shared-ui", "toolkit"])?;

    workspace.assert_no_dangling_dependencies()?;
    workspace.assert_no_staging_dirs()?;

    Ok(())
}

#[test]
fn group_roots_share_a_diamond_dependency_once_and_resolve_in_dependency_first_order() -> Result<()>
{
    let workspace = Workspace::new()?;

    workspace.publish("base", "1.0.0", 1, &[])?;
    workspace.publish_in_groups("left", "1.0.0", 1, &["base"], &["graph"])?;
    workspace.publish_in_groups("right", "1.0.0", 1, &["base"], &["graph"])?;
    workspace.publish_in_groups("top", "1.0.0", 1, &["left", "right"], &["graph"])?;

    workspace.install_group("graph")?;

    for name in ["base", "left", "right", "top"] {
        workspace.assert_installed(name, "1.0.0", 1)?;
    }

    let order = workspace.install_order()?;
    assert_eq!(
        order.len(),
        4,
        "the diamond must install each node once: {order:?}"
    );
    for dependent in ["left", "right", "top"] {
        assert!(
            position(&order, "base") < position(&order, dependent),
            "base must precede {dependent}: {order:?}"
        );
    }
    for dependent in ["top"] {
        for dependency in ["left", "right"] {
            assert!(
                position(&order, dependency) < position(&order, dependent),
                "{dependency} must precede {dependent}: {order:?}"
            );
        }
    }

    assert_eq!(workspace.row("base")?.reason, "dependency", "base reason");
    for member in ["left", "right", "top"] {
        assert_eq!(
            workspace.row(member)?.reason,
            "explicit",
            "{member} install reason"
        );
    }

    workspace.assert_deps("left", &["base"])?;
    workspace.assert_deps("right", &["base"])?;
    workspace.assert_deps("top", &["left", "right"])?;
    workspace.assert_no_dangling_dependencies()?;
    workspace.assert_no_staging_dirs()?;

    Ok(())
}

fn run_member_dependency_case(
    dependency_sorts_first: bool,
    app_published_first: bool,
    already_installed: bool,
) -> Result<()> {
    let workspace = Workspace::new()?;
    let (base, app) = if dependency_sorts_first {
        ("aaa-base", "zzz-app")
    } else {
        ("base", "app")
    };

    if app_published_first {
        workspace.publish_in_groups(app, "1.0.0", 1, &[base], &["apps"])?;
        workspace.publish_in_groups(base, "1.0.0", 1, &[], &["apps"])?;
    } else {
        workspace.publish_in_groups(base, "1.0.0", 1, &[], &["apps"])?;
        workspace.publish_in_groups(app, "1.0.0", 1, &[base], &["apps"])?;
    }

    if already_installed {
        workspace.publish("holder", "1.0.0", 1, &[base])?;
        workspace.install("holder")?;
        assert_eq!(
            workspace.row(base)?.reason,
            "dependency",
            "{base} must start as an already-installed dependency"
        );
    }

    workspace.install_group("apps")?;

    workspace.assert_installed(app, "1.0.0", 1)?;
    workspace.assert_installed(base, "1.0.0", 1)?;
    assert_eq!(
        workspace.row(base)?.reason,
        "explicit",
        "{base} is a selected group member and must remain an explicit root"
    );
    assert_eq!(workspace.row(app)?.reason, "explicit", "{app} reason");
    workspace.assert_deps(app, &[base])?;

    if already_installed {
        workspace.assert_installed("holder", "1.0.0", 1)?;
        assert_eq!(workspace.row("holder")?.reason, "explicit", "holder reason");
        workspace.assert_deps("holder", &[base])?;
    }

    workspace.assert_no_dangling_dependencies()?;
    workspace.assert_no_staging_dirs()?;
    Ok(())
}

#[test]
fn a_group_member_used_as_another_member_dependency_remains_an_explicit_root() -> Result<()> {
    for dependency_sorts_first in [false, true] {
        for app_published_first in [false, true] {
            for already_installed in [false, true] {
                run_member_dependency_case(
                    dependency_sorts_first,
                    app_published_first,
                    already_installed,
                )
                .with_context(|| {
                    format!(
                        "dependency_sorts_first={dependency_sorts_first}, \
                         app_published_first={app_published_first}, \
                         already_installed={already_installed}"
                    )
                })?;
            }
        }
    }
    Ok(())
}

#[test]
fn group_lookup_uses_exact_membership() -> Result<()> {
    let workspace = Workspace::new()?;

    // Requested group is `tool`. `tools-extra` only belongs to `tools`, so a
    // substring or prefix lookup must not select it. `linux-tool` belongs to
    // two groups, which must not confuse exact membership.
    workspace.publish_in_groups("linux-tool", "1.0.0", 1, &[], &["tool", "desktop"])?;
    workspace.publish_in_groups("tools-extra", "1.0.0", 1, &[], &["tools"])?;
    workspace.publish_in_groups("desktop-tool", "1.0.0", 1, &[], &["desktop"])?;
    workspace.install_group("tool")?;

    workspace.assert_installed("linux-tool", "1.0.0", 1)?;
    assert_eq!(
        workspace.row("linux-tool")?.reason,
        "explicit",
        "linux-tool reason"
    );
    for not_member in ["tools-extra", "desktop-tool"] {
        workspace.assert_not_installed(not_member);
    }

    workspace.assert_no_dangling_dependencies()?;
    workspace.assert_no_staging_dirs()?;
    Ok(())
}

#[test]
fn an_empty_or_missing_group_fails_before_install_state_is_created() -> Result<()> {
    let workspace = Workspace::new()?;

    // The repository exists and has a package, but no package belongs to the
    // requested group.
    workspace.publish_in_groups("present", "1.0.0", 1, &[], &["other"])?;

    let target = workspace.target_path("absent-root");
    let before = install_state_snapshot(&workspace.config, &target)?;
    assert!(before.database.is_none(), "no install database yet");

    let err = workspace
        .install_group_into("missing-group", &target)
        .expect_err("an unknown group must be rejected");
    assert!(
        format!("{err:#}").contains("missing-group"),
        "the error must name the requested group: {err:#}"
    );

    assert!(
        !target.exists(),
        "a rejected group install must not create the target root"
    );
    assert_eq!(
        install_state_snapshot(&workspace.config, &target)?,
        before,
        "a rejected group install must not create install state"
    );
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum InvalidMember {
    MissingDependency,
    Cycle,
}

fn run_invalid_group_case(kind: InvalidMember) -> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish_in_groups("aaa-ok", "1.0.0", 1, &[], &["broken"])?;
    match kind {
        InvalidMember::MissingDependency => {
            workspace.publish_in_groups("zzz-broken", "1.0.0", 1, &["ghost"], &["broken"])?;
        }
        InvalidMember::Cycle => {
            workspace.publish_in_groups("zzz-left", "1.0.0", 1, &["zzz-right"], &["broken"])?;
            workspace.publish_in_groups("zzz-right", "1.0.0", 1, &["zzz-left"], &["broken"])?;
        }
    }

    let root_before = snapshot_tree(workspace.root())?;
    let err = workspace
        .install_group("broken")
        .expect_err("the complete group plan must be rejected");
    let chain = format!("{err:#}");
    match kind {
        InvalidMember::MissingDependency => {
            assert!(
                chain.contains("ghost"),
                "the error must name the missing dependency: {chain}"
            );
            assert!(
                chain.contains("not found"),
                "the error must reject the missing dependency: {chain}"
            );
        }
        InvalidMember::Cycle => {
            assert!(
                chain.contains("cycle"),
                "the error must name the cycle: {chain}"
            );
        }
    }

    assert_eq!(
        snapshot_tree(workspace.root())?,
        root_before,
        "rejecting the plan must leave the target root untouched"
    );
    workspace.assert_not_installed("aaa-ok");
    Ok(())
}

#[test]
fn a_missing_dependency_or_cycle_in_any_group_member_fails_before_any_package_is_installed()
-> Result<()> {
    for kind in [InvalidMember::MissingDependency, InvalidMember::Cycle] {
        run_invalid_group_case(kind).with_context(|| format!("case {kind:?}"))?;
    }
    Ok(())
}

#[test]
fn a_later_invalid_group_member_leaves_only_prior_committed_packages_installed() -> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish_in_groups("aaa-good", "1.0.0", 1, &[], &["mixed"])?;
    let broken_entries = [PayloadEntry::file_with_mode(
        "zzz-broken/evil.bin",
        0o4755,
        b"evil\n".to_vec(),
    )];
    workspace.publish_with_entries_in_groups(
        "zzz-broken",
        "1.0.0",
        1,
        &[],
        &broken_entries,
        &["mixed"],
    )?;

    let err = workspace
        .install_group("mixed")
        .expect_err("the setuid payload must fail verification");
    assert!(
        format!("{err:#}").contains("setuid"),
        "the later member must fail for the expected reason: {err:#}"
    );

    workspace.assert_installed("aaa-good", "1.0.0", 1)?;
    workspace.assert_not_installed("zzz-broken");

    workspace.assert_no_dangling_dependencies()?;
    workspace.assert_no_staging_dirs()?;
    Ok(())
}

fn run_graph_plan(publish_order: &[&str]) -> Result<Vec<String>> {
    let workspace = Workspace::new()?;

    for name in publish_order {
        match *name {
            "base" => {
                workspace.publish("base", "1.0.0", 1, &[])?;
            }
            "left" => {
                workspace.publish_in_groups("left", "1.0.0", 1, &["base"], &["graph"])?;
            }
            "right" => {
                workspace.publish_in_groups("right", "1.0.0", 1, &["base"], &["graph"])?;
            }
            "top" => {
                workspace.publish_in_groups("top", "1.0.0", 1, &["left", "right"], &["graph"])?;
            }
            other => panic!("unknown package in test order: {other}"),
        }
    }

    workspace.install_group("graph")?;
    workspace.assert_no_dangling_dependencies()?;
    workspace.install_order()
}

#[test]
fn reversing_repository_insertion_order_does_not_change_the_group_plan() -> Result<()> {
    let forward = run_graph_plan(&["base", "left", "right", "top"]).context("forward order")?;
    let reversed = run_graph_plan(&["top", "right", "left", "base"]).context("reversed order")?;

    assert_eq!(
        forward, reversed,
        "repository insertion order must not change the selected package set or plan order"
    );

    assert_eq!(
        forward.len(),
        4,
        "each graph node must be planned once: {forward:?}"
    );
    for dependent in ["left", "right", "top"] {
        assert!(
            position(&forward, "base") < position(&forward, dependent),
            "base must precede {dependent}: {forward:?}"
        );
    }
    for dependent in ["top"] {
        for dependency in ["left", "right"] {
            assert!(
                position(&forward, dependency) < position(&forward, dependent),
                "{dependency} must precede {dependent}: {forward:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn install_accepts_either_a_package_or_a_group_but_not_both() -> Result<()> {
    let workspace = Workspace::new()?;

    workspace.publish("solo", "1.0.0", 1, &[])?;
    workspace.publish_in_groups("grouped", "1.0.0", 1, &[], &["tools"])?;

    // Existing package syntax must keep working.
    workspace.install("solo")?;
    workspace.assert_installed("solo", "1.0.0", 1)?;

    let root = workspace.root().to_path_buf();
    let before = workspace.install_state_snapshot()?;

    let both = install(Some("solo"), Some("tools"), &workspace.config, &root)
        .expect_err("selecting a package and a group must fail");
    assert!(
        both.to_string().contains("not both"),
        "the error must reject both selection modes: {both}"
    );

    let neither = install(None, None, &workspace.config, &root)
        .expect_err("selecting neither a package nor a group must fail");
    assert!(
        neither.to_string().contains("Specify a package or group"),
        "the error must require a selection mode: {neither}"
    );

    assert_eq!(
        workspace.install_state_snapshot()?,
        before,
        "selection errors must not mutate install state"
    );

    // Selecting only a group must work.
    workspace.install_group("tools")?;
    workspace.assert_installed("grouped", "1.0.0", 1)?;
    workspace.assert_installed("solo", "1.0.0", 1)?;

    workspace.assert_no_dangling_dependencies()?;
    workspace.assert_no_staging_dirs()?;
    Ok(())
}
