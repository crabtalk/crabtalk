//! Interactive TUI for configuring LLM providers, models, and channel tokens.

use crate::tui::{self, border_dim, border_focused, char_to_byte, handle_text_input, mask_token};
use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Tabs},
};
use toml_edit::{Array, DocumentMut, Item, Table, value};

/// Configure providers, models, and channel tokens interactively.
#[derive(clap::Args, Debug)]
pub struct Auth;

impl Auth {
    pub fn run(self) -> Result<()> {
        tui::run_app(" Walrus Auth ", AuthState::load, render, handle_key)
    }
}

// ── Presets ──────────────────────────────────────────────────────────

pub(crate) struct Preset {
    pub(crate) name: &'static str,
    pub(crate) base_url: &'static str,
    pub(crate) standard: &'static str,
}

pub(crate) const PRESETS: &[Preset] = &[
    Preset {
        name: "anthropic",
        base_url: "https://api.anthropic.com/v1/messages",
        standard: "anthropic",
    },
    Preset {
        name: "openai",
        base_url: "https://api.openai.com/v1/chat/completions",
        standard: "openai",
    },
    Preset {
        name: "deepseek",
        base_url: "https://api.deepseek.com/v1/chat/completions",
        standard: "openai",
    },
    Preset {
        name: "ollama",
        base_url: "http://localhost:11434/v1/chat/completions",
        standard: "openai",
    },
    Preset {
        name: "custom",
        base_url: "",
        standard: "openai",
    },
];

// ── Tabs ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Providers,
    Channels,
    Mcps,
}

const TAB_TITLES: &[&str] = &["Providers", "Channels", "MCPs"];

// ── Tree items (providers tab) ───────────────────────────────────────

#[derive(Clone)]
enum TreeItem {
    Provider(usize),
    Model(usize, usize),
}

struct ProviderData {
    name: String,
    api_key: String,
    base_url: String,
    standard: String,
    models: Vec<String>,
}

const PROVIDER_FIELDS: &[&str] = &["api_key", "base_url", "standard"];
const CHANNEL_NAMES: &[&str] = &["Telegram", "Discord"];

struct McpData {
    name: String,
    env: Vec<(String, String)>,
}

// ── Focus states ─────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    List,
    Editing,
    PresetSelector,
    AddModel,
}

// ── State ────────────────────────────────────────────────────────────

struct AuthState {
    tab: Tab,
    focus: Focus,
    // Providers.
    providers: Vec<ProviderData>,
    active_model: String,
    selected: usize,
    editing_field: Option<usize>,
    cursor: usize,
    edit_buf: String,
    preset_idx: usize,
    // Channels.
    channel_selected: usize,
    channel_tokens: [String; 2],
    // MCPs.
    mcps: Vec<McpData>,
    mcp_selected: usize,
    mcp_env_selected: usize,
    // Shared.
    status: String,
}

impl AuthState {
    fn load() -> Result<Self> {
        let config_path = wcore::paths::CONFIG_DIR.join("walrus.toml");
        let mut providers = Vec::new();
        let mut active_model = String::new();
        let mut channel_tokens = [String::new(), String::new()];
        let mut mcps = Vec::new();

        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)
                .with_context(|| format!("cannot read {}", config_path.display()))?;
            let doc: DocumentMut = content
                .parse()
                .with_context(|| format!("invalid TOML in {}", config_path.display()))?;

            if let Some(walrus) = doc.get("walrus").and_then(|w| w.as_table())
                && let Some(m) = walrus.get("model").and_then(|v| v.as_str())
            {
                active_model = m.to_string();
            }

            if let Some(provider_table) = doc.get("provider").and_then(|p| p.as_table()) {
                for (name, item) in provider_table.iter() {
                    let Some(tbl) = item.as_table() else {
                        continue;
                    };
                    let api_key = tbl
                        .get("api_key")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let base_url = tbl
                        .get("base_url")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let standard = tbl
                        .get("standard")
                        .and_then(|v| v.as_str())
                        .unwrap_or("openai")
                        .to_string();
                    let mut models = Vec::new();
                    if let Some(arr) = tbl.get("models").and_then(|v| v.as_array()) {
                        for m in arr.iter() {
                            if let Some(s) = m.as_str() {
                                models.push(s.to_string());
                            }
                        }
                    }
                    providers.push(ProviderData {
                        name: name.to_string(),
                        api_key,
                        base_url,
                        standard,
                        models,
                    });
                }
            }

            if let Some(channel) = doc.get("channel").and_then(|c| c.as_table()) {
                if let Some(tg) = channel.get("telegram").and_then(|t| t.as_table()) {
                    channel_tokens[0] = tg
                        .get("token")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                }
                if let Some(dc) = channel.get("discord").and_then(|t| t.as_table()) {
                    channel_tokens[1] = dc
                        .get("token")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                }
            }

