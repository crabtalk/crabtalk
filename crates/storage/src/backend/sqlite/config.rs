//! Install config and first-run scaffolding.

use crate::backend::sqlite::SqliteStorage;
use anyhow::Result;
use schema::{AgentConfig, Config};

impl SqliteStorage {
    pub(super) async fn load_config(&self) -> Result<Config> {
        let toml: Option<String> = sqlx::query_scalar("SELECT toml FROM config WHERE id = 1")
            .fetch_optional(&self.pool)
            .await?;
        match toml {
            Some(toml) => Config::from_toml(&toml),
            None => Ok(Config::default()),
        }
    }

    pub(super) async fn save_config(&self, config: &Config) -> Result<()> {
        sqlx::query(
            "INSERT INTO config (id, toml) VALUES (1, ?)
             ON CONFLICT(id) DO UPDATE SET toml = excluded.toml",
        )
        .bind(toml::to_string_pretty(config)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Seed the built-in `crab` agent if the store holds none. The
    /// directory layout is not this backend's concern — a tenant is one
    /// database file — so unlike `FsStorage` this creates no directories.
    pub(super) async fn scaffold(&self, default_model: &str) -> Result<()> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agents")
            .fetch_one(&self.pool)
            .await?;
        if count > 0 {
            return Ok(());
        }
        self.upsert_agent(&AgentConfig::crab(default_model)).await
    }
}
