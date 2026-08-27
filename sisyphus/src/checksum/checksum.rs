use ring::digest::Digest;

use crate::checksum::hash::hash;
use crate::fetch::fetch::fetch;
use crate::types::error::ChecksumError;
use crate::types::fetcher::FetchedSource;
use crate::types::package::Package;
use std::fs::File;

pub fn checksum_gen(package: &Package) -> Result<Digest, ChecksumError> {
    let (fetched, _) = fetch(package)?;
    let mut hashed: Digest;

    // Generate checksum and print it to the user
    match fetched {
        FetchedSource::Archive(path) => {
            let file = File::open(path)?;
            let hasher = hash(&file)?;
            hashed = hasher;
            println!(
                "Checksum: {}, paste this into your recipe",
                hashed
                    .as_ref()
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect::<String>()
            );
        }
        FetchedSource::Dir(dir) => {}
    }

    Ok(hashed)
}