            if let Some(mcps_table) = doc.get("mcps").and_then(|m| m.as_table()) {
                for (name, item) in mcps_table.iter() {
                    let Some(tbl) = item.as_table() else {
                        continue;
                    };
                    let mut env = Vec::new();
                    if let Some(env_tbl) = tbl.get("env").and_then(|e| e.as_table()) {
                        for (k, v) in env_tbl.iter() {
                            let val = v.as_str().unwrap_or("").to_string();
                            env.push((k.to_string(), val));
                        }
                    }
                    mcps.push(McpData {
                        name: name.to_string(),
                        env,
                    });
                }
            }
        }

        Ok(Self {
            tab: Tab::Providers,
            focus: Focus::List,
            providers,
            active_model,
            selected: 0,
            editing_field: None,
            cursor: 0,
            edit_buf: String::new(),
            preset_idx: 0,
            channel_selected: 0,
            channel_tokens,
            mcps,
            mcp_selected: 0,
            mcp_env_selected: 0,
            status: String::from("Ready"),
        })
    }

    fn save(&mut self) -> Result<()> {
        let config_path = wcore::paths::CONFIG_DIR.join("walrus.toml");
        std::fs::create_dir_all(&*wcore::paths::CONFIG_DIR)
            .with_context(|| format!("cannot create {}", wcore::paths::CONFIG_DIR.display()))?;

        let content = if config_path.exists() {
            std::fs::read_to_string(&config_path)
                .with_context(|| format!("cannot read {}", config_path.display()))?
        } else {
            String::new()
        };

        let mut doc: DocumentMut = content
            .parse()
            .with_context(|| format!("invalid TOML in {}", config_path.display()))?;

        // [walrus].model
        if !self.active_model.is_empty() {
            if doc.get("walrus").is_none() {
                doc.insert("walrus", Item::Table(Table::new()));
            }
            if let Some(walrus) = doc.get_mut("walrus").and_then(|w| w.as_table_mut()) {
                walrus.insert("model", value(&self.active_model));
            }
        }

        // [provider.*]
        doc.remove("provider");
        if !self.providers.is_empty() {
            let mut provider_table = Table::new();
            for p in &self.providers {
                let mut tbl = Table::new();
                if !p.api_key.is_empty() {
                    tbl.insert("api_key", value(&p.api_key));
                }
                if !p.base_url.is_empty() {
                    tbl.insert("base_url", value(&p.base_url));
                }
                tbl.insert("standard", value(&p.standard));
                if !p.models.is_empty() {
                    let mut arr = Array::new();
                    for m in &p.models {
                        arr.push(m.as_str());
                    }
                    tbl.insert("models", Item::Value(arr.into()));
                }
                provider_table.insert(&p.name, Item::Table(tbl));
            }
            doc.insert("provider", Item::Table(provider_table));
        }

        // [channel.*]
        doc.remove("channel");
        let mut channel_table = Table::new();
        if !self.channel_tokens[0].is_empty() {
            let mut tg = Table::new();
            tg.insert("token", value(&self.channel_tokens[0]));
            channel_table.insert("telegram", Item::Table(tg));
        }
        if !self.channel_tokens[1].is_empty() {
            let mut dc = Table::new();
            dc.insert("token", value(&self.channel_tokens[1]));
            channel_table.insert("discord", Item::Table(dc));
        }
        if !channel_table.is_empty() {
            doc.insert("channel", Item::Table(channel_table));
        }

        // [mcps.*.env] — surgical update, only touch env values.
        for mcp in &self.mcps {
            if let Some(server) = doc
                .get_mut("mcps")
                .and_then(|m| m.as_table_mut())
                .and_then(|t| t.get_mut(&mcp.name))
                .and_then(|s| s.as_table_mut())
            {
                let env_table = server
                    .entry("env")
                    .or_insert(Item::Table(Table::new()))
                    .as_table_mut();
                if let Some(env_table) = env_table {
                    for (k, v) in &mcp.env {
                        env_table.insert(k, value(v));
                    }
                }
            }
        }

        std::fs::write(&config_path, doc.to_string())
            .with_context(|| format!("failed to write {}", config_path.display()))?;

        self.status = String::from("Saved!");
        Ok(())
    }

    // ── Provider tree helpers ────────────────────────────────────────

    fn tree_items(&self) -> Vec<TreeItem> {
        let mut items = Vec::new();
        for (pi, p) in self.providers.iter().enumerate() {
            items.push(TreeItem::Provider(pi));
            for (mi, _) in p.models.iter().enumerate() {
                items.push(TreeItem::Model(pi, mi));
            }
        }
        items
    }

    fn tree_len(&self) -> usize {
        self.providers
            .iter()
            .map(|p| 1 + p.models.len())
            .sum::<usize>()
    }

    fn selected_item(&self) -> Option<TreeItem> {
        self.tree_items().get(self.selected).cloned()
    }

    fn provider_field_value(&self, pi: usize, field: usize) -> &str {
        let p = &self.providers[pi];
        match field {
            0 => &p.api_key,
            1 => &p.base_url,
            2 => &p.standard,
            _ => "",
        }
    }

    fn set_provider_field(&mut self, pi: usize, field: usize, val: String) {
        let p = &mut self.providers[pi];
        match field {
            0 => p.api_key = val,
            1 => p.base_url = val,
            2 => p.standard = val,
            _ => {}
        }
    }

    fn add_preset(&mut self, preset: &Preset) {
        self.providers.push(ProviderData {
            name: preset.name.to_string(),
            api_key: String::new(),
            base_url: preset.base_url.to_string(),
            standard: preset.standard.to_string(),
            models: Vec::new(),
        });
        let new_idx = self.tree_len().saturating_sub(1);
        self.selected = new_idx;
    }
}

