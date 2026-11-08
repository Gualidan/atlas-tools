use std::{fs, path::Path};

use inquire::Confirm;

pub struct SkyGuard<'a> {
    pub sky_path: Option<&'a Path>,
}

impl Drop for SkyGuard<'_> {
    fn drop(&mut self) {
        if let Some(ref path) = self.sky_path {
            let ans = Confirm::new(
                &format!("A sky file was written to the file system but a later critical step failed do you wish to delete the sky file at {}", path.display()),
            )
            .with_default(true)
            .prompt();

            match ans {
                Ok(true) => {
                    println!("Proceeding...");
                }
                Ok(false)
                | Err(
                    inquire::InquireError::OperationInterrupted
                    | inquire::InquireError::OperationCanceled,
                ) => {
                    println!("Transaction aborted");
                }
                Err(inquire::InquireError::NotTTY) => {
                    println!("Command is running outside of a TTY try passing --yes");
                }
                _ => {
                    println!("Failed to read confirmation");
                }
            }

            fs::remove_file(path).expect("Failed to delete sky file");
        }
    }
}

impl<'a> SkyGuard<'a> {
    #[must_use]
    pub fn new(sky_path: &'a Path) -> Self {
        Self {
            sky_path: Some(sky_path),
        }
    }

    pub fn disarm(&mut self) {
        self.sky_path.take();
    }
}
