use anyhow::{Context, Result};
use ring::digest::{Context as DigestContext, Digest, SHA256};
use std::io::Read;

/// Computes the SHA-256 digest of the bytes read from `reader`.
///
/// # Errors
///
/// Returns an error if reading from the source fails.
pub fn hash<R: Read>(mut reader: R) -> Result<Digest> {
    let mut context = DigestContext::new(&SHA256);
    let mut buffer = [0; 1024];

    loop {
        let count = reader
            .read(&mut buffer)
            .context("Failed to read data for hashing")?;
        if count == 0 {
            break;
        }
        context.update(&buffer[..count]);
    }

    Ok(context.finish())
}
