use std::{collections::HashSet, fs};

use crate::types::{config::Config, error::BuildError, package::Package};

pub fn resolve(package: Package, config: Config) -> Result<HashSet<String>, BuildError> {
    let mut built = HashSet::new();
    let mut to_be_built: HashSet<String> = HashSet::new();
    for dep in package.dependencies {
        let built_entries: Vec<_> = fs::read_dir(&config.sky_repo)?
            .filter_map(|e| e.ok())
            .collect();
        for entry in built_entries {
            if entry.file_name().to_string_lossy() == dep {
                built.insert(dep.clone());
                continue;
            }
        }
        let entries: Vec<_> = fs::read_dir(&config.recipe_repo)?
            .filter_map(|e| e.ok())
            .collect();

        for entry in entries {
            if entry.file_name().to_string_lossy() == dep {
                to_be_built.insert(dep.clone());
                continue;
            }
        }
    }
    Ok(to_be_built)
}
