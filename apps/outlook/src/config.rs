use crate::error::Error;
use serde::Deserialize;
use std::path::PathBuf;
use wcore::paths::CONFIG_DIR;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub client_id: String,
    #[serde(default = "default_tenant")]
    pub tenant_id: String,
    #[serde(default = "default_port")]
    pub redirect_port: u16,
}

fn default_tenant() -> String {
    "common".to_owned()
}

fn default_port() -> u16 {
    8400
}

impl Config {
    pub fn load() -> Result<Self, Error> {
        let path = config_path();
        if !path.exists() {
            return Err(Error::Config(format!(
                "config not found at {}. Create it with client_id and tenant_id.",
                path.display()
            )));
        }
        let data = std::fs::read_to_string(&path)?;
        Ok(toml::from_str(&data)?)
    }
}

pub fn config_path() -> PathBuf {
    CONFIG_DIR.join("config").join("outlook.toml")
}
