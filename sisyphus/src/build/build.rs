use crate::build::dependencies::resolve;
use crate::types::error::BuildError;
use crate::types::package::Package;
use crate::types::runtime_config::RuntimeConfig;

pub fn setup(runtime_config: RuntimeConfig) -> Result<(), BuildError> {
    
    Ok(())
}

pub fn build(package: Package, config: RuntimeConfig) -> Result<(), BuildError> {
    resolve(config, package)?;
    

    Ok(())
}
