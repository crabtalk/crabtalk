//! File-based credential storage for MCP OAuth tokens.

use rmcp::transport::{AuthError, CredentialStore, StoredCredentials};
use std::path::PathBuf;
use wcore::paths::TOKENS_DIR;

/// Filesystem-backed credential store. One JSON file per MCP server.
pub struct FileCredentialStore {
    path: PathBuf,
}

impl FileCredentialStore {
    /// Build a store for the given MCP server name.
    pub fn for_server(name: &str) -> Self {
        Self {
            path: TOKENS_DIR.join(format!("{name}.json")),
        }
    }
}

#[async_trait::async_trait]
impl CredentialStore for FileCredentialStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        let bytes = match std::fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(AuthError::InternalError(format!(
                    "failed to read token file {}: {e}",
                    self.path.display()
                )));
            }
        };
        let creds: StoredCredentials = serde_json::from_slice(&bytes).map_err(|e| {
            AuthError::InternalError(format!("invalid token file {}: {e}", self.path.display()))
        })?;
        Ok(Some(creds))
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                AuthError::InternalError(format!(
                    "failed to create token dir {}: {e}",
                    parent.display()
                ))
            })?;
        }
        let json = serde_json::to_string_pretty(&credentials).map_err(|e| {
            AuthError::InternalError(format!("failed to serialize credentials: {e}"))
        })?;
        std::fs::write(&self.path, json).map_err(|e| {
            AuthError::InternalError(format!(
                "failed to write token file {}: {e}",
                self.path.display()
            ))
        })?;
        Ok(())
    }

    async fn clear(&self) -> Result<(), AuthError> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(AuthError::InternalError(format!(
                "failed to remove token file {}: {e}",
                self.path.display()
            ))),
        }
    }
}
