use std::path::PathBuf;

use anyhow::{Context, Result};
use tempfile::{Builder, TempDir};

use crate::build::dependencies::BuildPlan;
use crate::pipeline::build::run_build;
use crate::types::bubblewrap::Bubblewrap;
use crate::types::package::Package;
use crate::types::runtime_config::RuntimeConfig;
use crate::types::sandbox::Sandbox;

pub fn build(
    runtime_config: &RuntimeConfig,
    package: &Package,
    srcdir: &PathBuf,
    build_plan: &Option<BuildPlan>,
) -> Result<TempDir> {
    if let Some(plan) = build_plan {
        for dep in &plan.packages_to_build {
            run_build(&dep.recipe_path, Some(runtime_config), true)
                .with_context(|| format!("Failed to build dependency `{}`", dep.name))?;
        }
    }

    let pkgdir = build_package(package, srcdir)
        .with_context(|| format!("Failed to build package `{}`", package.name))?;
    Ok(pkgdir)
}

pub fn build_package(package: &Package, srcdir: &PathBuf) -> Result<TempDir> {
    let builddir = Builder::new()
        .prefix("sisyphus_builddir")
        .tempdir()
        .context("Failed to create build directory")?;
    let pkgdir = Builder::new()
        .prefix("sisyphus_pkgdir")
        .tempdir()
        .context("Failed to create package directory")?;

    let builddir = builddir.path().to_path_buf();
    let pkgdir_path = pkgdir.path().to_path_buf();
    let bwp = Bubblewrap {
        name: package.name.clone(),
        version: package.version.clone(),
        release: package.release.to_string(),
        arch: package.architecture,
        srcdir: srcdir.clone(),
        builddir,
        pkgdir: pkgdir_path,
    };

    if let Some(prepare) = &package.prepare {
        bwp.run_phase(prepare.clone())
            .context("Failed to run prepare phase")?;
    }
    if let Some(build) = &package.build {
        bwp.run_phase(build.clone())
            .context("Failed to run build phase")?;
    }
    if let Some(check) = &package.check {
        bwp.run_phase(check.clone())
            .context("Failed to run check phase")?;
    }
    bwp.run_phase(package.package.clone())
        .context("Failed to run package phase")?;

    Ok(pkgdir)
}
