use anyhow::Result;

pub trait Sandbox {
    /// Run phase commands in the bubblewrap sandbox.
    ///
    /// # Errors
    ///
    /// Returns an error if the sandbox cannot start or the phase exits
    /// unsuccessfully.
    fn run_phase(&self, phase: String) -> Result<()>;
}
