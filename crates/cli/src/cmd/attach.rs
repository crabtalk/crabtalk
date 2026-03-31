//! Attach to an agent via the interactive chat REPL.

use crate::repl::{ChatRepl, runner::Runner};
use anyhow::Result;
use clap::Args;
use dialoguer::{Input, Password, Select, theme::ColorfulTheme};
use std::path::Path;
use toml_edit::{Array, DocumentMut, Item, Table, value};
use wcore::config::PROVIDER_PRESETS;

/// Attach to an agent and start an interactive chat REPL.
#[derive(Args, Debug)]
pub struct Attach {
    /// Connect via TCP instead of Unix domain socket.
    /// Reads the port from ~/.crabtalk/run/crabtalk.port.
    #[arg(long, default_missing_value = "true", num_args = 0)]
    pub tcp: bool,
    /// Agent to attach to.
    #[arg(long, default_value = "crab")]
    pub agent: String,
}

impl Attach {
    /// Enter the interactive REPL with the given runner and agent.
    pub async fn run(self, runner: Runner) -> Result<()> {
        let mut repl = ChatRepl::new(runner, self.agent)?;
        repl.run().await
    }
}

/// Interactive provider setup for first-time daemon start.
pub(crate) fn setup_provider(config_path: &Path) -> Result<()> {
    let theme = ColorfulTheme::default();
    let preset_names: Vec<&str> = PROVIDER_PRESETS.iter().map(|p| p.name).collect();

    println!("\nNo providers configured. Let's set one up.\n");
    let idx = Select::with_theme(&theme)
        .with_prompt("Select a provider")
        .items(&preset_names)
        .default(0)
        .interact()?;
    let preset = &PROVIDER_PRESETS[idx];

    let provider_name = preset.name.to_string();

    // 2. API key — skipped for ollama.
    let api_key = if preset.name != "ollama" {
        let key: String = Password::with_theme(&theme)
            .with_prompt("API key")
            .interact()?;
        if key.is_empty() {
            anyhow::bail!("API key is required for {}", preset.name);
        }
        Some(key)
    } else {
        None
    };

    // 3. Base URL — fixed (read-only) or editable (with default if available).
    let base_url = if !preset.fixed_base_url.is_empty() {
        println!("  Base URL: {} (fixed)", preset.fixed_base_url);
        preset.base_url.to_string()
    } else {
        let url: String = if !preset.base_url.is_empty() {
            Input::with_theme(&theme)
                .with_prompt("Base URL")
                .default(preset.base_url.to_string())
                .interact_text()?
        } else {
            Input::with_theme(&theme)
                .with_prompt("Base URL")
                .interact_text()?
        };
        if url.trim().is_empty() {
            anyhow::bail!("base URL is required for {}", provider_name);
        }
        url
    };

    let model: String = if !preset.default_model.is_empty() {
        Input::with_theme(&theme)
            .with_prompt("Model name")
            .default(preset.default_model.to_string())
            .interact_text()?
    } else {
        let m: String = Input::with_theme(&theme)
            .with_prompt("Model name")
            .interact_text()?;
        if m.is_empty() {
            anyhow::bail!("model name is required");
        }
        m
    };

    // Write to config.toml.
    let content = std::fs::read_to_string(config_path)?;
    let mut doc: DocumentMut = content.parse()?;

    if !doc.contains_key("system") {
        doc.insert("system", Item::Table(Table::new()));
    }
    if doc["system"]
        .as_table()
        .and_then(|s| s.get("crab"))
        .is_none()
    {
        doc["system"]["crab"] = Item::Table(Table::new());
    }
    doc["system"]["crab"]["model"] = value(&model);

    if !doc.contains_key("provider") {
        doc.insert("provider", Item::Table(Table::new()));
    }
    let provider_table = doc["provider"].as_table_mut().unwrap();
    let mut entry = Table::new();
    if let Some(ref key) = api_key {
        entry.insert("api_key", value(key.as_str()));
    }
    if !base_url.is_empty() {
        entry.insert("base_url", value(base_url.as_str()));
    }
    let kind_str = serde_json::to_value(preset.kind)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "openai".to_string());
    entry.insert("kind", value(&kind_str));
    let mut models = Array::new();
    models.push(model.as_str());
    entry.insert("models", value(models));
    provider_table.insert(&provider_name, Item::Table(entry));

    std::fs::write(config_path, doc.to_string())?;
    println!("\nSaved to {}\n", config_path.display());
    Ok(())
}
