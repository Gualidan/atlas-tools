use crate::{signing::signing::keygen, types::error::KeygenError};
use dirs::config_dir;
use std::{fs::File, io::Write, path::PathBuf};

pub fn run_keygen(path: &Option<PathBuf>) -> Result<(), KeygenError> {
    let config_dir = config_dir()
        .ok_or(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Recipe directory not found",
        ))?
        .join("sisyphus");

    let keypair_path = if let Some(path) = path {
        keygen(&Some(path.clone()))?
    } else {
        keygen(&None)?
    };

    let yaml_input = format!(
        r#"
        priv_key_path: {}
        pub_key_path: {}
        "#,
        keypair_path.0.display(),
        keypair_path.1.display()
    );

    File::create(config_dir.join("sisyphus").with_extension("yaml"))?
        .write_all(serde_saphyr::from_str(yaml_input.as_str())?)?;

    Ok(())
}
