use std::sync::atomic::{AtomicBool, Ordering};

pub static ASSUME_YES: AtomicBool = AtomicBool::new(false);

/// Skips interactive confirmation prompts when set to `true`. Intended for the
/// CLI's `--yes` flag and for non-interactive test runs.
pub fn set_assume_yes(assume_yes: bool) {
    ASSUME_YES.store(assume_yes, Ordering::Relaxed);
}
