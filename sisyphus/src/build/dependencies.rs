use crate::types::{error::BuildError, package::Package, runtime_config::RuntimeConfig};
use std::collections::HashSet;
use walkdir::WalkDir;

pub struct Dependencies {
    pub dependencies: Vec<String>,
    pub built: HashSet<String>,
    pub to_be_built: HashSet<String>,
}

impl Dependencies {
    pub fn prepare(package: Package) -> Result<Self, BuildError> {
        Ok(Self {
            dependencies: package.dependencies,
            built: HashSet::new(),
            to_be_built: HashSet::new(),
        })
    }

    pub fn resolve(&mut self, config: RuntimeConfig) -> Result<(), BuildError> {
        'dependencies: for dep in &self.dependencies {
            if self.built.contains(dep) {
                continue;
            }

            for entry in WalkDir::new(&config.sky_repo) {
                if entry?.file_name().to_string_lossy() == *dep {
                    self.built.insert(dep.clone());
                    continue 'dependencies;
                }
            }

            for entry in WalkDir::new(&config.recipe_repo)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_dir())
            {
                if entry.file_name().to_string_lossy() == *dep {
                    self.to_be_built.insert(dep.clone());
                    continue 'dependencies;
                }
            }
        }
        Ok(())
    }
}
