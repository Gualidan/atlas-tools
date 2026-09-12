use which::which;

use crate::types::{package::Architectures, sandbox::Sandbox};
use std::{path::PathBuf, process::Command};

pub struct Bubblewrap {
    pub name: String,
    pub version: String,
    pub release: String,
    pub arch: Architectures,
    pub srcdir: PathBuf,
    pub builddir: PathBuf,
    pub pkgdir: PathBuf,
}

impl Sandbox for Bubblewrap {
    fn run_phase(&self, phase: String) -> Result<(), super::error::SandboxError> {
        which("bwrap")?;
        let cmd = Command::new("bwrap")
            .arg("--bind")
            .arg(&self.srcdir)
            .arg("/srcdir")
            .arg("--bind")
            .arg(&self.builddir)
            .arg("/builddir")
            .arg("--bind")
            .arg(&self.pkgdir)
            .arg("/pkgdir")
            .arg("--setenv")
            .arg("name")
            .arg(&self.name)
            .arg("--setenv")
            .arg("version")
            .arg(&self.version)
            .arg("--setenv")
            .arg("release")
            .arg(&self.release)
            .arg("--setenv")
            .arg("ARCH")
            .arg(&self.arch)
            .arg("--setenv")
            .arg("srcdir")
            .arg(&self.srcdir)
            .arg("--setenv")
            .arg("builddir")
            .arg(&self.builddir)
            .arg("--setenv")
            .arg("pkgdir")
            .arg(&self.pkgdir)
            .arg("--unshare-net")
            .args(&["bash", "-euo", "pipefail", "-c", &phase])
            .output()?;
        if !cmd.status.success() {
            return Err(super::error::SandboxError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                "non-zero exit status",
            )));
        }

        Ok(())
    }
}
