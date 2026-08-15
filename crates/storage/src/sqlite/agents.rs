//! Agent persistence — config stored whole, name and id as columns.

use crate::sqlite::SqliteStorage;
use anyhow::Result;
use sqlx::Row;
use wcore::{AgentConfig, AgentId, storage::validate_table_name};

impl SqliteStorage {
    pub(super) async fn list_agents(&self) -> Result<Vec<AgentConfig>> {
        let rows = sqlx::query("SELECT name, config_json, system_prompt FROM agents ORDER BY name")
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            match hydrate(&row) {
                Ok(cfg) => out.push(cfg),
                // One unreadable row shouldn't cost the caller every
                // other agent — the daemon lists agents to run them.
                Err(e) => tracing::warn!("skipping unreadable agent row: {e}"),
            }
        }
        Ok(out)
    }

    pub(super) async fn load_agent(&self, id: &AgentId) -> Result<Option<AgentConfig>> {
        if id.is_nil() {
            return Ok(None);
        }
        let row = sqlx::query("SELECT name, config_json, system_prompt FROM agents WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(hydrate).transpose()
    }

    pub(super) async fn load_agent_by_name(&self, name: &str) -> Result<Option<AgentConfig>> {
        let row = sqlx::query("SELECT name, config_json, system_prompt FROM agents WHERE name = ?")
            .bind(name)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(hydrate).transpose()
    }

    pub(super) async fn upsert_agent(&self, config: &AgentConfig, prompt: &str) -> Result<()> {
        if config.id.is_nil() {
            anyhow::bail!("cannot upsert agent with nil ID");
        }
        if config.name.is_empty() {
            anyhow::bail!("cannot upsert agent with empty name");
        }
        validate_table_name("agent", &config.name)?;
        // The prompt is a column, so it isn't duplicated inside the blob.
        let mut stored = config.clone();
        stored.system_prompt = String::new();
        sqlx::query(
            "INSERT INTO agents (id, name, config_json, system_prompt) VALUES (?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                 name = excluded.name,
                 config_json = excluded.config_json,
                 system_prompt = excluded.system_prompt",
        )
        .bind(config.id.to_string())
        .bind(&config.name)
        .bind(serde_json::to_string(&stored)?)
        .bind(prompt)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub(super) async fn delete_agent(&self, id: &AgentId) -> Result<bool> {
        let done = sqlx::query("DELETE FROM agents WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() > 0)
    }

    pub(super) async fn rename_agent(&self, id: &AgentId, new_name: &str) -> Result<bool> {
        validate_table_name("agent", new_name)?;
        let current: Option<String> = sqlx::query_scalar("SELECT name FROM agents WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        let Some(current) = current else {
            return Ok(false);
        };
        if current == new_name {
            return Ok(true);
        }
        // Report the collision rather than letting the UNIQUE index
        // surface as an opaque driver error.
        let taken: Option<String> = sqlx::query_scalar("SELECT id FROM agents WHERE name = ?")
            .bind(new_name)
            .fetch_optional(&self.pool)
            .await?;
        if taken.is_some() {
            anyhow::bail!("agent '{new_name}' already exists");
        }
        sqlx::query("UPDATE agents SET name = ? WHERE id = ?")
            .bind(new_name)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(true)
    }
}

/// Rebuild an `AgentConfig` from its row: the blob, plus the two fields
/// stored as columns.
fn hydrate(row: &sqlx::sqlite::SqliteRow) -> Result<AgentConfig> {
    let json: String = row.try_get("config_json")?;
    let mut cfg: AgentConfig = serde_json::from_str(&json)?;
    cfg.name = row.try_get("name")?;
    cfg.system_prompt = row.try_get("system_prompt")?;
    Ok(cfg)
}
