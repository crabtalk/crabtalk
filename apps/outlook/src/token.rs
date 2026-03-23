use crate::error::Error;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
}

impl Token {
    pub fn is_expired(&self) -> bool {
        let now = chrono::Utc::now().timestamp();
        // 5-minute buffer
        self.expires_at <= now + 300
    }

    pub fn load(path: &Path) -> Result<Self, Error> {
        let data = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&data)?)
    }

    pub fn save(&self, path: &Path) -> Result<(), Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
    }
}

pub fn token_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".crabtalk")
        .join("outlook")
        .join("token.json")
}
