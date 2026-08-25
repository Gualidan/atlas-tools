use serde::Deserialize;

#[derive(Deserialize)]
pub struct Package {
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub url: String,
    pub download_method: String,
}
