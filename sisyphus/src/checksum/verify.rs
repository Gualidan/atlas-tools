use std::fs::File;
use std::path::PathBuf;

use crate::checksum::hash::hash;
use crate::types::error::ChecksumError;
use crate::types::package::Package;

pub fn verify(file: &PathBuf, package: &Package) -> Result<(), ChecksumError> {
    let computed = hash(File::open(file)?)?;
    let expected = package.checksum.clone();

    let computed = computed
        .as_ref()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();
    if computed != expected {
        return Err(ChecksumError::ChecksumMismatchError);
    }
    Ok(())
}
