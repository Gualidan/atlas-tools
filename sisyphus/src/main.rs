mod cli;
mod pipeline;
mod types;

use crate::cli::cli::run;
use crate::types::error::CliError;

fn main() -> Result<(), CliError> {
    run()?;
    Ok(())
}
