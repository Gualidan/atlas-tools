use std::{env, path::PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use common::types::config::load_config;

use crate::{
    install::install::{install, set_assume_yes},
    types::runtime_config::RuntimeConfig,
    uninstall::{orphans::uninstall_orphans, uninstall::uninstall},
    update::update::update,
    verify::verify::verify,
};

#[derive(Parser)]
#[command(
    name = "atlas",
    author = "Gualidan",
    version = env!("CARGO_PKG_VERSION"),
    about = "The Atlas package manager"
)]
struct Cli {
    /// Assume yes for all confirmation prompts
    #[arg(short = 'y', long = "yes", global = true)]
    assume_yes: bool,

    #[command(subcommand)]
    commands: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Install(InstallArgs),
    Verify(VerifyArgs),
    Uninstall(UninstallArgs),
    Update(UpdateArgs),
    Clean(CleanArgs),
}

#[derive(Args)]
pub struct InstallArgs {
    /// The name package to install
    pub package: String,
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
pub struct UninstallArgs {
    /// The package to uninstall
    pub package: String,
}

#[derive(Args)]
pub struct UpdateArgs {
    /// A package to update
    pub package: Option<String>,
}

#[derive(Args)]
pub struct CleanArgs {}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    set_assume_yes(cli.assume_yes);

    match cli.commands {
        Commands::Install(args) => {
            let runtime_config =
                RuntimeConfig::build().context("Failed to build runtime config")?;

            let install_dir =
                env::var("ATLAS_INSTALL_DIR").unwrap_or(String::from("/var/lib/atlas/local"));

            install(&args.package, &runtime_config, PathBuf::from(install_dir))?;
        }
        Commands::Verify(args) => {
            let runtime_config =
                RuntimeConfig::build().context("Failed to build runtime config")?;
            let settings = load_config().context("Failed to load config")?;
            verify(args, &runtime_config, settings)?;
        }
        Commands::Uninstall(args) => {
            let runtime_config =
                RuntimeConfig::build().context("Failed to build runtime config")?;

            uninstall(&runtime_config, &args.package, true)?;
        }
        Commands::Update(args) => {
            let runtime_config =
                RuntimeConfig::build().context("Failed to build runtime config")?;
            update(&runtime_config, args)?;
        }
        Commands::Clean(_) => {
            let runtime_config =
                RuntimeConfig::build().context("Failed to build runtime config")?;
            uninstall_orphans(&runtime_config)?;
        }
    }
    Ok(())
}
