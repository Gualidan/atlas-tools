//! Dependency planning for Sisyphus builds.
//!
//! This module deliberately *does not* fetch sources or invoke `run_build`.
//! Its only responsibility is to turn one root recipe into a complete,
//! deterministic, cycle-free build plan. The executor will later consume that
//! plan, build missing packages, and assemble a Bubblewrap build root.

use walkdir::WalkDir;

use crate::types::{
    error::BuildError, package::Package, recipe::Recipe, runtime_config::RuntimeConfig,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

/// A recipe that must be built before the requested root package.
///
/// Entries are stored in dependency-first order. For example, if `gcc`
/// requires `binutils`, `binutils` appears before `gcc`.
#[derive(Debug, Clone)]
pub struct RecipeBuild {
    pub name: String,
    pub recipe_path: PathBuf,
}

/// A previously built `.sky` package that can be reused instead of rebuilt.
///
/// The artifact lookup is intentionally a skeleton for now. Once `.sky`
/// metadata exists, this should hold the verified artifact path and enough
/// metadata to resolve its runtime dependency closure.
#[derive(Debug, Clone)]
pub struct ReusableArtifact {
    pub name: String,
    pub artifact_path: PathBuf,
}

/// The complete result of dependency resolution.
///
/// `packages_to_build` is ordered and must be executed in exactly this order.
/// The other collections are also ordered, even when they are initially
/// populated from sets/maps, so that builds are reproducible.
#[derive(Debug, Default)]
pub struct BuildPlan {
    /// Recipes whose `.sky` artifacts do not exist locally yet.
    pub packages_to_build: Vec<RecipeBuild>,
    /// Verified local `.sky` artifacts that the executor may reuse.
    pub reusable_artifacts: Vec<ReusableArtifact>,
    /// Package artifacts that must be extracted into the root mounted read-only
    /// at `/` by Bubblewrap for the requested root package's build.
    pub build_root_packages: Vec<String>,
    /// The requested root recipe's runtime dependencies. These are written to
    /// its resulting `.sky` metadata; they are not build requirements.
    pub root_runtime_deps: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisitState {
    /// The package has not been traversed in this resolution run.
    Unvisited,
    /// The package is on the active DFS path. Encountering it again is a cycle.
    Visiting,
    /// All of the package's required dependencies have been traversed.
    Resolved,
}

/// Shared state for one complete dependency-resolution run.
///
/// This context must be reused for all nested dependencies. Creating a fresh
/// resolver for every dependency loses the active path and misses indirect
/// cycles such as `a -> b -> c -> a`.
struct Resolver {
    config: RuntimeConfig,
    states: HashMap<String, VisitState>,
    active_path: Vec<String>,
    plan: BuildPlan,
}

impl Resolver {
    fn new(config: RuntimeConfig, root_package: &Package) -> Self {
        Self {
            config,
            states: HashMap::new(),
            active_path: Vec::new(),
            plan: BuildPlan {
                // `deps` belong to the root package's final metadata. They are
                // deliberately not passed to `resolve_dependency` here.
                root_runtime_deps: stable_names(&root_package.deps),
                ..BuildPlan::default()
            },
        }
    }

    /// Resolve every package needed before the root recipe may enter its
    /// sandbox. Only `makedeps` start this traversal.
    fn resolve_root(mut self, root_package: &Package) -> Result<BuildPlan, BuildError> {
        for dependency in stable_names(&root_package.makedeps) {
            self.resolve_dependency(&dependency)?;
        }
        Ok(self.plan)
    }

    /// Resolve one package using depth-first traversal.
    ///
    /// The post-order insertion near the end is what produces a valid
    /// dependency-first build order without relying on HashMap/HashSet
    /// iteration order.
    fn resolve_dependency(&mut self, name: &str) -> Result<(), BuildError> {
        match self
            .states
            .get(name)
            .copied()
            .unwrap_or(VisitState::Unvisited)
        {
            VisitState::Resolved => return Ok(()),
            VisitState::Visiting => return Err(BuildError::CircularDependency(self.cycle(name))),
            VisitState::Unvisited => {}
        }

        self.states.insert(name.to_owned(), VisitState::Visiting);
        self.active_path.push(name.to_owned());

        // TODO: Look up a compatible, verified `.sky` artifact in
        // `config.sky_repo`. Compatibility must eventually include package
        // name, version/release policy, architecture, and signature validity.
        // When reuse is implemented, load the artifact's runtime `deps` and
        // resolve that closure before adding it to `reusable_artifacts` and
        // `build_root_packages`.
        if let Some(artifact) = self.find_reusable_artifact(name)? {
            self.plan.build_root_packages.push(name.to_owned());
            self.plan.reusable_artifacts.push(artifact);
        } else {
            let recipe_path = self.recipe_path(name)?;
            let recipe = Recipe { path: &recipe_path }.parse().map_err(|error| {
                BuildError::ResolveDependenciesError(format!(
                    "failed to parse recipe for dependency `{name}`: {error}"
                ))
            })?;

            // A missing artifact must itself be built. First resolve its
            // makedeps, because they are needed to construct *its* build root.
            for dependency in stable_names(&recipe.makedeps) {
                self.resolve_dependency(&dependency)?;
            }

            // The artifact eventually produced for this dependency needs its
            // runtime dependencies when mounted into another package's build
            // root (for example, gcc needs glibc). Resolve them too. The
            // executor will later decide which artifacts to extract into each
            // individual build root.
            for dependency in stable_names(&recipe.deps) {
                self.resolve_dependency(&dependency)?;
            }

            self.plan.packages_to_build.push(RecipeBuild {
                name: name.to_owned(),
                recipe_path,
            });
            self.plan.build_root_packages.push(name.to_owned());
        }

        self.active_path.pop();
        self.states.insert(name.to_owned(), VisitState::Resolved);
        Ok(())
    }

    /// Return the canonical v1 recipe path. Do not scan arbitrary directories:
    /// the repository contract is exactly `core/<name>/sky.yaml`.
    fn recipe_path(&self, name: &str) -> Result<PathBuf, BuildError> {
        let recipe_path = self
            .config
            .recipe_repo
            .join("core")
            .join(name)
            .join("sky.yaml");
        if recipe_path.is_file() {
            Ok(recipe_path)
        } else {
            Err(BuildError::ResolveDependenciesError(format!(
                "dependency `{name}` was not found at `{}`",
                recipe_path.display()
            )))
        }
    }

    /// Skeleton for the local artifact-store lookup.
    fn find_reusable_artifact(&self, _name: &str) -> Result<Option<ReusableArtifact>, BuildError> {
        let artifact_store: &Path = &self.config.sky_repo;

        // TODO: Define the local artifact index/layout, then:
        // 1. locate the candidate `.sky` artifact by exact package identity;
        // 2. read and validate its `metadata.yaml`;
        // 3. verify its Ed25519 signature against trusted keys;
        // 4. return `Some(ReusableArtifact { .. })` only when all checks pass.
        //
        // Returning `None` currently means every dependency is planned from
        // its recipe, which is safer than treating an arbitrary directory as
        // an installed package.
        //
        Ok(None)
    }

    fn cycle(&self, repeated_name: &str) -> String {
        let start = self
            .active_path
            .iter()
            .position(|name| name == repeated_name)
            .unwrap_or(0);
        let mut cycle = self.active_path[start..].to_vec();
        cycle.push(repeated_name.to_owned());
        cycle.join(" -> ")
    }
}

/// Sort and deduplicate names before traversal. This makes independent nodes
/// resolve in a stable order and protects the plan from duplicate recipe
/// entries until recipe validation rejects duplicates directly.
fn stable_names(names: &[String]) -> Vec<String> {
    let mut result = names.to_vec();
    result.sort();
    result.dedup();
    result
}

/// Create the dependency plan for one root package.
///
/// The caller must execute this plan separately. In particular, do not call
/// `run_build` from this module: the complete graph must be known before the
/// first package build starts.
pub fn resolve(config: RuntimeConfig, root_package: &Package) -> Result<BuildPlan, BuildError> {
    Resolver::new(config, root_package).resolve_root(root_package)
}