// ── Key handling ────────────────────────────────────────────────────

fn handle_key(
    key: crossterm::event::KeyEvent,
    state: &mut AuthState,
) -> Result<Option<Result<()>>> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
        if let Err(e) = state.save() {
            state.status = format!("Error: {e}");
        }
        return Ok(None);
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Ok(Some(Ok(())));
    }

    // Tab switching only in list focus.
    if key.code == KeyCode::Tab && state.focus == Focus::List {
        state.tab = match state.tab {
            Tab::Providers => Tab::Channels,
            Tab::Channels => Tab::Mcps,
            Tab::Mcps => Tab::Providers,
        };
        return Ok(None);
    }

    match state.tab {
        Tab::Providers => handle_providers_key(key, state),
        Tab::Channels => handle_channels_key(key, state),
        Tab::Mcps => handle_mcps_key(key, state),
    }
}

// ── Providers key handling ──────────────────────────────────────────

fn handle_providers_key(
    key: crossterm::event::KeyEvent,
    state: &mut AuthState,
) -> Result<Option<Result<()>>> {
    match state.focus {
        Focus::List => handle_provider_list(key, state),
        Focus::Editing => {
            handle_provider_editing(key, state);
            Ok(None)
        }
        Focus::PresetSelector => {
            handle_preset(key, state);
            Ok(None)
        }
        Focus::AddModel => {
            handle_add_model(key, state);
            Ok(None)
        }
    }
}

fn handle_provider_list(
    key: crossterm::event::KeyEvent,
    state: &mut AuthState,
) -> Result<Option<Result<()>>> {
    let tree_len = state.tree_len();
    match key.code {
        KeyCode::Char('q') => return Ok(Some(Ok(()))),
        KeyCode::Up | KeyCode::Char('k') => {
            state.selected = state.selected.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if tree_len > 0 && state.selected < tree_len - 1 {
                state.selected += 1;
            }
        }
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            if let Some(item) = state.selected_item() {
                match item {
                    TreeItem::Provider(pi) => {
                        state.editing_field = Some(0);
                        let val = state.provider_field_value(pi, 0).to_string();
                        state.cursor = val.chars().count();
                        state.edit_buf = val;
                        state.focus = Focus::Editing;
                    }
                    TreeItem::Model(pi, mi) => {
                        state.editing_field = None;
                        let val = state.providers[pi].models[mi].clone();
                        state.cursor = val.chars().count();
                        state.edit_buf = val;
                        state.focus = Focus::Editing;
                    }
                }
            }
        }
        KeyCode::Char('n') => {
            state.preset_idx = 0;
            state.focus = Focus::PresetSelector;
        }
        KeyCode::Char('m') => {
            if let Some(item) = state.selected_item() {
                let pi = match item {
                    TreeItem::Provider(pi) => pi,
                    TreeItem::Model(pi, _) => pi,
                };
                state.providers[pi].models.push(String::new());
                let items = state.tree_items();
                let new_mi = state.providers[pi].models.len() - 1;
                if let Some(idx) = items
                    .iter()
                    .position(|it| matches!(it, TreeItem::Model(p, m) if *p == pi && *m == new_mi))
                {
                    state.selected = idx;
                }
                state.editing_field = None;
                state.edit_buf = String::new();
                state.cursor = 0;
                state.focus = Focus::AddModel;
            } else {
                state.status = String::from("Add a provider first (n)");
            }
        }
        KeyCode::Char('a') => {
            if let Some(TreeItem::Model(pi, mi)) = state.selected_item() {
                state.active_model = state.providers[pi].models[mi].clone();
                state.status = format!("Active: {}", state.active_model);
            }
        }
        KeyCode::Char('d') | KeyCode::Delete => {
            if let Some(item) = state.selected_item() {
                match item {
                    TreeItem::Provider(pi) => {
                        state.providers.remove(pi);
                        let tree_len = state.tree_len();
                        if state.selected >= tree_len && tree_len > 0 {
                            state.selected = tree_len - 1;
                        }
                        if tree_len == 0 {
                            state.selected = 0;
                        }
                        state.status = String::from("Provider deleted");
                    }
                    TreeItem::Model(pi, mi) => {
                        let removed = state.providers[pi].models.remove(mi);
                        if state.active_model == removed {
                            state.active_model.clear();
                        }
                        let tree_len = state.tree_len();
                        if state.selected >= tree_len && tree_len > 0 {
                            state.selected = tree_len - 1;
                        }
                        state.status = String::from("Model deleted");
                    }
                }
            }
        }
        _ => {}
    }
    Ok(None)
}

