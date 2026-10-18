use std::{env, path::PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use common::types::config::load_config;

use crate::{
    install::install::install, types::runtime_config::RuntimeConfig, verify::verify::verify,
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
    /// The name package to install
    pub package: Option<String>,
    // TODO add suport for installing a specific sky file
}

#[derive(Args)]
pub struct VerifyArgs {
    /// The name package to verify
    pub package: Option<String>,

    /// The path to a sky file to install
    pub sky_path: Option<PathBuf>,
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

pub fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.commands {
        Commands::Install(args) => {
            let runtime_config =
                RuntimeConfig::build().context("Failed to build runtime config")?;

            let install_dir =
                env::var("ATLAS_INSTALL_DIR").unwrap_or(String::from("/var/lib/atlas/local"));

            install(args, &runtime_config, PathBuf::from(install_dir))?;
        }
        Commands::Verify(args) => {
            let runtime_config =
                RuntimeConfig::build().context("Failed to build runtime config")?;
            let settings = load_config().context("Failed to load config")?;
            verify(args, &runtime_config, settings)?;
        }
        Commands::Uninstall(_args) => {}
        Commands::Update(_args) => {}
    }
    Ok(())
}
