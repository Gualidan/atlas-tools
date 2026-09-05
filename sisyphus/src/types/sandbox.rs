use crate::types::error::SandboxError;

pub trait Sandbox {
    fn run_phase(&self, phase: String) -> Result<(), SandboxError>;
}
