use crate::types::error::Cli as CliError;
use clap::Parser;

#[derive(Parser)]
#[command(
    name = "sisyphus",
    author = "Gualidan",
    version = "0.1.0",
    about = "Package build system for the Atlas package manager"
)]
pub struct Cli {
    #[command(subcommand)]
    pub commands: Commands,
}

#[derive(Parser)]
pub enum Commands {}

pub fn run() -> Result<(), CliError> {
    Ok(())
}
