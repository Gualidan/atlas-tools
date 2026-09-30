use std::path::PathBuf;

use tempfile::{Builder, TempDir};

use crate::build::dependencies::BuildPlan;
use crate::pipeline::build::run_build;
use crate::types::bubblewrap::Bubblewrap;
use crate::types::error::BuildError;
use crate::types::package::Package;
use crate::types::runtime_config::RuntimeConfig;
use crate::types::sandbox::Sandbox;

pub fn build(
    runtime_config: &RuntimeConfig,
    package: &Package,
    srcdir: &PathBuf,
    build_plan: &Option<BuildPlan>,
) -> Result<TempDir, BuildError> {
    if let Some(plan) = build_plan {
        for dep in &plan.packages_to_build {
            run_build(&dep.recipe_path, Some(runtime_config), true)?;
        }
    }

    let pkgdir = build_package(package, srcdir)?;
    Ok(pkgdir)
}

pub fn build_package(package: &Package, srcdir: &PathBuf) -> Result<TempDir, BuildError> {
    let builddir = Builder::new().prefix("sisyphus_builddir").tempdir()?;
    let pkgdir = Builder::new().prefix("sisyphus_pkgdir").tempdir()?;

    let builddir = builddir.path().to_path_buf();
    let pkgdir_path = pkgdir.path().to_path_buf();
    let bwp = Bubblewrap {
        name: package.name.clone(),
        version: package.version.clone(),
        release: package.release.to_string(),
        arch: package.architecture,
        srcdir: srcdir.clone(),
        builddir: builddir,
        pkgdir: pkgdir_path,
    };

    if let Some(prepare) = &package.prepare {
        bwp.run_phase(prepare.clone())?;
    }
    if let Some(build) = &package.build {
        bwp.run_phase(build.clone())?;
    }
    if let Some(check) = &package.check {
        bwp.run_phase(check.clone())?;
    }
    bwp.run_phase(package.package.clone())?;

    Ok(pkgdir)
}
