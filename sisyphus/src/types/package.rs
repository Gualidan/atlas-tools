use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct Package {
    pub name: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub url: String,
    pub download_method: String,
    pub checksum: Option<String>,
}