fn handle_provider_editing(key: crossterm::event::KeyEvent, state: &mut AuthState) {
    match key.code {
        KeyCode::Esc => {
            state.focus = Focus::List;
        }
        KeyCode::Enter => {
            commit_provider_edit(state);
            if let Some(field) = state.editing_field
                && let Some(TreeItem::Provider(pi)) = state.selected_item()
            {
                if field == 2 {
                    toggle_standard(state, pi);
                    return;
                }
                let next = field + 1;
                if next < PROVIDER_FIELDS.len() {
                    state.editing_field = Some(next);
                    let val = state.provider_field_value(pi, next).to_string();
                    state.cursor = val.chars().count();
                    state.edit_buf = val;
                    return;
                }
            }
            state.focus = Focus::List;
        }
        KeyCode::Up => {
            if let Some(field) = state.editing_field
                && field > 0
            {
                commit_provider_edit(state);
                let new_field = field - 1;
                state.editing_field = Some(new_field);
                if let Some(TreeItem::Provider(pi)) = state.selected_item() {
                    let val = state.provider_field_value(pi, new_field).to_string();
                    state.cursor = val.chars().count();
                    state.edit_buf = val;
                }
            }
        }
        KeyCode::Down => {
            if let Some(field) = state.editing_field
                && field + 1 < PROVIDER_FIELDS.len()
            {
                commit_provider_edit(state);
                let new_field = field + 1;
                state.editing_field = Some(new_field);
                if let Some(TreeItem::Provider(pi)) = state.selected_item() {
                    let val = state.provider_field_value(pi, new_field).to_string();
                    state.cursor = val.chars().count();
                    state.edit_buf = val;
                }
            }
        }
        KeyCode::Tab => {
            if state.editing_field == Some(2)
                && let Some(TreeItem::Provider(pi)) = state.selected_item()
            {
                toggle_standard(state, pi);
            }
        }
        _ => handle_text_input(key.code, &mut state.edit_buf, &mut state.cursor),
    }
}

fn toggle_standard(state: &mut AuthState, pi: usize) {
    let p = &mut state.providers[pi];
    p.standard = if p.standard == "anthropic" {
        "openai".to_string()
    } else {
        "anthropic".to_string()
    };
    state.edit_buf = state.providers[pi].standard.clone();
    state.cursor = state.edit_buf.chars().count();
}

fn handle_preset(key: crossterm::event::KeyEvent, state: &mut AuthState) {
    match key.code {
        KeyCode::Esc => {
            state.focus = Focus::List;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.preset_idx = state.preset_idx.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if state.preset_idx < PRESETS.len() - 1 {
                state.preset_idx += 1;
            }
        }
        KeyCode::Enter => {
            state.add_preset(&PRESETS[state.preset_idx]);
            state.status = format!("Added provider: {}", PRESETS[state.preset_idx].name);
            state.focus = Focus::List;
        }
        _ => {}
    }
}

fn handle_add_model(key: crossterm::event::KeyEvent, state: &mut AuthState) {
    match key.code {
        KeyCode::Esc => {
            if let Some(item) = state.selected_item()
                && let TreeItem::Model(pi, mi) = item
                && state.providers[pi].models[mi].is_empty()
            {
                state.providers[pi].models.remove(mi);
                let tree_len = state.tree_len();
                if state.selected >= tree_len && tree_len > 0 {
                    state.selected = tree_len - 1;
                }
            }
            state.focus = Focus::List;
        }
        KeyCode::Enter => {
            if !state.edit_buf.is_empty() {
                commit_provider_edit(state);
            } else if let Some(TreeItem::Model(pi, mi)) = state.selected_item() {
                state.providers[pi].models.remove(mi);
                let tree_len = state.tree_len();
                if state.selected >= tree_len && tree_len > 0 {
                    state.selected = tree_len - 1;
                }
            }
            state.focus = Focus::List;
        }
        _ => handle_text_input(key.code, &mut state.edit_buf, &mut state.cursor),
    }
}

fn commit_provider_edit(state: &mut AuthState) {
    let val = state.edit_buf.clone();
    if let Some(item) = state.selected_item() {
        match item {
            TreeItem::Provider(pi) => {
                if let Some(field) = state.editing_field {
                    state.set_provider_field(pi, field, val);
                }
            }
            TreeItem::Model(pi, mi) => {
                state.providers[pi].models[mi] = val;
            }
        }
    }
}

// ── Channels key handling ───────────────────────────────────────────

