use crate::checksum::hash::hash;
use crate::fetch::fetch::fetch;
use crate::types::error::ChecksumError;
use crate::types::fetcher::FetchedSource;
use crate::types::package::Package;
use ring::digest::Digest;
use std::fs::File;
use std::path::PathBuf;

pub enum ChecksumResult {
    Hashed(Digest),
    Dir(PathBuf),
}

pub fn checksum_gen(package: &Package) -> Result<ChecksumResult, ChecksumError> {
    #[allow(unused_variables)]
    let (fetched, temp_dir) = fetch(package, false)?;

    // Generate checksum and print it to the user
    match fetched {
        FetchedSource::Archive(path) => {
            let file = File::open(path)?;
            let hasher = hash(&file)?;

            println!(
                "Checksum: {}, paste this into your recipe",
                hasher
                    .as_ref()
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect::<String>()
            );
            Ok(ChecksumResult::Hashed(hasher))
        }
        FetchedSource::Dir(dir) => Ok(ChecksumResult::Dir(dir)),
    }
}
