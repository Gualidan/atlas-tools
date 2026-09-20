use std::path::PathBuf;

use crate::types::error::PackageError;

pub struct ManifestLine {
    pub file_type: FileType,
    pub mode: u32,
    pub target_or_hash: String,
    pub path: PathBuf,
}

impl ManifestLine {
    pub fn format(self) -> Result<String, PackageError> {
        Ok(format!(
            "{}\t{:o}\t{}\t{}\n",
            self.file_type.to_string(),
            self.mode,
            self.target_or_hash,
            self.path.display()
        ))
    }
}

pub enum FileType {
    File,
    Dir,
    Symlink,
}

impl ToString for FileType {
    fn to_string(&self) -> String {
        match self {
            FileType::File => "file".to_string(),
            FileType::Dir => "directory".to_string(),
            FileType::Symlink => "symlink".to_string(),
        }
    }
}
