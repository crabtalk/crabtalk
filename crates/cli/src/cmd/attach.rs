//! Attach to an agent via the interactive chat REPL.

use crate::cmd::auth::PRESETS;
use crate::repl::{ChatRepl, runner::Runner};
use anyhow::Result;
use clap::Args;
use compact_str::CompactString;
use dialoguer::{Input, Select, theme::ColorfulTheme};
use std::path::Path;
use toml_edit::{Array, DocumentMut, Item, Table, value};
use wcore::paths::CONFIG_DIR;

/// Attach to an agent and start an interactive chat REPL.
#[derive(Args, Debug)]
pub struct Attach {
    /// Connect via TCP instead of Unix domain socket.
    /// Reads the port from ~/.openwalrus/walrus.tcp.
    #[arg(long, default_missing_value = "true", num_args = 0)]
    pub tcp: bool,
}

impl Attach {
    /// Enter the interactive REPL with the given runner and agent.
    pub async fn run(self, runner: Runner, agent: CompactString) -> Result<()> {
        let mut repl = ChatRepl::new(runner, agent)?;
        repl.run().await
    }
}

/// Check if providers are configured; prompt and reload the daemon if empty.
pub async fn ensure_providers(socket_path: &Path) -> Result<()> {
    let config_path = CONFIG_DIR.join("walrus.toml");
    if !config_path.exists() {
        return Ok(());
    }

    let config = ::daemon::DaemonConfig::load(&config_path)?;
    if config.provider.is_empty() {
        setup_provider(&config_path)?;
        if let Ok(mut runner) = Runner::connect(socket_path).await {
            let _ = runner.reload().await;
        }
    }
    Ok(())
}

/// Interactive provider setup for first-time daemon start.
pub(crate) fn setup_provider(config_path: &Path) -> Result<()> {
    let theme = ColorfulTheme::default();
    let preset_names: Vec<&str> = PRESETS.iter().map(|p| p.name).collect();

    println!("\nNo providers configured. Let's set one up.\n");
    let idx = Select::with_theme(&theme)
        .with_prompt("Select a provider")
        .items(&preset_names)
        .default(0)
        .interact()?;
    let preset = &PRESETS[idx];

    let api_key = if preset.name != "ollama" {
        let key: String = Input::with_theme(&theme)
            .with_prompt("API key")
            .interact_text()?;
        if key.is_empty() {
            anyhow::bail!("API key is required for {}", preset.name);
        }
        Some(key)
    } else {
        None
    };

    let base_url = if preset.name == "custom" {
        Input::with_theme(&theme)
            .with_prompt("Base URL")
            .interact_text()?
    } else {
        preset.base_url.to_string()
    };

    let default_model = default_model_for(preset.name);
    let model: String = Input::with_theme(&theme)
        .with_prompt("Model name")
        .default(default_model.to_string())
        .interact_text()?;

    // Write to walrus.toml.
    let content = std::fs::read_to_string(config_path)?;
    let mut doc: DocumentMut = content.parse()?;

    if !doc.contains_key("walrus") {
        doc.insert("walrus", Item::Table(Table::new()));
    }
    doc["walrus"]["model"] = value(&model);

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
    entry.insert("standard", value(preset.standard));
    let mut models = Array::new();
    models.push(model.as_str());
    entry.insert("models", value(models));
    provider_table.insert(preset.name, Item::Table(entry));

    std::fs::write(config_path, doc.to_string())?;
    println!("\nSaved to {}\n", config_path.display());
    Ok(())
}

fn default_model_for(provider: &str) -> &str {
    match provider {
        "anthropic" => "claude-sonnet-4-5-20250514",
        "openai" => "gpt-4o",
        "deepseek" => "deepseek-chat",
        "ollama" => "llama3",
        _ => "default",
    }
}
