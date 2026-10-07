use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::types::error::Cli as CliError;

#[derive(Parser)]
#[command(
    name = "atlas",
    author = "Gualidan",
    version = env!("CARGO_PKG_VERSION"),
    about = "The Atlas package manager"
)]
struct Cli {
    #[command(subcommand)]
    commands: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Install(InstallArgs),
    Verify(VerifyArgs),
    Uninstall(UninstallArgs),
    Update(UpdateArgs),
}

#[derive(Args)]
struct InstallArgs {
    /// The package to install
    package: String,
    /// The path to a sky file to install
    sky_path: Option<PathBuf>,
}

#[derive(Args)]
struct VerifyArgs {
    /// The package to verify
    package: String,
}

#[derive(Args)]
struct UninstallArgs {
    /// The package to uninstall
    package: String,
}

#[derive(Args)]
struct UpdateArgs {
    /// The package to update
    package: Option<String>,
}

pub fn run() -> Result<(), CliError> {
    let cli = Cli::parse();

    match &cli.commands {
        Commands::Install(args) => {
            let sky_path = if let Some(path) = &args.sky_path {
                Some(PathBuf::from(path))
            } else {
                None
            };
            // let context = Context {
            //     package: &args.package,
            //     sky_path: sky_path,
            //     sky_repo: PathBuf::new(),
            //     install_dir: PathBuf::new(),
            //     temp_dir: PathBuf::new(),
            //     settings: &load_settings()?,
            //     dependencies: Vec::new(),
            // };
        }
        Commands::Verify(args) => {}
        Commands::Uninstall(args) => {}
        Commands::Update(args) => {}
    }
    Ok(())
}
