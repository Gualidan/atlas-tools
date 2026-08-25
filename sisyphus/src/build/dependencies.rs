use std::fs;

use crate::types::{config::Config, error::BuildError, package::Package};

pub fn resolve(package: Package, config: Config) -> Result<(), BuildError> {
    for dep in package.dependencies {
        let entries: Vec<_> = fs::read_dir(&config.sky_repo)?
            .filter_map(|e| e.ok())
            .collect();
        for entry in entries {
            if entry.file_name().to_string_lossy() == dep {
                todo!()
            }
        }
    }
    Ok(())
}
