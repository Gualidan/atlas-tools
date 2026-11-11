mod fixtures;

use crate::fixtures::{
    ConfigEnv, Workspace, count, generate_keypair, runtime_config,
    seed_repo_db_with_deps_and_groups, tempdir, write_signed_sky_with_file,
    write_signed_sky_with_raw_entry,
};
use anyhow::Result;
use atlas::{
    cli::{cli::UpdateArgs, confirmation::set_assume_yes},
    install::install::install,
    types::runtime_config::RuntimeConfig,
    update::update::update,
};
use ring::signature::Ed25519KeyPair;

use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
use std::{
    env,
    fs::{self, File},
    io::{Read, Write},
    os::fd::FromRawFd,
    process::Command,
    thread,
    time::{Duration, Instant},
};
use tar::EntryType;
use tempfile::TempDir;

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

#[test]
fn a_group_member_that_would_be_downgraded_rejects_the_whole_plan() -> Result<()> {
    let ws = Workspace::new()?;

    ws.publish_in_groups("foo", "2.0.0", 2, &[], &["default"])?;
    ws.publish_in_groups("bar", "1.0.0", 1, &[], &["default"])?;

    ws.install_group("default")?;

    ws.assert_installed("foo", "2.0.0", 2)?;
    ws.assert_installed("bar", "1.0.0", 1)?;

    ws.assert_no_dangling_dependencies()?;
    ws.assert_no_staging_dirs()?;

    ws.publish_in_groups("foo", "1.0.0", 1, &[], &["default"])?;
    ws.publish_in_groups("bar", "2.0.0", 2, &[], &["default"])?;

    let err = ws
        .update_groups("default")
        .expect_err("a group member that would be downgraded must reject the plan");

    let chain = format!("{err:#}");
    assert!(
        chain.contains("Not allowed to downgrade foo: installed version is 2.0.0 while the repository's is 1.0.0"),
        "unexpected error: {chain}"
    );
    Ok(())
}

#[test]
fn a_shared_dependency_that_is_also_a_group_member_appears_once_in_the_update_plan() -> Result<()> {
    let ws = Workspace::new()?;
    ws.publish_in_groups("base", "1.0.0", 1, &[], &["default"])?;
    ws.publish_in_groups("alpha", "1.0.0", 1, &["base"], &["default"])?;
    ws.publish_in_groups("beta", "1.0.0", 1, &["base"], &["default"])?;
    ws.install_group("default")?;

    ws.publish_in_groups("base", "2.0.0", 2, &[], &["default"])?;
    ws.publish_in_groups("alpha", "2.0.0", 2, &["base"], &["default"])?;
    ws.publish_in_groups("beta", "2.0.0", 2, &["base"], &["default"])?;
    ws.update_groups("default")?;

    for name in ["base", "alpha", "beta"] {
        ws.assert_installed(name, "2.0.0", 2)?;
    }
    let log = ws.get_update_log()?;
    assert_eq!(log.iter().filter(|row| row.name == "base").count(), 1);
    let base = log.iter().position(|row| row.name == "base").unwrap();
    for dependent in ["alpha", "beta"] {
        let dependent = log.iter().position(|row| row.name == dependent).unwrap();
        assert!(
            base < dependent,
            "base must update before its dependents: {log:?}"
        );
    }
    ws.assert_no_dangling_dependencies()?;
    ws.assert_no_staging_dirs()?;
    Ok(())
}

#[test]
fn a_newer_dependency_outside_the_group_is_updated_alongside_its_group_dependent() -> Result<()> {
    let ws = Workspace::new()?;
    ws.publish_in_groups("library", "1.0.0", 1, &[], &[])?;
    ws.publish_in_groups("app", "1.0.0", 1, &["library"], &["default"])?;
    ws.install_group("default")?;

    ws.publish_in_groups("library", "2.0.0", 2, &[], &[])?;
    ws.publish_in_groups("app", "2.0.0", 2, &["library"], &["default"])?;
    ws.update_groups("default")?;

    ws.assert_installed("library", "2.0.0", 2)?;
    ws.assert_installed("app", "2.0.0", 2)?;
    assert_eq!(
        ws.get_update_log()?
            .iter()
            .map(|row| row.name.as_str())
            .collect::<Vec<_>>(),
        ["library", "app"]
    );
    ws.assert_no_dangling_dependencies()?;
    ws.assert_no_staging_dirs()?;
    Ok(())
}

