use ring::digest::{Context, Digest, SHA256};
use std::io::Read;

use crate::types::error::ChecksumError;

pub fn hash<R: Read>(mut reader: R) -> Result<Digest, ChecksumError> {
    let mut context = Context::new(&SHA256);
    let mut buffer = [0; 1024];

    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        context.update(&buffer[..count]);
    }

    Ok(context.finish())
}
