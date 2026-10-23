use std::{os::unix::fs::PermissionsExt, path::PathBuf};

use anyhow::Result;
use sisyphus::signing::signing::keygen;
use tempfile::TempDir;

/// Generates a key pair at an explicit path inside a scratch directory, so no
/// test ever reads or writes the user's real `~/.config/sisyphus` keys.
fn keypair_in(dir: &TempDir) -> Result<(PathBuf, PathBuf)> {
    keygen(&Some(dir.path().join("signer.key")))
}

/// Pin: the private key is written with mode 0o600 regardless of how keygen is
/// refactored. Anyone who can read the private key can forge signatures for
/// every package the machine verifies, so owner-only must not regress to a
/// umask-dependent `std::fs::write`.
#[test]
fn the_private_key_is_readable_and_writable_by_its_owner_only() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (priv_key_path, _) = keypair_in(&dir)?;

    let mode = std::fs::metadata(&priv_key_path)
        .expect("keygen must write the private key")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode, 0o600,
        "the private key must keep its owner-only 0o600 mode"
    );

    Ok(())
}

/// Pin: the public key is written with mode 0o644, explicitly and not by the
/// invoking process's umask. Verifiers on other accounts must be able to read
/// it, so it may neither regress to private-key strictness nor become
/// group/other-writable the way a umask 002 system would leave it.
#[test]
fn the_public_key_is_readable_by_everyone_but_writable_only_by_its_owner() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (_, pub_key_path) = keypair_in(&dir)?;

    let mode = std::fs::metadata(&pub_key_path)
        .expect("keygen must write the public key")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode, 0o644,
        "the public key must keep its world-readable 0o644 mode"
    );

    Ok(())
}
