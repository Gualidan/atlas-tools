use std::{fmt::Display, path::PathBuf};

use anyhow::Result;

#[derive(Clone)]
pub struct ManifestLine {
    pub file_type: FileType,
    pub mode: u32,
    pub target_or_hash: String,
    pub path: PathBuf,
}

impl ManifestLine {
    pub fn format(self) -> Result<String> {
        let mode = self.mode & 0o7777;
        Ok(format!(
            "{}\t{:04o}\t{}\t{}\n",
            self.file_type,
            mode,
            self.target_or_hash,
            self.path.display()
        ))
    }
}

#[derive(Clone)]
pub enum FileType {
    File,
    Dir,
    Symlink,
}

impl Display for FileType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileType::File => write!(f, "f"),
            FileType::Dir => write!(f, "d"),
            FileType::Symlink => write!(f, "l"),
        }
    }
}