fn handle_channels_key(
    key: crossterm::event::KeyEvent,
    state: &mut AuthState,
) -> Result<Option<Result<()>>> {
    match state.focus {
        Focus::List => {
            match key.code {
                KeyCode::Char('q') => return Ok(Some(Ok(()))),
                KeyCode::Up | KeyCode::Char('k') => {
                    state.channel_selected = state.channel_selected.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if state.channel_selected < CHANNEL_NAMES.len() - 1 {
                        state.channel_selected += 1;
                    }
                }
                KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                    state.focus = Focus::Editing;
                    let token = &state.channel_tokens[state.channel_selected];
                    state.cursor = token.chars().count();
                    state.edit_buf = token.clone();
                }
                KeyCode::Char('x') | KeyCode::Delete => {
                    state.channel_tokens[state.channel_selected].clear();
                }
                _ => {}
            }
            Ok(None)
        }
        Focus::Editing => {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    state.channel_tokens[state.channel_selected] = state.edit_buf.clone();
                    state.focus = Focus::List;
                }
                _ => handle_text_input(key.code, &mut state.edit_buf, &mut state.cursor),
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

// ── MCPs key handling ───────────────────────────────────────────────

fn handle_mcps_key(
    key: crossterm::event::KeyEvent,
    state: &mut AuthState,
) -> Result<Option<Result<()>>> {
    match state.focus {
        Focus::List => {
            match key.code {
                KeyCode::Char('q') => return Ok(Some(Ok(()))),
                KeyCode::Up | KeyCode::Char('k') => {
                    state.mcp_selected = state.mcp_selected.saturating_sub(1);
                    state.mcp_env_selected = 0;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if !state.mcps.is_empty() && state.mcp_selected < state.mcps.len() - 1 {
                        state.mcp_selected += 1;
                        state.mcp_env_selected = 0;
                    }
                }
                KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                    if let Some(mcp) = state.mcps.get(state.mcp_selected)
                        && !mcp.env.is_empty()
                    {
                        state.mcp_env_selected = 0;
                        let val = mcp.env[0].1.clone();
                        state.cursor = val.chars().count();
                        state.edit_buf = val;
                        state.focus = Focus::Editing;
                    }
                }
                _ => {}
            }
            Ok(None)
        }
        Focus::Editing => {
            match key.code {
                KeyCode::Esc => {
                    commit_mcp_edit(state);
                    state.focus = Focus::List;
                }
                KeyCode::Enter => {
                    commit_mcp_edit(state);
                    let env_len = state
                        .mcps
                        .get(state.mcp_selected)
                        .map(|m| m.env.len())
                        .unwrap_or(0);
                    if state.mcp_env_selected + 1 < env_len {
                        state.mcp_env_selected += 1;
                        let val = state.mcps[state.mcp_selected].env[state.mcp_env_selected]
                            .1
                            .clone();
                        state.cursor = val.chars().count();
                        state.edit_buf = val;
                    } else {
                        state.focus = Focus::List;
                    }
                }
                KeyCode::Up => {
                    if state.mcp_env_selected > 0 {
                        commit_mcp_edit(state);
                        state.mcp_env_selected -= 1;
                        let val = state.mcps[state.mcp_selected].env[state.mcp_env_selected]
                            .1
                            .clone();
                        state.cursor = val.chars().count();
                        state.edit_buf = val;
                    }
                }
                KeyCode::Down => {
                    let env_len = state
                        .mcps
                        .get(state.mcp_selected)
                        .map(|m| m.env.len())
                        .unwrap_or(0);
                    if state.mcp_env_selected + 1 < env_len {
                        commit_mcp_edit(state);
                        state.mcp_env_selected += 1;
                        let val = state.mcps[state.mcp_selected].env[state.mcp_env_selected]
                            .1
                            .clone();
                        state.cursor = val.chars().count();
                        state.edit_buf = val;
                    }
                }
                _ => handle_text_input(key.code, &mut state.edit_buf, &mut state.cursor),
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

fn commit_mcp_edit(state: &mut AuthState) {
    if let Some(mcp) = state.mcps.get_mut(state.mcp_selected)
        && let Some(entry) = mcp.env.get_mut(state.mcp_env_selected)
    {
        entry.1 = state.edit_buf.clone();
    }
}

// ── Rendering ───────────────────────────────────────────────────────

fn render(frame: &mut Frame, state: &AuthState) {
    let area = frame.area();

    let outer = Block::default()
        .title(" Walrus Auth ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let vert = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(2),
    ])
    .split(inner);

    // Tab bar.
    let tab_idx = match state.tab {
        Tab::Providers => 0,
        Tab::Channels => 1,
        Tab::Mcps => 2,
    };
    let tabs = Tabs::new(TAB_TITLES.iter().map(|t| Line::from(*t)))
        .select(tab_idx)
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .divider(" | ");
    frame.render_widget(tabs, vert[0]);

    match state.tab {
        Tab::Providers => render_providers(frame, state, vert[1]),
        Tab::Channels => render_channels(frame, state, vert[1]),
        Tab::Mcps => render_mcps(frame, state, vert[1]),
    }

    render_status(frame, state, vert[2]);
}

// ── Providers rendering ─────────────────────────────────────────────

fn render_providers(frame: &mut Frame, state: &AuthState, area: Rect) {
    if state.focus == Focus::PresetSelector {
        render_presets(frame, state, area);
        return;
    }

    let horiz =
        Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)]).split(area);
    render_provider_tree(frame, state, horiz[0]);
    render_provider_detail(frame, state, horiz[1]);
}

