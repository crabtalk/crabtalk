//! Interactive TUI for configuring LLM providers, models, and channel tokens.

use crate::tui;
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

use channels::{handle_channels_key, render_channels};
use mcps::{handle_mcps_key, render_mcps};
use providers::{handle_providers_key, render_providers};

mod channels;
mod mcps;
mod providers;

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
pub(crate) enum Tab {
    Providers,
    Channels,
    Mcps,
}

const TAB_TITLES: &[&str] = &["Providers", "Channels", "MCPs"];

// ── Tree items (providers tab) ───────────────────────────────────────

#[derive(Clone)]
pub(crate) enum TreeItem {
    Provider(usize),
    Model(usize, usize),
}

pub(crate) struct ProviderData {
    pub(crate) name: String,
    pub(crate) api_key: String,
    pub(crate) base_url: String,
    pub(crate) standard: String,
    pub(crate) models: Vec<String>,
}

pub(crate) const PROVIDER_FIELDS: &[&str] = &["api_key", "base_url", "standard"];
pub(crate) const CHANNEL_NAMES: &[&str] = &["Telegram", "Discord"];

pub(crate) struct McpData {
    pub(crate) name: String,
    pub(crate) env: Vec<(String, String)>,
}

// ── Focus states ─────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    List,
    Editing,
    PresetSelector,
    AddModel,
}

// ── State ────────────────────────────────────────────────────────────

pub(crate) struct AuthState {
    pub(crate) tab: Tab,
    pub(crate) focus: Focus,
    // Providers.
    pub(crate) providers: Vec<ProviderData>,
    pub(crate) active_model: String,
    pub(crate) selected: usize,
    pub(crate) editing_field: Option<usize>,
    pub(crate) cursor: usize,
    pub(crate) edit_buf: String,
    pub(crate) preset_idx: usize,
    // Channels.
    pub(crate) channel_selected: usize,
    pub(crate) channel_tokens: [String; 2],
    // MCPs.
    pub(crate) mcps: Vec<McpData>,
    pub(crate) mcp_selected: usize,
    pub(crate) mcp_env_selected: usize,
    // Shared.
    pub(crate) status: String,
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

    pub(crate) fn tree_items(&self) -> Vec<TreeItem> {
        let mut items = Vec::new();
        for (pi, p) in self.providers.iter().enumerate() {
            items.push(TreeItem::Provider(pi));
            for (mi, _) in p.models.iter().enumerate() {
                items.push(TreeItem::Model(pi, mi));
            }
        }
        items
    }

    pub(crate) fn tree_len(&self) -> usize {
        self.providers
            .iter()
            .map(|p| 1 + p.models.len())
            .sum::<usize>()
    }

    pub(crate) fn selected_item(&self) -> Option<TreeItem> {
        self.tree_items().get(self.selected).cloned()
    }

    pub(crate) fn provider_field_value(&self, pi: usize, field: usize) -> &str {
        let p = &self.providers[pi];
        match field {
            0 => &p.api_key,
            1 => &p.base_url,
            2 => &p.standard,
            _ => "",
        }
    }

    pub(crate) fn set_provider_field(&mut self, pi: usize, field: usize, val: String) {
        let p = &mut self.providers[pi];
        match field {
            0 => p.api_key = val,
            1 => p.base_url = val,
            2 => p.standard = val,
            _ => {}
        }
    }

    pub(crate) fn add_preset(&mut self, preset: &Preset) {
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

// ── Shared helpers ──────────────────────────────────────────────────

pub(crate) fn commit_provider_edit(state: &mut AuthState) {
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
