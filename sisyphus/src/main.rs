mod build;
mod checksum;
mod cli;
mod extract;
mod fetch;
mod pipeline;
mod signing;
mod types;

use crate::cli::cli::run;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::FAILURE
        }
    }
}
