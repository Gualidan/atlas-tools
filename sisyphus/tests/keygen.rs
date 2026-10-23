use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use anyhow::Result;
use sisyphus::signing::signing::keygen;
use tempfile::TempDir;

/// Generates a key pair at an explicit path inside a scratch directory, so no
/// test ever reads or writes the user's real `~/.config/sisyphus` keys.
fn keypair_in(dir: &TempDir) -> Result<(PathBuf, PathBuf)> {
    keygen(&Some(dir.path().join("signer.key")))
}

/// umask is process-global state and `cargo test` runs a test binary's tests on
/// parallel threads, so the two masked windows below are serialized with a
/// lock, and every window puts the process's previous umask back.
static UMASK_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn with_umask<T>(mask: libc::mode_t, run: impl FnOnce() -> T) -> T {
    let lock: MutexGuard<'static, ()> = UMASK_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let previous = unsafe { libc::umask(mask) };
    let result = run();
    unsafe { libc::umask(previous) };
    drop(lock);
    result
}

fn mode_of(path: &PathBuf) -> u32 {
    std::fs::metadata(path)
        .expect("keygen must write the key file")
        .permissions()
        .mode()
        & 0o777
}

/// Pin: the private key is written with mode 0o600 regardless of how keygen is
/// refactored. The process umask is forced to 0o077 so the assertion cannot be
/// satisfied by a friendly default mask. Anyone who can read the private key
/// can forge signatures for every package the machine verifies, so owner-only
/// must not regress to a umask-dependent `std::fs::write`.
#[test]
fn the_private_key_is_readable_and_writable_by_its_owner_only() -> Result<()> {
    let dir = TempDir::new()?;
    let (priv_key_path, _) = with_umask(0o077, || keypair_in(&dir))?;

    let mode = mode_of(&priv_key_path);
    assert_eq!(
        mode, 0o600,
        "under umask 0o077 the private key must keep its owner-only 0o600 mode (came out {mode:#04o})"
    );

    Ok(())
}

/// Pin: the public key is written with mode 0o644 even when the process umask
/// would mask the world-read bits away. Because `OpenOptionsExt::mode` at
/// creation time is filtered by the umask at open(2), a pub key whose mode is
/// only ever set at creation comes out 0o600 under 0o077 — exactly what this
/// test watches for. Verifiers on other accounts must be able to read it.
#[test]
fn the_public_key_is_readable_by_everyone_but_writable_only_by_its_owner() -> Result<()> {
    let dir = TempDir::new()?;
    let (_, pub_key_path) = with_umask(0o077, || keypair_in(&dir))?;

    let mode = mode_of(&pub_key_path);
    assert_eq!(
        mode, 0o644,
        "under umask 0o077 the public key must still be world-readable 0o644 (came out {mode:#04o})"
    );

    Ok(())
}
