use serde::Deserialize;

#[derive(Deserialize)]
pub struct Package {
    pub url: String,
    pub download_method: String,
}
