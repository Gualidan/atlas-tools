use std::env::consts::ARCH;

use anyhow::Result;
use rusqlite::{Connection, params};
use tempfile::tempdir;

use crate::uninstall::resolve::SkippedPackage;
use crate::{
    install::db::ensure_install_schema,
    types::runtime_config::RuntimeConfig,
    uninstall::{resolve::resolve_group, uninstall::uninstall},
};

/// Inserts an installed row for `name` recorded in the given groups.
fn seed_installed(conn: &Connection, name: &str, groups: &[&str]) -> Result<()> {
    seed_installed_with_reason(conn, name, groups, "explicit")
}

/// Inserts an installed row for `name` recorded in the given groups with the
/// given `install_reason` label (`explicit` or `dependency`).
fn seed_installed_with_reason(
    conn: &Connection,
    name: &str,
    groups: &[&str],
    install_reason: &str,
) -> Result<()> {
    let quoted: Vec<String> = groups.iter().map(|group| format!("\"{group}\"")).collect();
    conn.execute(
        "INSERT INTO installed_packages
         (name, version, release, architecture, sky_path, install_dir, install_reason, groups)
         VALUES (?, '1.0.0', 1, ?, ?, ?, ?, ?)",
        params![
            name,
            ARCH,
            format!("{name}.sky"),
            format!("/installed/{name}"),
            install_reason,
            format!("[{}]", quoted.join(", ")),
        ],
    )?;
    Ok(())
}

/// Records a direct dependency edge from `package` to `dependency`.
fn seed_dependency(conn: &Connection, package: &str, dependency: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO package_dependencies (package_name, architecture, dependency_name)
         VALUES (?, ?, ?)",
        params![package, ARCH, dependency],
    )?;
    Ok(())
}

// Approved case 2: uninstall bails when both a package and a group are given.
#[test]
fn uninstall_rejects_a_package_and_a_group_together() -> Result<()> {
    let dir = tempdir()?;
    let config = RuntimeConfig::build_at(dir.path())?;

    let err = uninstall(&config, Some("foo"), Some("tools"), false)
        .expect_err("package and group together must be rejected");

    assert_eq!(
        err.to_string(),
        "You should specify either a package or group to uninstall, not both"
    );
    Ok(())
}

// Approved case 3: uninstall bails when neither a package nor a group is given.
#[test]
fn uninstall_rejects_neither_a_package_nor_a_group() -> Result<()> {
    let dir = tempdir()?;
    let config = RuntimeConfig::build_at(dir.path())?;

    let err =
        uninstall(&config, None, None, false).expect_err("a package or a group must be required");

    assert_eq!(
        err.to_string(),
        "You should specify a package or group to uninstall"
    );
    Ok(())
}

// Approved case 4: every installed member of the group is planned for removal.
#[test]
fn group_resolution_plans_every_installed_member_of_the_group() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    ensure_install_schema(&conn)?;
    seed_installed(&conn, "alpha", &["tools"])?;
    seed_installed(&conn, "beta", &["tools"])?;
    seed_installed(&conn, "gamma", &["other"])?;

    let (plan, report) = resolve_group(&conn, "tools")?;

    let mut names: Vec<&str> = plan
        .iter()
        .map(|(package, _)| package.name.as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["alpha", "beta"], "gamma must not be in the plan");

    let alpha = plan
        .iter()
        .find(|(package, _)| package.name == "alpha")
        .expect("alpha must be planned");
    assert_eq!(alpha.1, "/installed/alpha");

    assert_eq!(report.skipped, vec![]);

    Ok(())
}

// Approved case 5: an unknown group is rejected with an error naming the group.
#[test]
fn group_resolution_names_the_unknown_group_in_its_error() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    ensure_install_schema(&conn)?;
    seed_installed(&conn, "alpha", &["other"])?;

    let err = resolve_group(&conn, "tools").expect_err("an unknown group must be rejected");

    assert!(
        err.to_string().contains("tools"),
        "the error must name the group: {err}"
    );
    Ok(())
}

// Approved case 6: a non-explicit dependency of a group member joins the plan.
#[test]
fn group_resolution_plans_a_members_dependency_install() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    ensure_install_schema(&conn)?;
    seed_installed(&conn, "alpha", &["tools"])?;
    seed_installed_with_reason(&conn, "lib", &[], "dependency")?;
    seed_dependency(&conn, "alpha", "lib")?;

    let (plan, report) = resolve_group(&conn, "tools")?;

    let mut names: Vec<&str> = plan
        .iter()
        .map(|(package, _)| package.name.as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["alpha", "lib"], "lib must be planned exactly once");

    let alpha = plan
        .iter()
        .find(|(package, _)| package.name == "alpha")
        .expect("alpha must be planned");
    assert_eq!(alpha.1, "/installed/alpha");

    let lib = plan
        .iter()
        .find(|(package, _)| package.name == "lib")
        .expect("lib must be planned");
    assert_eq!(lib.1, "/installed/lib");

    assert_eq!(report.skipped, vec![]);

    Ok(())
}

