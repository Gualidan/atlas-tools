use anyhow::Result;

pub trait Sandbox {
    /// Run phase commands in the bubblewrap sandbox
    fn run_phase(&self, phase: String) -> Result<()>;
}