fn render_provider_tree(frame: &mut Frame, state: &AuthState, area: Rect) {
    let focused = state.tab == Tab::Providers && state.focus == Focus::List;
    let block = Block::default()
        .title(" Providers ")
        .borders(Borders::ALL)
        .border_style(if focused {
            border_focused()
        } else {
            border_dim()
        });

    let items = state.tree_items();
    let lines: Vec<Line> = items
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let is_selected = idx == state.selected;
            let marker = if is_selected { "> " } else { "  " };
            let style = if is_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            match item {
                TreeItem::Provider(pi) => {
                    let p = &state.providers[*pi];
                    let key_indicator = if p.api_key.is_empty() { "" } else { " [key]" };
                    let text = format!("{marker}{}{key_indicator}", p.name);
                    Line::from(Span::styled(text, style))
                }
                TreeItem::Model(pi, mi) => {
                    let model_name = &state.providers[*pi].models[*mi];
                    let active = if *model_name == state.active_model && !model_name.is_empty() {
                        " *"
                    } else {
                        ""
                    };
                    let text = format!("{marker}  {model_name}{active}");
                    Line::from(Span::styled(text, style))
                }
            }
        })
        .collect();

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_provider_detail(frame: &mut Frame, state: &AuthState, area: Rect) {
    let editing = matches!(state.focus, Focus::Editing | Focus::AddModel);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if editing {
            border_focused()
        } else {
            border_dim()
        });

    let Some(item) = state.selected_item() else {
        let block = block.title(" (empty) ");
        frame.render_widget(
            Paragraph::new("Press n to add a provider").block(block),
            area,
        );
        return;
    };

    match item {
        TreeItem::Provider(pi) => {
            let p = &state.providers[pi];
            let block = block.title(format!(" {} ", p.name));
            let inner = block.inner(area);
            frame.render_widget(block, area);

            let lines: Vec<Line> = PROVIDER_FIELDS
                .iter()
                .enumerate()
                .map(|(fi, label)| {
                    let is_editing = editing && state.editing_field == Some(fi);
                    let label_style = Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD);
                    let label_span = Span::styled(format!(" {:>10}: ", label), label_style);

                    let value = if is_editing {
                        let byte_pos = char_to_byte(&state.edit_buf, state.cursor);
                        let mut s = state.edit_buf.clone();
                        s.insert(byte_pos, '|');
                        Span::styled(s, Style::default().fg(Color::Green))
                    } else {
                        let raw = state.provider_field_value(pi, fi);
                        if fi == 0 && !raw.is_empty() {
                            Span::styled(mask_token(raw), Style::default().fg(Color::White))
                        } else if raw.is_empty() {
                            Span::styled(
                                "(empty)",
                                Style::default()
                                    .fg(Color::DarkGray)
                                    .add_modifier(Modifier::ITALIC),
                            )
                        } else {
                            Span::styled(raw, Style::default().fg(Color::White))
                        }
                    };

                    let indicator = if is_editing { " <" } else { "" };
                    Line::from(vec![
                        label_span,
                        value,
                        Span::styled(indicator, Style::default().fg(Color::Yellow)),
                    ])
                })
                .collect();

            frame.render_widget(Paragraph::new(lines), inner);
        }
        TreeItem::Model(pi, mi) => {
            let model_name = &state.providers[pi].models[mi];
            let provider_name = &state.providers[pi].name;
            let block = block.title(format!(" {provider_name} > model "));
            let inner = block.inner(area);
            frame.render_widget(block, area);

            let label_span = Span::styled(
                "      Name: ",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            );

            let line = if editing {
                let byte_pos = char_to_byte(&state.edit_buf, state.cursor);
                let mut s = state.edit_buf.clone();
                s.insert(byte_pos, '|');
                Line::from(vec![
                    label_span,
                    Span::styled(s, Style::default().fg(Color::Green)),
                ])
            } else if model_name.is_empty() {
                Line::from(vec![
                    label_span,
                    Span::styled(
                        "(enter model name)",
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ])
            } else {
                let active_marker = if *model_name == state.active_model {
                    "  [active]"
                } else {
                    ""
                };
                Line::from(vec![
                    label_span,
                    Span::styled(model_name, Style::default().fg(Color::White)),
                    Span::styled(active_marker, Style::default().fg(Color::Green)),
                ])
            };

            frame.render_widget(Paragraph::new(line), inner);
        }
    }
}