// Approved case 7: an explicit dependency of a group member stays installed.
#[test]
fn group_resolution_leaves_a_members_explicit_dependency_installed() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    ensure_install_schema(&conn)?;
    seed_installed(&conn, "alpha", &["tools"])?;
    seed_installed(&conn, "toolkit", &[])?;
    seed_dependency(&conn, "alpha", "toolkit")?;

    let (plan, report) = resolve_group(&conn, "tools")?;

    let names: Vec<&str> = plan
        .iter()
        .map(|(package, _)| package.name.as_str())
        .collect();
    assert_eq!(
        names,
        ["alpha"],
        "an explicit dependency must not be planned"
    );

    assert_eq!(report.skipped, vec![]);

    Ok(())
}

// Approved case 8: a shared dependency of two group members is planned exactly once.
#[test]
fn group_resolution_plans_a_shared_dependency_exactly_once() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    ensure_install_schema(&conn)?;
    seed_installed(&conn, "alpha", &["tools"])?;
    seed_installed(&conn, "beta", &["tools"])?;
    seed_installed_with_reason(&conn, "lib", &[], "dependency")?;
    seed_dependency(&conn, "alpha", "lib")?;
    seed_dependency(&conn, "beta", "lib")?;

    let (plan, report) = resolve_group(&conn, "tools")?;

    let mut names: Vec<&str> = plan
        .iter()
        .map(|(package, _)| package.name.as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["alpha", "beta", "lib"],
        "the shared dependency must be planned exactly once"
    );

    assert_eq!(report.skipped, vec![]);

    Ok(())
}

// Approved case 9: a group member reached as another member's dependency is
// still removed.
#[test]
fn group_resolution_keeps_a_member_reached_as_another_members_dependency() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    ensure_install_schema(&conn)?;
    seed_installed(&conn, "alpha", &["tools"])?;
    seed_installed(&conn, "beta", &["tools"])?;
    seed_dependency(&conn, "alpha", "beta")?;

    let (plan, report) = resolve_group(&conn, "tools")?;

    let mut names: Vec<&str> = plan
        .iter()
        .map(|(package, _)| package.name.as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["alpha", "beta"],
        "both members must be planned, each exactly once"
    );

    assert_eq!(report.skipped, vec![]);

    Ok(())
}

// Approved case 10: a planned package required by an installed package outside
// the plan is skipped, with the dependent named.
#[test]
fn group_resolution_skips_a_planned_package_required_from_outside_the_plan() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    ensure_install_schema(&conn)?;
    seed_installed(&conn, "alpha", &["tools"])?;
    seed_installed_with_reason(&conn, "lib", &[], "dependency")?;
    seed_installed(&conn, "keeper", &[])?;
    seed_dependency(&conn, "alpha", "lib")?;
    seed_dependency(&conn, "keeper", "lib")?;

    let (plan, report) = resolve_group(&conn, "tools")?;

    let mut names: Vec<&str> = plan
        .iter()
        .map(|(package, _)| package.name.as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["alpha"],
        "lib must not be planned: keeper, installed outside the plan, requires it"
    );

    assert_eq!(
        report.skipped,
        vec![SkippedPackage {
            name: "lib".to_string(),
            required_by: "keeper".to_string(),
        }],
        "the skip must name the external dependent"
    );

    Ok(())
}

// Approved case 11: skipping cascades to the skipped package's planned
// dependencies, each attributed to the skipped package that required it.
#[test]
fn group_resolution_cascades_a_skip_to_the_skipped_packages_planned_dependencies() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    ensure_install_schema(&conn)?;
    seed_installed(&conn, "alpha", &["tools"])?;
    seed_installed_with_reason(&conn, "lib", &[], "dependency")?;
    seed_installed_with_reason(&conn, "sub", &[], "dependency")?;
    seed_installed(&conn, "keeper", &[])?;
    seed_dependency(&conn, "alpha", "lib")?;
    seed_dependency(&conn, "lib", "sub")?;
    seed_dependency(&conn, "keeper", "lib")?;

    let (plan, report) = resolve_group(&conn, "tools")?;

    let mut names: Vec<&str> = plan
        .iter()
        .map(|(package, _)| package.name.as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["alpha"],
        "both lib and sub must be skipped: keeper requires lib, and sub is lib's dependency"
    );

    let mut skipped_names: Vec<&str> = report
        .skipped
        .iter()
        .map(|skip| skip.name.as_str())
        .collect();
    skipped_names.sort_unstable();
    assert_eq!(skipped_names, ["lib", "sub"], "both skips must be reported");

    let lib = report
        .skipped
        .iter()
        .find(|skip| skip.name == "lib")
        .expect("lib must be skipped");
    assert_eq!(lib.required_by, "keeper", "lib's skip must name keeper");

    let sub = report
        .skipped
        .iter()
        .find(|skip| skip.name == "sub")
        .expect("sub must be skipped");
    assert_eq!(
        sub.required_by, "lib",
        "the cascade must attribute sub to the skipped package that required it"
    );

    Ok(())
}

