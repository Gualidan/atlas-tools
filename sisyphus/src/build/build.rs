use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use common::functions::unpack_payload::unpack_payload;
use sandbox_install::install_group_into_root;
use tempfile::{Builder, TempDir};

use crate::build::dependencies::BuildPlan;
use crate::pipeline::build::run_planned_build;
use crate::types::bubblewrap::Bubblewrap;
use crate::types::package::Package;
use crate::types::runtime_config::RuntimeConfig;
use crate::types::sandbox::Sandbox;

const BASE_DEVEL: &str = "base-devel";

/// Builds the requested package and any dependencies not already available as artifacts.
///
/// # Errors
///
/// Returns an error if a dependency or the requested package cannot be built.
pub fn build(
    runtime_config: &RuntimeConfig,
    package: &Package,
    srcdir: &Path,
    build_plan: &BuildPlan,
) -> Result<TempDir> {
    let mut artifacts: Vec<PathBuf> = build_plan
        .reusable_artifacts
        .iter()
        .map(|artifact| artifact.artifact_path.clone())
        .collect();

    for dep in &build_plan.packages_to_build {
        let artifact = run_planned_build(&dep.recipe_path, runtime_config, &artifacts)
            .with_context(|| format!("Failed to build dependency `{}`", dep.name))?;
        artifacts.push(artifact);
    }

    let pkgdir = build_package(&runtime_config.repo_db_path, package, srcdir, &artifacts)
        .with_context(|| format!("Failed to build package `{}`", package.name))?;
    Ok(pkgdir)
}

/// Runs the optional prepare, build, and check phases, then the required package
/// phase in Bubblewrap, returning the temporary package output directory.
/// `srcdir` is mounted writable as `/srcdir`; `artifacts` supply payloads for
/// the sandbox's `/usr`. The source directory can be modified by the phases.
/// Propagates temporary-directory, artifact extraction, sandbox startup, and
/// phase execution errors, including nonzero phase exit statuses.
///
/// # Errors
///
/// Returns an error if temporary directories cannot be created, artifacts cannot
/// be unpacked, or a sandboxed phase fails.
pub fn build_package(
    repo_db: &Path,
    package: &Package,
    srcdir: &Path,
    artifacts: &[PathBuf],
) -> Result<TempDir> {
    let builddir = Builder::new()
        .prefix("sisyphus_builddir")
        .tempdir()
        .context("Failed to create build directory")?;
    let pkgdir = Builder::new()
        .prefix("sisyphus_pkgdir")
        .tempdir()
        .context("Failed to create package directory")?;
    let usr_root = Builder::new()
        .prefix("sisyphus_usr_root")
        .tempdir()
        .context("Failed to create temporary directory")?;

    let builddir = builddir.path().to_path_buf();
    let pkgdir_path = pkgdir.path().to_path_buf();
    let usr_root_path = usr_root.path().to_path_buf();

    assemble_sandbox_usr(artifacts, repo_db, &usr_root)
        .context("Failed to assemble sandbox /usr")?;

    let bwp = Bubblewrap {
        name: package.name.clone(),
        version: package.version.clone(),
        release: package.release.to_string(),
        arch: package.architecture,
        srcdir: srcdir.to_path_buf(),
        builddir,
        pkgdir: pkgdir_path,
        usr_root: usr_root_path,
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

/// Unpacks each package artifact into the temporary build root.
///
/// # Errors
///
/// Returns an error if the root directory cannot be created or an artifact
/// cannot be unpacked.
pub fn extract_artifacts(artifacts: &[PathBuf], temp_root: &TempDir) -> Result<()> {
    fs::create_dir_all(temp_root.path().join("usr"))?;
    for artifact in artifacts {
        unpack_payload(artifact, temp_root).context("Failed to unpack payload")?;
    }
    Ok(())
}

/// Assembles a sandbox `/usr` root by extracting the provided package artifacts
/// and installing the `base-devel` group from the repository database.
///
/// # Errors
///
/// Returns an error if artifacts cannot be extracted or the `base-devel` group
/// cannot be installed into the root.
pub fn assemble_sandbox_usr<P: AsRef<Path>>(
    artifacts: &[PathBuf],
    repo_db: P,
    usr_root: &TempDir,
) -> Result<()> {
    extract_artifacts(artifacts, usr_root).context("Failed to extract artifacts")?;

    // Should be fine since makedepes that are part of the base-devel group will not be allowed
    install_group_into_root(BASE_DEVEL, repo_db.as_ref(), usr_root.path())
        .context("Failed to install group into root")?;

    Ok(())
}
