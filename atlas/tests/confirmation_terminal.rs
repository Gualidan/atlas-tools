#![cfg(target_os = "linux")]

mod fixtures;

use std::{
    env,
    ffi::{c_char, c_int, c_void},
    fs::{self, File},
    io::{self, Read, Write},
    os::fd::FromRawFd,
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use atlas::{
    cli::{
        cli::UpdateArgs,
        confirmation::{Action, confirmation_prompt, set_assume_yes},
    },
    install::{db::ensure_install_schema, install::install},
    types::runtime_config::RuntimeConfig,
    uninstall::{orphans::uninstall_orphans, uninstall::uninstall},
    update::update::update,
};
use fixtures::{ARCH, ConfigEnv, count, generate_keypair, runtime_config, seed_repo_db, tempdir};
use rusqlite::{Connection, params};
use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};

#[repr(C)]
struct WindowSize {
    rows: u16,
    columns: u16,
    x_pixels: u16,
    y_pixels: u16,
}

unsafe extern "C" {
    fn openpty(
        master: *mut c_int,
        slave: *mut c_int,
        name: *mut c_char,
        termios: *const c_void,
        size: *const WindowSize,
    ) -> c_int;
}

fn terminal_pair() -> io::Result<(File, File)> {
    let (mut master, mut slave) = (-1, -1);
    let size = WindowSize {
        rows: 24,
        columns: 120,
        x_pixels: 0,
        y_pixels: 0,
    };
    // SAFETY: both descriptor outputs and size point to valid storage. Null
    // name/termios request no name output and the platform's default settings.
    if unsafe {
        openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            &size,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful openpty returned two distinct, owned descriptors.
    let pair = unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) };
    fcntl_setfl(&pair.0, fcntl_getfl(&pair.0)? | OFlags::NONBLOCK)?;
    Ok(pair)
}

struct TestChild(Child);

impl Drop for TestChild {
    fn drop(&mut self) {
        // Also reap the child on assertion failures or terminal I/O errors.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Exercise the real terminal backend with a programmatic answer. No human
/// input is read. A deadline prevents extra/unexpected prompts from hanging CI.
fn answer_prompt(test_name: &str, accept: bool, body: impl FnOnce() -> Result<()>) -> Result<()> {
    answer_prompt_with_keys(test_name, if accept { b"\r" } else { b"n\r" }, accept, body)
}

fn answer_prompt_with_keys(
    test_name: &str,
    keys: &[u8],
    accept: bool,
    body: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if env::var_os("ATLAS_TERMINAL_TEST_CHILD").as_deref() == Some(test_name.as_ref()) {
        return body();
    }

    let (mut terminal, slave) = terminal_pair()?;
    let mut child = TestChild(
        Command::new(env::current_exe()?)
            .args(["--exact", test_name, "--nocapture", "--color", "never"])
            .env("ATLAS_TERMINAL_TEST_CHILD", test_name)
            .env("TERM", "xterm")
            .stdin(slave.try_clone()?)
            .stdout(slave.try_clone()?)
            .stderr(slave)
            .spawn()?,
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut output = Vec::new();
    let mut cursor = 0;
    let mut answered = false;
    let status = loop {
        let mut buffer = [0; 4096];
        match terminal.read(&mut buffer) {
            Ok(size) => output.extend_from_slice(&buffer[..size]),
            Err(err)
                if err.kind() == io::ErrorKind::WouldBlock
                    // Linux returns EIO when the last slave descriptor closes.
                    || err.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error()) => {}
            Err(err) => return Err(err.into()),
        }
        // Crossterm asks the terminal for its cursor position during rendering.
        while let Some(offset) = output[cursor..]
            .windows(b"\x1b[6n".len())
            .position(|bytes| bytes == b"\x1b[6n")
        {
            terminal.write_all(b"\x1b[1;1R")?;
            cursor += offset + 4;
        }
        if !answered
            && output
                .windows(b"do you wish to proceed?".len())
                .any(|bytes| bytes == b"do you wish to proceed?")
        {
            terminal.write_all(keys)?;
            answered = true;
        }
        if let Some(status) = child.0.try_wait()? {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "{test_name} timed out:\n{}",
            String::from_utf8_lossy(&output)
        );
        thread::sleep(Duration::from_millis(5));
    };
    // The child can exit between the last read and try_wait. Drain any final
    // output before checking its result so a fast child cannot cause a race.
    loop {
        let mut buffer = [0; 4096];
        match terminal.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => output.extend_from_slice(&buffer[..size]),
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err)
                if err.kind() == io::ErrorKind::WouldBlock
                    || err.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error()) =>
            {
                break;
            }
            Err(err) => return Err(err.into()),
        }
    }
    let output = String::from_utf8_lossy(&output);
    assert!(status.success(), "{test_name}:\n{output}");
    assert!(
        answered,
        "the operation must ask for confirmation: {output}"
    );
    assert_eq!(
        output.matches("Transaction aborted").count(),
        usize::from(!accept),
        "{output}"
    );
    Ok(())
}