fn render_presets(frame: &mut Frame, state: &AuthState, area: Rect) {
    let block = Block::default()
        .title(" Select Provider Preset ")
        .borders(Borders::ALL)
        .border_style(border_focused());

    let lines: Vec<Line> = PRESETS
        .iter()
        .enumerate()
        .map(|(i, preset)| {
            let marker = if i == state.preset_idx { "> " } else { "  " };
            let style = if i == state.preset_idx {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let detail = if preset.base_url.is_empty() {
                String::new()
            } else {
                format!("  ({})", preset.base_url)
            };
            Line::from(vec![
                Span::styled(format!("{marker}{}", preset.name), style),
                Span::styled(
                    detail,
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                ),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

// ── Channels rendering ──────────────────────────────────────────────

fn render_channels(frame: &mut Frame, state: &AuthState, area: Rect) {
    let horiz =
        Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)]).split(area);
    render_channel_list(frame, state, horiz[0]);
    render_channel_detail(frame, state, horiz[1]);
}

fn render_channel_list(frame: &mut Frame, state: &AuthState, area: Rect) {
    let focused = state.tab == Tab::Channels && state.focus == Focus::List;
    let block = Block::default()
        .title(" Channels ")
        .borders(Borders::ALL)
        .border_style(if focused {
            border_focused()
        } else {
            border_dim()
        });

    let lines: Vec<Line> = CHANNEL_NAMES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let marker = if i == state.channel_selected {
                "> "
            } else {
                "  "
            };
            let configured = if state.channel_tokens[i].is_empty() {
                ""
            } else {
                " *"
            };
            let text = format!("{marker}{name}{configured}");
            let style = if i == state.channel_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            Line::from(Span::styled(text, style))
        })
        .collect();

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_channel_detail(frame: &mut Frame, state: &AuthState, area: Rect) {
    let name = CHANNEL_NAMES[state.channel_selected];
    let token = &state.channel_tokens[state.channel_selected];
    let editing = state.tab == Tab::Channels && state.focus == Focus::Editing;

    let hints = [
        "https://core.telegram.org/bots#botfather",
        "https://discord.com/developers/applications",
    ];
    let hint = hints[state.channel_selected];

    let block = Block::default()
        .title(format!(" {name} "))
        .borders(Borders::ALL)
        .border_style(if editing {
            border_focused()
        } else {
            border_dim()
        });
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let label_span = Span::styled(
        "     Token: ",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );

    let line = if editing {
        let byte_pos = char_to_byte(&state.edit_buf, state.cursor);
        let mut s = state.edit_buf.clone();
        s.insert(byte_pos, '|');
        Line::from(vec![
            label_span,
            Span::styled(s, Style::default().fg(Color::Green)),
        ])
    } else if token.is_empty() {
        Line::from(vec![
            label_span,
            Span::styled(
                hint,
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ),
        ])
    } else {
        Line::from(vec![
            label_span,
            Span::styled(mask_token(token), Style::default().fg(Color::White)),
        ])
    };

    frame.render_widget(Paragraph::new(line), inner);
}

// ── MCPs rendering ──────────────────────────────────────────────────

fn render_mcps(frame: &mut Frame, state: &AuthState, area: Rect) {
    let horiz =
        Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)]).split(area);
    render_mcp_list(frame, state, horiz[0]);
    render_mcp_detail(frame, state, horiz[1]);
}

