use anyhow::Result;

pub trait Sandbox {
    fn run_phase(&self, phase: String) -> Result<()>;
}