fn seed_installation(config: &RuntimeConfig, name: &str, reason: &str) -> Result<()> {
    let conn = Connection::open(&config.install_db)?;
    ensure_install_schema(&conn)?;
    let dir = config.install_db.parent().unwrap().join(name);
    fs::create_dir(&dir)?;
    fs::write(dir.join("sentinel"), b"original")?;
    conn.execute(
        "INSERT INTO installed_packages
         (name, version, release, architecture, sky_path, install_dir, install_reason)
         VALUES (?, '1.0.0', 1, ?, 'old.sky', ?, ?)",
        params![name, ARCH.to_string(), dir.to_str().unwrap(), reason],
    )?;
    conn.execute(
        "INSERT INTO owned_files VALUES (?, ?, ?)",
        params![format!("{name}/sentinel"), name, ARCH.to_string()],
    )?;
    Ok(())
}

fn assert_unchanged(config: &RuntimeConfig, names: &[&str]) -> Result<()> {
    let conn = Connection::open(&config.install_db)?;
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM installed_packages", &[]),
        names.len() as i64
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM owned_files", &[]),
        names.len() as i64
    );
    for name in names {
        let (version, dir): (String, String) = conn.query_row(
            "SELECT version, install_dir FROM installed_packages WHERE name = ?",
            [name],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(version, "1.0.0");
        assert_eq!(
            fs::read(std::path::Path::new(&dir).join("sentinel"))?,
            b"original"
        );
        assert_eq!(fs::read_dir(dir)?.count(), 1);
    }
    Ok(())
}

#[test]
fn declining_install_leaves_no_files_or_install_database() -> Result<()> {
    answer_prompt(
        "declining_install_leaves_no_files_or_install_database",
        false,
        || {
            let tmp = tempdir();
            let mut config = runtime_config(&tmp)?;
            config.install_db = tmp.path().join("atlas/state/installed.db");
            let (_, public_key) = generate_keypair()?;
            let _env = ConfigEnv::new(&tmp, &public_key)?;
            set_assume_yes(false);
            seed_repo_db(&config.repo_db, &[("foo", "1.0.0", 1, "missing.sky")]);
            let root = tmp.path().join("root");

            install(&"foo".into(), &config, &root)?;

            assert!(!root.exists());
            assert!(!config.install_db.exists());
            assert!(!tmp.path().join("atlas").exists());
            Ok(())
        },
    )
}

#[test]
fn declining_uninstall_preserves_files_and_install_records() -> Result<()> {
    answer_prompt(
        "declining_uninstall_preserves_files_and_install_records",
        false,
        || {
            let tmp = tempdir();
            let config = runtime_config(&tmp)?;
            seed_installation(&config, "foo", "explicit")?;

            uninstall(&config, &"foo".into(), true)?;

            assert_unchanged(&config, &["foo"])
        },
    )
}

#[test]
fn declining_uninstall_preserves_every_package_in_the_resolved_plan() -> Result<()> {
    answer_prompt(
        "declining_uninstall_preserves_every_package_in_the_resolved_plan",
        false,
        || {
            let tmp = tempdir();
            let config = runtime_config(&tmp)?;
            seed_installation(&config, "app", "explicit")?;
            seed_installation(&config, "library", "dependency")?;
            Connection::open(&config.install_db)?.execute(
                "INSERT INTO package_dependencies VALUES ('app', ?, 'library')",
                [ARCH.to_string()],
            )?;
            let before = fs::read(&config.install_db)?;

            uninstall(&config, &"app".into(), true)?;

            assert_eq!(fs::read(&config.install_db)?, before);
            assert_unchanged(&config, &["app", "library"])
        },
    )
}

