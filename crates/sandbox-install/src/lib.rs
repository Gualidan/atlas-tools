//! Sandbox group installs for Atlas packages.
//!
//! [`install_group_into_root`] installs a package group — together with its
//! full repository-declared dependency closure — into an arbitrary target
//! root, without touching the host's install state.
//!
//! # Contract
//!
//! - **Non-interactive**: the confirmation prompt is bypassed by enabling
//!   `assume_yes` for the duration of the call; the previous value is
//!   restored afterwards, including on error paths.
//! - **Throwaway install state**: a fresh install database is created inside
//!   a scratch directory that is deleted when the call returns. Every call
//!   therefore resolves and installs the whole group from scratch, and the
//!   host install database (`RuntimeConfig::build`) is never opened for
//!   writing.
//! - **Published input**: the group must already be published in the repo
//!   database. Bootstrapping a repository from recipes (the
//!   chicken-and-egg case) is explicitly out of scope.
//! - **Inherited environment**: like every caller of
//!   `atlas::install::install`, this crate loads the Sisyphus signing
//!   configuration itself, so `SISYPHUS_CONFIG_DIR` (or the default config
//!   path) must be set in the environment.
//! - **Staging**: packages stage under `install_root/var/lib/atlas/tmp`,
//!   exactly as a regular Atlas install does, which is fine because the
//!   staging directory shares a filesystem with the target root.

use std::path::Path;

use anyhow::{Context, Result};
use atlas::{
    cli::confirmation::{assume_yes, set_assume_yes},
    install::install::install,
    types::runtime_config::RuntimeConfig,
};
use tempfile::tempdir;

/// Installs the package group `group` and its dependency closure from the
/// repository database at `repo_db` into `install_root`, non-interactively
/// and without touching the host's install state.
///
/// The repo database is opened read/write only in the sense that
/// `rusqlite::Connection::open` may create the file if it does not exist;
/// resolution and recording write only to the throwaway install database.
/// Every call installs the full group freshly; already-installed state in
/// `install_root` is not consulted.
///
/// # Errors
///
/// Propagates resolution, confirmation, verification, filesystem, and
/// database errors from the underlying install, naming the group in the
/// error chain.
pub fn install_group_into_root(group: &str, repo_db: &Path, install_root: &Path) -> Result<()> {
    let scratch = tempdir().context("Failed to create the sandbox scratch directory")?;
    let runtime_config = RuntimeConfig {
        repo_db: repo_db.to_path_buf(),
        install_db: scratch.path().join("installed.db"),
        sky_repo: repo_db
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf(),
    };

    let previous_assume_yes = assume_yes();
    set_assume_yes(true);
    let result = install(
        None,
        Some(group),
        &runtime_config,
        &install_root.to_path_buf(),
    );
    set_assume_yes(previous_assume_yes);

    result.with_context(|| format!("Failed to install group `{group}`"))
}