// Approved case 12: a package required by another group member is not skipped,
// even when several in-plan dependents require it.
#[test]
fn group_resolution_never_skips_a_package_required_by_another_group_member() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    ensure_install_schema(&conn)?;
    seed_installed(&conn, "alpha", &["tools"])?;
    seed_installed(&conn, "beta", &["tools"])?;
    seed_installed_with_reason(&conn, "shared", &[], "dependency")?;
    seed_dependency(&conn, "beta", "shared")?;
    seed_dependency(&conn, "alpha", "shared")?;

    let (plan, report) = resolve_group(&conn, "tools")?;

    let mut names: Vec<&str> = plan
        .iter()
        .map(|(package, _)| package.name.as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["alpha", "beta", "shared"],
        "each package must be planned exactly once"
    );

    assert_eq!(
        report.skipped,
        vec![],
        "a dependent inside the plan must never trigger a skip"
    );

    Ok(())
}

// Approved case end-to-end: a group uninstall with assume-yes removes every
// member's directory and install-database records while leaving non-members alone.
#[test]
fn group_uninstall_with_assume_yes_removes_member_directories_and_records() -> Result<()> {
    let dir = tempdir()?;
    let config = RuntimeConfig::build_at(dir.path())?;

    let conn = Connection::open(&config.install_db)?;
    ensure_install_schema(&conn)?;
    conn.execute("PRAGMA foreign_keys = ON", [])?;

    let root = dir.path().join("root");
    for (name, groups, marker) in [
        ("alpha", "[\"tools\"]", "alpha.marker"),
        ("beta", "[\"tools\"]", "beta.marker"),
        ("keeper", "[]", "keeper.marker"),
    ] {
        let install_dir = root.join(name);
        std::fs::create_dir_all(&install_dir)?;
        std::fs::write(install_dir.join(marker), b"original")?;
        conn.execute(
            "INSERT INTO installed_packages
             (name, version, release, architecture, sky_path, install_dir, install_reason, groups)
             VALUES (?, '1.0.0', 1, ?, 'old.sky', ?, 'explicit', ?)",
            params![
                name,
                ARCH.to_string(),
                install_dir.to_str().unwrap(),
                groups
            ],
        )?;
    }

    conn.execute(
        "INSERT INTO owned_files VALUES ('alpha.marker', 'alpha', ?)",
        [ARCH.to_string()],
    )?;
    conn.execute(
        "INSERT INTO owned_files VALUES ('alpha.extra', 'alpha', ?)",
        [ARCH.to_string()],
    )?;
    conn.execute(
        "INSERT INTO owned_files VALUES ('beta.marker', 'beta', ?)",
        [ARCH.to_string()],
    )?;
    conn.execute(
        "INSERT INTO owned_files VALUES ('keeper.marker', 'keeper', ?)",
        [ARCH.to_string()],
    )?;

    conn.execute(
        "INSERT INTO package_dependencies VALUES ('alpha', ?, 'beta')",
        [ARCH.to_string()],
    )?;

    let report = uninstall(&config, None, Some("tools"), false)?;

    assert!(
        !root.join("alpha").join("alpha.marker").exists(),
        "alpha's sentinel must be removed"
    );
    assert!(
        !root.join("beta").join("beta.marker").exists(),
        "beta's sentinel must be removed"
    );
    assert!(
        root.join("keeper").join("keeper.marker").exists(),
        "keeper's sentinel must survive"
    );

    let count = |sql: &str| -> Result<i64> { Ok(conn.query_row(sql, [], |row| row.get(0))?) };
    assert_eq!(
        count("SELECT COUNT(*) FROM installed_packages WHERE name IN ('alpha', 'beta')",)?,
        0,
        "members' install records must be gone"
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM installed_packages WHERE name = 'keeper'")?,
        1,
        "keeper's install record must survive"
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM owned_files WHERE package_name = 'alpha'")?,
        0,
        "member's owned-file records must be gone"
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM owned_files WHERE package_name = 'keeper'")?,
        1,
        "keeper's owned-file record must survive"
    );
    assert_eq!(
        count(
            "SELECT COUNT(*) FROM package_dependencies WHERE package_name = 'alpha'
             AND dependency_name = 'beta'",
        )?,
        0,
        "the member dependency edge must be gone"
    );

    assert_eq!(report.skipped, vec![]);

    Ok(())
}
