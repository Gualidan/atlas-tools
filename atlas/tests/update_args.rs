use atlas::cli::cli::UpdateArgs;
use clap::{Args, Command, FromArgMatches, error::ErrorKind};

fn command() -> Command {
    UpdateArgs::augment_args(Command::new("update"))
}

#[test]
fn omitting_the_package_selects_all_installed_packages() {
    let matches = command().try_get_matches_from(["update"]).unwrap();
    let args = UpdateArgs::from_arg_matches(&matches).unwrap();

    assert_eq!(args.package, None);
}

#[test]
fn supplying_a_package_preserves_its_name() {
    let matches = command()
        .try_get_matches_from(["update", "lib-example"])
        .unwrap();
    let args = UpdateArgs::from_arg_matches(&matches).unwrap();

    assert_eq!(args.package.as_deref(), Some("lib-example"));
}

#[test]
fn multiple_package_arguments_are_rejected() {
    let err = command()
        .try_get_matches_from(["update", "foo", "bar"])
        .expect_err("update accepts at most one package");

    assert_eq!(err.kind(), ErrorKind::UnknownArgument);
}

#[test]
fn unknown_options_are_not_treated_as_a_request_to_update_everything() {
    let err = command()
        .try_get_matches_from(["update", "--unknown"])
        .expect_err("a mistyped option must be rejected");

    assert_eq!(err.kind(), ErrorKind::UnknownArgument);
}
