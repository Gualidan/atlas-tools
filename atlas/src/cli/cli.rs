use clap::Parser;

#[derive(Parser)]
#[command(
    name = "atlas",
    author = "Gualidan",
    version = "0.1.0",
    about = "The Atlas package manager"
)]
pub struct Cli {
    #[command(subcommand)]
    pub commands: Commands,
}

#[derive(Parser)]
pub enum Commands {}
