use std::fs::File;
use std::path::PathBuf;

use common::functions::hash::hash;
use common::types::error::ChecksumError;
use common::types::package::Package;

pub fn verify(file: &PathBuf, package: &Package) -> Result<(), ChecksumError> {
    let computed = hash(File::open(file)?)?;
    let expected = package.source.sha256.clone();

    let computed = computed
        .as_ref()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();
    let expected = expected.unwrap();
    if computed != expected {
        return Err(ChecksumError::ChecksumMismatch(format!(
            "Expected: {expected}, got: {computed}"
        )));
    }
    Ok(())
}
