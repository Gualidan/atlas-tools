use std::{
    fmt::Display,
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::{Result, anyhow};
use inquire::{
    Confirm,
    InquireError::{NotTTY, OperationCanceled, OperationInterrupted},
};

pub enum Action {
    Install,
    Update,
    Uninstall,
}

impl Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Action::Install => write!(f, "install"),
            Action::Update => write!(f, "update"),
            Action::Uninstall => write!(f, "uninstall"),
        }
    }
}

static ASSUME_YES: AtomicBool = AtomicBool::new(false);

/// Skips interactive confirmation prompts when set to `true`. Intended for the
/// CLI's `--yes` flag and for non-interactive test runs.
pub fn set_assume_yes(assume_yes: bool) {
    ASSUME_YES.store(assume_yes, Ordering::Relaxed);
}

pub fn confirmation_prompt(
    packages: Vec<(&str, &str)>,
    indent: usize,
    action: Action,
) -> Result<bool> {
    if packages.is_empty() {
        return Ok(true);
    }

    println!("Packages:");
    for (name, version) in packages {
        println!("{:indent$}{}-{}", "", name, version, indent = indent);
    }

    if ASSUME_YES.load(Ordering::Relaxed) {
        return Ok(true);
    }

    let ans = Confirm::new(&format!(
        "This will {} these packages, do you wish to proceed?",
        action
    ))
    .with_default(true)
    .prompt();

    match ans {
        Ok(true) => return Ok(true),
        Ok(false) => {
            println!("Transaction aborted");
            return Ok(false);
        }
        Err(e) => match e {
            OperationInterrupted => {
                println!("Transaction aborted: user pressed Ctrl-C");
                return Ok(false);
            }
            OperationCanceled => {
                println!("Transaction aborted: user pressed Esc");
                return Ok(false);
            }
            NotTTY => {
                return Err(anyhow!(
                    "Command is running outside of a TTY try passing --yes"
                ));
            }
            _ => {
                return Err(anyhow!(e).context("Failed to read confirmation"));
            }
        },
    }
}
