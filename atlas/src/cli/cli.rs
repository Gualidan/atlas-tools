use std::{env, path::PathBuf};

use clap::{Args, Parser, Subcommand};

use crate::{
    cli::install::install,
    types::{error::Cli as CliError, runtime_config::RuntimeConfig},
};

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
pub struct InstallArgs {
    /// The package to install
    pub package: String,
    /// The path to a sky file to install
    pub sky_path: Option<PathBuf>,
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
            let runtime_config = RuntimeConfig::build()?;
            let install_dir =
                env::var("ATLAS_INSTALL_DIR").unwrap_or(String::from("/var/lib/atlas/local"));

            install(args, &runtime_config, PathBuf::from(install_dir))?;
        }
        Commands::Verify(args) => {}
        Commands::Uninstall(args) => {}
        Commands::Update(args) => {}
    }
    Ok(())
}