#[test]
fn declining_single_update_preserves_files_and_install_records() -> Result<()> {
    answer_prompt(
        "declining_single_update_preserves_files_and_install_records",
        false,
        || {
            let tmp = tempdir();
            let config = runtime_config(&tmp)?;
            seed_installation(&config, "foo", "explicit")?;
            seed_repo_db(&config.repo_db, &[("foo", "2.0.0", 7, "missing.sky")]);

            update(
                &config,
                UpdateArgs {
                    package: Some("foo".into()),
                },
            )?;

            assert_unchanged(&config, &["foo"])
        },
    )
}

#[test]
fn declining_bulk_update_preserves_every_candidate() -> Result<()> {
    answer_prompt(
        "declining_bulk_update_preserves_every_candidate",
        false,
        || {
            let tmp = tempdir();
            let config = runtime_config(&tmp)?;
            for name in ["alpha", "zeta"] {
                seed_installation(&config, name, "explicit")?;
            }
            seed_repo_db(
                &config.repo_db,
                &[
                    ("alpha", "2.0.0", 7, "missing-alpha.sky"),
                    ("zeta", "3.0.0", 9, "missing-zeta.sky"),
                ],
            );

            update(&config, UpdateArgs { package: None })?;

            assert_unchanged(&config, &["alpha", "zeta"])
        },
    )
}

#[test]
fn declining_clean_preserves_all_orphans() -> Result<()> {
    answer_prompt("declining_clean_preserves_all_orphans", false, || {
        let tmp = tempdir();
        let config = runtime_config(&tmp)?;
        for name in ["alpha", "zeta"] {
            seed_installation(&config, name, "dependency")?;
        }

        uninstall_orphans(&config)?;

        assert_unchanged(&config, &["alpha", "zeta"])
    })
}

#[test]
fn pressing_enter_accepts_the_default_confirmation() -> Result<()> {
    answer_prompt(
        "pressing_enter_accepts_the_default_confirmation",
        true,
        || {
            assert!(confirmation_prompt(
                vec![("foo", "1.0.0")],
                4,
                Action::Install
            )?);
            Ok(())
        },
    )
}

#[test]
fn accepting_clean_asks_once_and_removes_all_orphans() -> Result<()> {
    answer_prompt(
        "accepting_clean_asks_once_and_removes_all_orphans",
        true,
        || {
            let tmp = tempdir();
            let config = runtime_config(&tmp)?;
            for name in ["alpha", "zeta"] {
                seed_installation(&config, name, "dependency")?;
            }

            uninstall_orphans(&config)?;

            for name in ["alpha", "zeta"] {
                assert!(!tmp.path().join(name).exists());
            }
            let conn = Connection::open(&config.install_db)?;
            assert_eq!(
                count(&conn, "SELECT COUNT(*) FROM installed_packages", &[]),
                0
            );
            assert_eq!(count(&conn, "SELECT COUNT(*) FROM owned_files", &[]), 0);
            Ok(())
        },
    )
}

#[test]
fn escape_cancels_confirmation_cleanly() -> Result<()> {
    answer_prompt_with_keys(
        "escape_cancels_confirmation_cleanly",
        b"\x1b",
        false,
        || {
            assert!(!confirmation_prompt(
                vec![("foo", "1.0.0")],
                4,
                Action::Install
            )?);
            Ok(())
        },
    )
}

#[test]
fn ctrl_c_cancels_confirmation_cleanly() -> Result<()> {
    answer_prompt_with_keys(
        "ctrl_c_cancels_confirmation_cleanly",
        b"\x03",
        false,
        || {
            assert!(!confirmation_prompt(
                vec![("foo", "2.0.0")],
                4,
                Action::Update
            )?);
            Ok(())
        },
    )
}
