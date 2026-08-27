use crate::{pipeline::build::run_build, signing::signing::keygen, types::error::CliError};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "sisyphus",
    author = "Gualidan",
    version = "0.1.0",
    about = "Package build system for the Atlas package manager"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build a package
    Build(BuildArgs),
    /// Generate RSA keypair
    Keygen(KeygenArgs),
    /// Generate a checksum for the source file and update the YAML
    ChecksumGen(ChecksumArgs),
}

#[derive(Args)]
struct BuildArgs {
    /// Path to the package recipe YAML file
    recipe_path: PathBuf,

    /// Skip build phase (for testing download/extract only)
    #[arg(short, long)]
    skip_build: bool,

    /// Path to the configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,
}

#[derive(Args)]
struct KeygenArgs {
    /// Path to save the RSA keypair
    #[arg(short, long)]
    keypair_path: Option<PathBuf>,
}

#[derive(Args)]
struct ChecksumArgs {
    /// Path to the package recipe YAML file
    recipe_path: PathBuf,
}

pub fn run() -> Result<(), CliError> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Build(args) => {
            if args.skip_build {
                println!("Skipping build.");
                return Ok(());
            }
            run_build(&args.recipe_path, None)?;
            Ok(())
        }
        Commands::Keygen(args) => {
            keygen(&args.keypair_path)?;
            Ok(())
        }
        Commands::ChecksumGen(args) => Ok(()),
    }
}
