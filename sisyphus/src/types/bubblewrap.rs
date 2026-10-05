use which::which;

use crate::types::sandbox::Sandbox;
use common::types::package::Architectures;
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
        let mut cmd = Command::new("bwrap")
            .arg("--ro-bind")
            .arg("/usr")
            .arg("/usr")
            .arg("--symlink")
            .arg("/usr/bin")
            .arg("/bin")
            .arg("--symlink")
            .arg("/usr/lib")
            .arg("/lib")
            .arg("--symlink")
            .arg("/usr/lib64")
            .arg("/lib64")
            .arg("--symlink")
            .arg("/usr/sbin")
            .arg("/sbin")
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
            .arg("/srcdir")
            .arg("--setenv")
            .arg("builddir")
            .arg("/builddir")
            .arg("--setenv")
            .arg("pkgdir")
            .arg("/pkgdir")
            .arg("--unshare-net")
            .args(&["bash", "-euo", "pipefail", "-c", &phase])
            .spawn()?;

        let status = cmd.wait()?;

        if !status.success() {
            return Err(super::error::SandboxError::BwrapExitError(status));
        }

        Ok(())
    }
}