fn render_mcp_list(frame: &mut Frame, state: &AuthState, area: Rect) {
    let focused = state.tab == Tab::Mcps && state.focus == Focus::List;
    let block = Block::default()
        .title(" MCP Servers ")
        .borders(Borders::ALL)
        .border_style(if focused {
            border_focused()
        } else {
            border_dim()
        });

    let lines: Vec<Line> = state
        .mcps
        .iter()
        .enumerate()
        .map(|(i, mcp)| {
            let marker = if i == state.mcp_selected { "> " } else { "  " };
            let has_env = if mcp.env.iter().any(|(_, v)| !v.is_empty()) {
                " *"
            } else {
                ""
            };
            let text = format!("{marker}{}{has_env}", mcp.name);
            let style = if i == state.mcp_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            Line::from(Span::styled(text, style))
        })
        .collect();

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_mcp_detail(frame: &mut Frame, state: &AuthState, area: Rect) {
    let editing = state.tab == Tab::Mcps && state.focus == Focus::Editing;

    let Some(mcp) = state.mcps.get(state.mcp_selected) else {
        let block = Block::default()
            .title(" (no MCP servers) ")
            .borders(Borders::ALL)
            .border_style(border_dim());
        frame.render_widget(
            Paragraph::new("Install a hub package with MCP servers").block(block),
            area,
        );
        return;
    };

    let block = Block::default()
        .title(format!(" {} ", mcp.name))
        .borders(Borders::ALL)
        .border_style(if editing {
            border_focused()
        } else {
            border_dim()
        });
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if mcp.env.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "  (no env vars)",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )),
            inner,
        );
        return;
    }

    let lines: Vec<Line> = mcp
        .env
        .iter()
        .enumerate()
        .map(|(ei, (key, val))| {
            let is_editing = editing && ei == state.mcp_env_selected;
            let label_style = Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD);
            let label_span = Span::styled(format!(" {key}: "), label_style);

            let value_span = if is_editing {
                let byte_pos = char_to_byte(&state.edit_buf, state.cursor);
                let mut s = state.edit_buf.clone();
                s.insert(byte_pos, '|');
                Span::styled(s, Style::default().fg(Color::Green))
            } else if val.is_empty() {
                Span::styled(
                    "(empty)",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                )
            } else {
                Span::styled(mask_token(val), Style::default().fg(Color::White))
            };

            let indicator = if is_editing { " <" } else { "" };
            Line::from(vec![
                label_span,
                value_span,
                Span::styled(indicator, Style::default().fg(Color::Yellow)),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

// ── Status bar ──────────────────────────────────────────────────────

fn render_status(frame: &mut Frame, state: &AuthState, area: Rect) {
    let help = match (state.tab, state.focus) {
        (_, Focus::PresetSelector) => Line::from(vec![
            Span::styled(" Enter ", Style::default().fg(Color::Cyan)),
            Span::raw("Select  "),
            Span::styled("Esc ", Style::default().fg(Color::Cyan)),
            Span::raw("Cancel  "),
            status_span(state),
        ]),
        (_, Focus::AddModel) => Line::from(vec![
            Span::styled(" Enter ", Style::default().fg(Color::Cyan)),
            Span::raw("Confirm  "),
            Span::styled("Esc ", Style::default().fg(Color::Cyan)),
            Span::raw("Cancel  "),
            status_span(state),
        ]),
        (Tab::Providers, Focus::Editing) => Line::from(vec![
            Span::styled(" Enter ", Style::default().fg(Color::Cyan)),
            Span::raw("Next  "),
            Span::styled("Up/Dn ", Style::default().fg(Color::Cyan)),
            Span::raw("Field  "),
            Span::styled("Esc ", Style::default().fg(Color::Cyan)),
            Span::raw("Back  "),
            Span::styled("Ctrl+S ", Style::default().fg(Color::Cyan)),
            Span::raw("Save  "),
            status_span(state),
        ]),
        (Tab::Channels, Focus::Editing) => Line::from(vec![
            Span::styled(" Enter ", Style::default().fg(Color::Cyan)),
            Span::raw("Save field  "),
            Span::styled("Esc ", Style::default().fg(Color::Cyan)),
            Span::raw("Back  "),
            Span::styled("Ctrl+S ", Style::default().fg(Color::Cyan)),
            Span::raw("Save  "),
            status_span(state),
        ]),
        (Tab::Providers, Focus::List) => Line::from(vec![
            Span::styled(" Tab ", Style::default().fg(Color::Cyan)),
            Span::raw("Switch  "),
            Span::styled("n ", Style::default().fg(Color::Cyan)),
            Span::raw("New  "),
            Span::styled("m ", Style::default().fg(Color::Cyan)),
            Span::raw("Model  "),
            Span::styled("a ", Style::default().fg(Color::Cyan)),
            Span::raw("Active  "),
            Span::styled("d ", Style::default().fg(Color::Cyan)),
            Span::raw("Delete  "),
            Span::styled("Enter ", Style::default().fg(Color::Cyan)),
            Span::raw("Edit  "),
            Span::styled("Ctrl+S ", Style::default().fg(Color::Cyan)),
            Span::raw("Save  "),
            Span::styled("q ", Style::default().fg(Color::Cyan)),
            Span::raw("Quit  "),
            status_span(state),
        ]),
        (Tab::Mcps, Focus::Editing) => Line::from(vec![
            Span::styled(" Enter ", Style::default().fg(Color::Cyan)),
            Span::raw("Next  "),
            Span::styled("Up/Dn ", Style::default().fg(Color::Cyan)),
            Span::raw("Field  "),
            Span::styled("Esc ", Style::default().fg(Color::Cyan)),
            Span::raw("Back  "),
            Span::styled("Ctrl+S ", Style::default().fg(Color::Cyan)),
            Span::raw("Save  "),
            status_span(state),
        ]),
        (Tab::Mcps, Focus::List) => Line::from(vec![
            Span::styled(" Tab ", Style::default().fg(Color::Cyan)),
            Span::raw("Switch  "),
            Span::styled("Enter ", Style::default().fg(Color::Cyan)),
            Span::raw("Edit  "),
            Span::styled("Ctrl+S ", Style::default().fg(Color::Cyan)),
            Span::raw("Save  "),
            Span::styled("q ", Style::default().fg(Color::Cyan)),
            Span::raw("Quit  "),
            status_span(state),
        ]),
        (Tab::Channels, Focus::List) => Line::from(vec![
            Span::styled(" Tab ", Style::default().fg(Color::Cyan)),
            Span::raw("Switch  "),
            Span::styled("Enter ", Style::default().fg(Color::Cyan)),
            Span::raw("Edit  "),
            Span::styled("x ", Style::default().fg(Color::Cyan)),
            Span::raw("Clear  "),
            Span::styled("Ctrl+S ", Style::default().fg(Color::Cyan)),
            Span::raw("Save  "),
            Span::styled("q ", Style::default().fg(Color::Cyan)),
            Span::raw("Quit  "),
            status_span(state),
        ]),
    };
    frame.render_widget(Paragraph::new(help), area);
}

fn status_span(state: &AuthState) -> Span<'_> {
    Span::styled(&state.status, Style::default().fg(Color::Green))
}
