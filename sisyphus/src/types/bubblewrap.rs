use crate::types::package::Architectures;
use std::path::PathBuf;

pub struct Bubblewrap {
    name: String,
    version: String,
    release: String,
    arch: Architectures,
    srcdir: PathBuf,
    builddir: PathBuf,
    pkgdir: PathBuf,
}