#[test]
fn a_member_whose_newer_version_leaves_the_group_is_updated_then_never_selected_again() -> Result<()>
{
    let ws = Workspace::new()?;
    ws.publish_in_groups("solo", "1.0.0", 1, &[], &["default"])?;
    ws.publish_in_groups("anchor", "1.0.0", 1, &[], &["default"])?;
    ws.install_group("default")?;
    ws.publish_in_groups("solo", "2.0.0", 2, &[], &[])?;

    ws.update_groups("default")?;
    ws.assert_installed("solo", "2.0.0", 2)?;
    ws.assert_installed("anchor", "1.0.0", 1)?;
    let groups: String = ws.install_conn()?.query_row(
        "SELECT groups FROM installed_packages WHERE name = 'solo'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(groups.trim(), "[]");
    assert_eq!(
        ws.get_update_log()?
            .iter()
            .map(|row| row.name.as_str())
            .collect::<Vec<_>>(),
        ["solo"]
    );

    let before_root = crate::fixtures::snapshot_tree(ws.root())?;
    let before_anchor = ws.row("anchor")?;
    ws.update_groups("default")?;
    assert_eq!(ws.install_state_snapshot()?.root, before_root);
    let after_anchor = ws.row("anchor")?;
    assert_eq!(after_anchor.version, before_anchor.version);
    assert_eq!(after_anchor.release, before_anchor.release);
    assert_eq!(ws.get_update_log()?.len(), 0);
    ws.assert_no_staging_dirs()?;
    Ok(())
}

#[test]
fn a_dependency_only_package_whose_newer_version_joins_the_group_is_selected_by_the_repository()
-> Result<()> {
    let ws = Workspace::new()?;
    ws.publish_in_groups("base", "1.0.0", 1, &[], &[])?;
    ws.publish_in_groups("app", "1.0.0", 1, &["base"], &["default"])?;
    ws.install_group("default")?;
    ws.set_metadata("base", "2001-02-03", "dependency")?;

    ws.publish_in_groups("base", "2.0.0", 2, &[], &["default"])?;
    ws.publish_in_groups("app", "1.0.0", 2, &["base"], &["default"])?;
    ws.update_groups("default")?;

    let base = ws.assert_installed("base", "2.0.0", 2)?;
    assert_eq!(base.reason, "dependency");
    assert_eq!(base.installed_at, "2001-02-03");
    assert_eq!(
        ws.get_update_log()?
            .iter()
            .map(|row| row.name.as_str())
            .collect::<Vec<_>>(),
        ["base"]
    );
    ws.assert_no_dangling_dependencies()?;
    ws.assert_no_staging_dirs()?;
    Ok(())
}

#[test]
fn an_unknown_group_fails_before_prompting_and_leaves_install_state_unchanged() -> Result<()> {
    let ws = Workspace::new()?;
    ws.publish_in_groups("solo", "1.0.0", 1, &[], &["other"])?;
    ws.install_group("other")?;
    let before = ws.row("solo")?;
    let files_before = fs::read(ws.root().join("solo/1.0.0.txt"))?;

    set_assume_yes(false);
    let err = ws
        .update_groups("missing")
        .expect_err("an absent repository group must be rejected");
    assert!(format!("{err:#}").contains("Unknown package group `missing`"));
    let after = ws.row("solo")?;
    assert_eq!(after.version, before.version);
    assert_eq!(after.release, before.release);
    assert_eq!(after.sky_path, before.sky_path);
    assert_eq!(fs::read(ws.root().join("solo/1.0.0.txt"))?, files_before);
    assert_eq!(
        ws.get_update_log()?.len(),
        0,
        "failed resolution must leave an explicitly empty update log"
    );
    ws.assert_no_staging_dirs()?;
    Ok(())
}

#[test]
fn a_missing_repository_dependency_of_a_group_member_rejects_the_plan_before_prompting()
-> Result<()> {
    let ws = Workspace::new()?;
    ws.publish_in_groups("app", "1.0.0", 1, &[], &["default"])?;
    ws.install_group("default")?;
    let before = ws.install_state_snapshot()?;
    ws.publish_in_groups("app", "2.0.0", 2, &["missing-library"], &["default"])?;

    set_assume_yes(false);
    let err = ws
        .update_groups("default")
        .expect_err("a dependency missing from the repository must reject the complete plan");
    assert!(
        format!("{err:#}")
            .contains("Dependency of `app` `missing-library` not found in repository")
    );
    assert_eq!(ws.row("app")?.version, "1.0.0");
    assert_eq!(fs::read(ws.root().join("app/1.0.0.txt"))?, b"app 1.0.0\\n");
    assert_eq!(ws.get_update_log()?.len(), 0);
    assert_eq!(ws.install_state_snapshot()?.root, before.root);
    ws.assert_no_dangling_dependencies()?;
    ws.assert_no_staging_dirs()?;
    Ok(())
}

fn update_order_with_publish_order(reverse: bool) -> Result<Vec<String>> {
    let ws = Workspace::new()?;
    let packages: [(&str, &[&str]); 3] = [("base", &[]), ("alpha", &["base"]), ("beta", &["base"])];
    let indices = if reverse { [2, 1, 0] } else { [0, 1, 2] };
    for index in indices {
        let (name, deps) = packages[index];
        ws.publish_in_groups(name, "1.0.0", 1, deps, &["default"])?;
    }
    ws.install_group("default")?;
    for index in indices {
        let (name, deps) = packages[index];
        ws.publish_in_groups(name, "2.0.0", 2, deps, &["default"])?;
    }
    ws.update_groups("default")?;
    ws.assert_no_dangling_dependencies()?;
    ws.assert_no_staging_dirs()?;
    Ok(ws
        .get_update_log()?
        .into_iter()
        .map(|row| row.name)
        .collect())
}

#[test]
fn reversing_repository_insertion_order_does_not_change_the_group_update_plan() -> Result<()> {
    let forward = update_order_with_publish_order(false)?;
    let reverse = update_order_with_publish_order(true)?;
    assert_eq!(
        forward, reverse,
        "candidate and apply order must not depend on insertion order"
    );
    assert_eq!(forward.len(), 3);
    assert_eq!(
        forward
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        3
    );
    Ok(())
}

#[test]
fn update_accepts_either_a_package_or_a_group_but_not_both() -> Result<()> {
    let ws = Workspace::new()?;
    ws.publish_in_groups("pkg", "1.0.0", 1, &[], &["default"])?;
    ws.install_group("default")?;

    let both = update(
        &ws.config,
        &UpdateArgs {
            package: Some("pkg".into()),
            group: Some("default".into()),
        },
    )
    .expect_err("package and group selectors are mutually exclusive");
    assert!(format!("{both:#}").contains("Specify either a package or group to update, not both"));

    ws.publish_in_groups("pkg", "2.0.0", 2, &[], &["default"])?;
    update(
        &ws.config,
        &UpdateArgs {
            package: None,
            group: Some("default".into()),
        },
    )?;
    ws.assert_installed("pkg", "2.0.0", 2)?;

    ws.publish_in_groups("pkg", "3.0.0", 3, &[], &["default"])?;
    update(
        &ws.config,
        &UpdateArgs {
            package: Some("pkg".into()),
            group: None,
        },
    )?;
    ws.assert_installed("pkg", "3.0.0", 3)?;

    ws.publish_in_groups("pkg", "4.0.0", 4, &[], &["default"])?;
    update(
        &ws.config,
        &UpdateArgs {
            package: None,
            group: None,
        },
    )?;
    ws.assert_installed("pkg", "4.0.0", 4)?;
    ws.assert_no_staging_dirs()?;
    Ok(())
}

struct RawEntryWorkspace {
    tmp: TempDir,
    config: RuntimeConfig,
    root: std::path::PathBuf,
    keypair: Ed25519KeyPair,
    _env: ConfigEnv,
}

impl RawEntryWorkspace {
    fn new() -> Result<Self> {
        let tmp = tempdir();
        let config = runtime_config(&tmp)?;
        let (keypair, public_key) = generate_keypair()?;
        let env = ConfigEnv::new(&tmp, &public_key)?;
        let root = tmp.path().join("root");
        fs::create_dir(&root)?;
        Ok(Self {
            tmp,
            config,
            root,
            keypair,
            _env: env,
        })
    }

    fn publish(
        &self,
        name: &str,
        version: &str,
        release: u32,
        deps: &[&str],
        groups: &[&str],
    ) -> Result<()> {
        let sky = write_signed_sky_with_file(
            &self.tmp,
            &self.keypair,
            &format!("{name}-{version}-{release}.sky"),
            name,
            version,
            release,
            format!("{name} {version}\\n").as_bytes(),
            deps.iter().map(ToString::to_string).collect(),
            name,
            &format!("{version}.txt"),
        )?;
        seed_repo_db_with_deps_and_groups(
            &self.config.repo_db,
            &[(name, version, release, sky.to_str().unwrap(), deps, groups)],
        );
        Ok(())
    }
}

#[test]
fn a_member_that_fails_mid_update_leaves_earlier_members_updated_and_itself_untouched() -> Result<()>
{
    let ws = RawEntryWorkspace::new()?;
    ws.publish("first", "1.0.0", 1, &[], &["default"])?;
    ws.publish("second", "1.0.0", 1, &["first"], &["default"])?;
    install(None, Some("default"), &ws.config, &ws.root)?;

    ws.publish("first", "2.0.0", 2, &[], &["default"])?;
    let bad_sky = write_signed_sky_with_raw_entry(
        &ws.tmp,
        &ws.keypair,
        "second-2.0.0-2.sky",
        "second",
        "2.0.0",
        2,
        "second",
        0o4755,
        EntryType::file(),
        b"setuid payload",
    )?;
    seed_repo_db_with_deps_and_groups(
        &ws.config.repo_db,
        &[(
            "second",
            "2.0.0",
            2,
            bad_sky.to_str().unwrap(),
            &["first"],
            &["default"],
        )],
    );

    let _err = update(
        &ws.config,
        &UpdateArgs {
            package: None,
            group: Some("default".into()),
        },
    )
    .expect_err("verification must reject the setuid raw entry after first is updated");

    let conn = rusqlite::Connection::open(&ws.config.install_db)?;
    let first_version: String = conn.query_row(
        "SELECT version FROM installed_packages WHERE name = 'first'",
        [],
        |row| row.get(0),
    )?;
    let second_version: String = conn.query_row(
        "SELECT version FROM installed_packages WHERE name = 'second'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(first_version, "2.0.0");
    assert_eq!(second_version, "1.0.0");
    assert!(ws.root.join("first/2.0.0.txt").exists());
    assert!(ws.root.join("second/1.0.0.txt").exists());
    assert!(!ws.root.join("second/2.0.0.txt").exists());
    let updated: Vec<String> = conn
        .prepare("SELECT name FROM installed_packages_update_log ORDER BY id")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    assert_eq!(updated, ["first"]);
    assert_eq!(fs::read_dir(ws.root.join("var/lib/atlas/tmp"))?.count(), 0);
    let mut entries = fs::read_dir(&ws.root)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort();
    assert_eq!(
        entries,
        [
            std::ffi::OsString::from("first"),
            std::ffi::OsString::from("second"),
            std::ffi::OsString::from("var"),
        ]
    );
    let dangling = count(
        &conn,
        "SELECT COUNT(*) FROM package_dependencies d LEFT JOIN installed_packages p ON p.name = d.dependency_name AND p.architecture = d.architecture WHERE p.name IS NULL",
        &[],
    );
    assert_eq!(dangling, 0);
    Ok(())
}

#[repr(C)]
struct PtyWindowSize {
    rows: u16,
    columns: u16,
    x_pixels: u16,
    y_pixels: u16,
}

unsafe extern "C" {
    fn openpty(
        master: *mut i32,
        slave: *mut i32,
        name: *mut i8,
        termios: *const std::ffi::c_void,
        size: *const PtyWindowSize,
    ) -> i32;
}

fn answer_decline_prompt(test_name: &str, body: impl FnOnce() -> Result<()>) -> Result<()> {
    if env::var("ATLAS_GROUP_PROMPT_CHILD").as_deref() == Ok(test_name) {
        return body();
    }

    let size = PtyWindowSize {
        rows: 24,
        columns: 120,
        x_pixels: 0,
        y_pixels: 0,
    };
    let (mut master_fd, mut slave_fd) = (-1, -1);
    // SAFETY: openpty receives valid pointers to the descriptor outputs and size.
    if unsafe {
        openpty(
            &raw mut master_fd,
            &raw mut slave_fd,
            std::ptr::null_mut(),
            std::ptr::null(),
            &raw const size,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: successful openpty returned distinct owned descriptors.
    let mut master = unsafe { File::from_raw_fd(master_fd) };
    // SAFETY: successful openpty returned distinct owned descriptors.
    let slave = unsafe { File::from_raw_fd(slave_fd) };
    fcntl_setfl(&master, fcntl_getfl(&master)? | OFlags::NONBLOCK)?;
    let mut child = Command::new(std::env::current_exe()?)
        .args(["--exact", test_name, "--nocapture", "--color", "never"])
        .env("ATLAS_GROUP_PROMPT_CHILD", test_name)
        .env("TERM", "xterm")
        .stdin(slave.try_clone()?)
        .stdout(slave.try_clone()?)
        .stderr(slave)
        .spawn()?;

    let deadline = Instant::now() + Duration::from_secs(8);
    let mut output = Vec::new();
    let mut cursor = 0;
    let mut answered = false;
    let status = loop {
        let mut buffer = [0; 4096];
        match master.read(&mut buffer) {
            Ok(size) => output.extend_from_slice(&buffer[..size]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) if error.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error()) => {}
            Err(error) => return Err(error.into()),
        }
        while let Some(offset) = output[cursor..]
            .windows(b"\x1b[6n".len())
            .position(|bytes| bytes == b"\x1b[6n")
        {
            master.write_all(b"\x1b[1;1R")?;
            cursor += offset + 4;
        }
        if !answered
            && output
                .windows(b"(Y/n)".len())
                .any(|bytes| bytes == b"(Y/n)")
        {
            master.write_all(b"n\r")?;
            answered = true;
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!(
                "{test_name} timed out: {}",
                String::from_utf8_lossy(&output)
            );
        }
        thread::sleep(Duration::from_millis(5));
    };
    assert!(
        status.success(),
        "{test_name}: {}",
        String::from_utf8_lossy(&output)
    );
    assert!(answered, "the group update must ask for confirmation");
    Ok(())
}

#[test]
fn canceling_the_group_update_prompt_leaves_install_state_and_target_root_unchanged() -> Result<()>
{
    answer_decline_prompt(
        "canceling_the_group_update_prompt_leaves_install_state_and_target_root_unchanged",
        || {
            let ws = Workspace::new()?;
            ws.publish_in_groups("alpha", "1.0.0", 1, &[], &["default"])?;
            ws.publish_in_groups("beta", "1.0.0", 1, &[], &["default"])?;
            ws.install_group("default")?;
            ws.publish_in_groups("alpha", "2.0.0", 2, &[], &["default"])?;
            ws.publish_in_groups("beta", "2.0.0", 2, &[], &["default"])?;
            set_assume_yes(false);
            let before_root = crate::fixtures::snapshot_tree(ws.root())?;
            let before_alpha = ws.row("alpha")?;
            let before_beta = ws.row("beta")?;

            ws.update_groups("default")?;

            assert_eq!(ws.install_state_snapshot()?.root, before_root);
            let after_alpha = ws.row("alpha")?;
            let after_beta = ws.row("beta")?;
            assert_eq!(after_alpha.version, before_alpha.version);
            assert_eq!(after_alpha.sky_path, before_alpha.sky_path);
            assert_eq!(after_beta.version, before_beta.version);
            assert_eq!(after_beta.sky_path, before_beta.sky_path);
            assert_eq!(ws.get_update_log()?.len(), 0);
            ws.assert_no_staging_dirs()?;
            Ok(())
        },
    )
}

#[test]
fn updating_a_group_updates_each_member_in_its_recorded_install_directory() -> Result<()> {
    let ws = Workspace::new()?;
    let first_root = ws.create_target_root("first-root")?;
    let second_root = ws.create_target_root("second-root")?;
    ws.publish_in_groups("alpha", "1.0.0", 1, &[], &["default"])?;
    ws.publish_in_groups("beta", "1.0.0", 1, &[], &["default"])?;
    ws.install_into("alpha", &first_root)?;
    ws.install_into("beta", &second_root)?;
    assert_eq!(ws.row("alpha")?.install_dir, first_root);
    assert_eq!(ws.row("beta")?.install_dir, second_root);

    ws.publish_in_groups("alpha", "2.0.0", 2, &[], &["default"])?;
    ws.publish_in_groups("beta", "2.0.0", 2, &[], &["default"])?;
    ws.update_groups("default")?;

    ws.assert_installed("alpha", "2.0.0", 2)?;
    ws.assert_installed("beta", "2.0.0", 2)?;
    assert_eq!(ws.row("alpha")?.install_dir, first_root);
    assert_eq!(ws.row("beta")?.install_dir, second_root);
    assert_eq!(
        fs::read(first_root.join("alpha/2.0.0.txt"))?,
        b"alpha 2.0.0\\n"
    );
    assert_eq!(
        fs::read(second_root.join("beta/2.0.0.txt"))?,
        b"beta 2.0.0\\n"
    );
    assert!(!ws.root().join("alpha/2.0.0.txt").exists());
    assert!(!ws.root().join("beta/2.0.0.txt").exists());
    ws.assert_no_dangling_dependencies()?;
    Ok(())
}
