use crate::{
    pipeline::build::run_build,
    types::{error::BuildError, package::Package, recipe::Recipe, runtime_config::RuntimeConfig},
};
use std::collections::HashSet;
use std::path::PathBuf;
use walkdir::WalkDir;

pub struct Dependencies {
    pub dependencies: Vec<String>,
    pub built: HashSet<String>,
    pub to_be_built: HashSet<String>,
    pub in_progress: HashSet<String>,
}

impl Dependencies {
    pub fn prepare(package: Package) -> Result<Self, BuildError> {
        Ok(Self {
            dependencies: package.dependencies,
            built: HashSet::new(),
            to_be_built: HashSet::new(),
            in_progress: HashSet::from([package.name]),
        })
    }

    pub fn resolve(&mut self, config: &RuntimeConfig) -> Result<(), BuildError> {
        'dependencies: for dep in &self.dependencies {
            if self.built.contains(dep) {
                continue;
            }
            if self.in_progress.contains(dep) {
                return Err(BuildError::CircularDependency(dep.clone()));
            }

            for entry in WalkDir::new(&config.sky_repo)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if entry.file_name().to_string_lossy() == *dep {
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
            if !self.built.contains(dep) && !self.to_be_built.contains(dep) {
                return Err(BuildError::ResolveDependenciesError());
            }
        }

        Ok(())
    }
}

pub fn build(config: RuntimeConfig, package: Package) -> Result<(), BuildError> {
    let mut deps = Dependencies::prepare(package)?;
    deps.resolve(&config)?;
    for dep in deps.to_be_built {
        let recipe = Recipe {
            path: &PathBuf::from(&config.recipe_repo)
                .join(dep)
                .join("sky")
                .with_extension("yaml"),
        };
        run_build(recipe.path, Some(config.clone()))?;
    }
    Ok(())
}
