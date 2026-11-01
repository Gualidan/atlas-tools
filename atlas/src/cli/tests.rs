use super::*;
use clap::error::ErrorKind;
use std::{env, path::PathBuf, sync::Mutex};

#[test]
fn confirmation_is_required_by_default_for_mutating_commands() {
    for args in [
        vec!["atlas", "install", "foo"],
        vec!["atlas", "uninstall", "foo"],
        vec!["atlas", "update", "foo"],
        vec!["atlas", "update"],
        vec!["atlas", "clean"],
    ] {
        let cli = Cli::try_parse_from(&args).unwrap();
        assert!(!cli.assume_yes, "{args:?}");
    }
}

#[test]
fn both_yes_flags_work_before_and_after_every_mutating_subcommand() {
    for flag in ["--yes", "-y"] {
        for command in ["install", "uninstall", "update", "clean"] {
            let package = matches!(command, "install" | "uninstall" | "update");
            for position in 1..=if package { 3 } else { 2 } {
                let mut args = vec!["atlas", command];
                if package {
                    args.push("foo");
                }
                args.insert(position, flag);

                let cli = Cli::try_parse_from(&args).unwrap();
                assert!(cli.assume_yes, "{args:?}");
                match cli.commands {
                    Commands::Install(args) => assert_eq!(args.package, "foo"),
                    Commands::Uninstall(args) => assert_eq!(args.package, "foo"),
                    Commands::Update(args) => assert_eq!(args.package.as_deref(), Some("foo")),
                    Commands::Clean(_) => {}
                    Commands::Verify(_) => panic!("unexpected verify command: {args:?}"),
                }
            }
        }
    }
}

#[test]
fn yes_does_not_supply_the_optional_update_package() {
    for args in [
        ["atlas", "--yes", "update"],
        ["atlas", "update", "--yes"],
        ["atlas", "-y", "update"],
        ["atlas", "update", "-y"],
    ] {
        let cli = Cli::try_parse_from(args).unwrap();
        assert!(cli.assume_yes);
        let Commands::Update(args) = cli.commands else {
            panic!("expected update command");
        };
        assert!(args.package.is_none());
    }
}

#[test]
fn a_global_yes_flag_is_also_accepted_by_verify() {
    let cli = Cli::try_parse_from(["atlas", "verify", "-y", "foo"]).unwrap();
    assert!(cli.assume_yes);
    let Commands::Verify(args) = cli.commands else {
        panic!("expected verify command");
    };
    assert_eq!(args.package.as_deref(), Some("foo"));
}

#[test]
fn yes_does_not_relax_required_arguments_or_accept_a_boolean_value() {
    for command in ["install", "uninstall"] {
        let err = Cli::try_parse_from(["atlas", command, "--yes"])
            .err()
            .expect("a package is still required");
        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
    }
    let err = Cli::try_parse_from(["atlas", "--yes=false", "clean"])
        .err()
        .expect("--yes is a switch, not a boolean-valued option");
    assert_eq!(err.kind(), ErrorKind::TooManyValues);
}

#[test]
fn yes_after_the_option_terminator_is_a_package_name() {
    let cli = Cli::try_parse_from(["atlas", "install", "--", "--yes"]).unwrap();
    assert!(!cli.assume_yes);
    let Commands::Install(args) = cli.commands else {
        panic!("expected install command");
    };
    assert_eq!(args.package, "--yes");
}

#[test]
fn install_root_defaults_to_slash_and_accepts_an_explicit_root() {
    let cli = Cli::try_parse_from(["atlas", "install", "tool"]).unwrap();
    let Commands::Install(args) = cli.commands else {
        panic!("expected install command");
    };
    assert_eq!(
        args.root.unwrap_or_else(|| PathBuf::from("/")),
        PathBuf::from("/")
    );

    let cli = Cli::try_parse_from(["atlas", "install", "tool", "--root", "/tmp/atlas-test-root"])
        .unwrap();
    let Commands::Install(args) = cli.commands else {
        panic!("expected install command");
    };
    assert_eq!(args.root, Some(PathBuf::from("/tmp/atlas-test-root")));
}

#[test]
fn explicit_install_root_wins_over_legacy_install_dir_environment() {
    static ENV_LOCK: Mutex<()> = Mutex::new(());
    let _lock = ENV_LOCK.lock().unwrap();
    let previous = env::var_os("ATLAS_INSTALL_DIR");
    unsafe { env::set_var("ATLAS_INSTALL_DIR", "/legacy/atlas/install-dir") };

    let cli = Cli::try_parse_from([
        "atlas",
        "install",
        "tool",
        "--root",
        "/tmp/atlas-explicit-root",
    ])
    .unwrap();
    let parsed_root = match cli.commands {
        Commands::Install(args) => args.root,
        _ => panic!("expected install command"),
    };

    match previous {
        Some(value) => unsafe { env::set_var("ATLAS_INSTALL_DIR", value) },
        None => unsafe { env::remove_var("ATLAS_INSTALL_DIR") },
    }

    assert_eq!(parsed_root, Some(PathBuf::from("/tmp/atlas-explicit-root")));
}
