use std::path::PathBuf;

pub struct ManifestLine {
    pub file_type: FileType,
    pub mode: u32,
    pub target_or_hash: String,
    pub path: PathBuf,
}

pub enum FileType {
    File,
    Dir,
    Symlink,
}
