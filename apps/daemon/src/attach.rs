//! Provider setup utility for first-time configuration.

use anyhow::Result;
use dialoguer::{Password, Select, theme::ColorfulTheme};
use std::io::{self, BufRead, Write};
use std::path::Path;
use toml_edit::{Array, DocumentMut, Item, Table, value};
use wcore::config::PROVIDER_PRESETS;

/// Line-oriented prompt — robust to bracketed paste, unlike dialoguer's
/// char-by-char `Input`.
fn prompt_line(prompt: &str, default: Option<&str>) -> Result<String> {
    match default {
        Some(d) => print!("{prompt} [{d}]: "),
        None => print!("{prompt}: "),
    }
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    let trimmed = line.trim();
    Ok(if trimmed.is_empty() {
        default.unwrap_or("").to_string()
    } else {
        trimmed.to_string()
    })
}

/// Interactive provider setup for first-time daemon start.
pub fn setup_provider(config_path: &Path) -> Result<()> {
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

    // Only ollama is truly auth-free at runtime — the openai dispatch path
    // (used by "custom") always sends a Bearer header, so a blank api_key
    // results in `Authorization: Bearer ` which most endpoints reject.
    let api_key_optional = preset.name == "ollama";
    let api_key = {
        let key: String = Password::with_theme(&theme)
            .with_prompt(if api_key_optional {
                "API key (optional)"
            } else {
                "API key"
            })
            .allow_empty_password(api_key_optional)
            .interact()?;
        if key.is_empty() {
            if !api_key_optional {
                anyhow::bail!("API key is required for {}", preset.name);
            }
            None
        } else {
            Some(key)
        }
    };

    let base_url = if !preset.fixed_base_url.is_empty() {
        println!("  Base URL: {} (fixed)", preset.fixed_base_url);
        preset.base_url.to_string()
    } else {
        let default = (!preset.base_url.is_empty()).then_some(preset.base_url);
        let url = prompt_line("Base URL", default)?;
        if url.is_empty() {
            anyhow::bail!("base URL is required for {}", provider_name);
        }
        url
    };

    let default_model = (!preset.default_model.is_empty()).then_some(preset.default_model);
    let model = prompt_line("Model name", default_model)?;
    if model.is_empty() {
        anyhow::bail!("model name is required");
    }

    let content = std::fs::read_to_string(config_path)?;
    let mut doc: DocumentMut = content.parse()?;

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
