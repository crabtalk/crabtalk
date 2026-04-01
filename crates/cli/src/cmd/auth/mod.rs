//! Interactive TUI for configuring LLM providers and MCP servers.

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
pub(crate) use wcore::config::{PROVIDER_PRESETS, ProviderPreset};
use wcore::protocol::{
    api::Client,
    message::{McpInfo, SourceKind},
};

use mcps::{handle_mcps_key, render_mcps};
use providers::{handle_providers_key, render_providers};

mod mcps;
mod providers;

/// Configure providers and MCP servers interactively.
#[derive(clap::Args, Debug)]
pub struct Auth {}

impl Auth {
    pub async fn run(self) -> Result<()> {
        let mut runner = crate::cmd::connect_default()
            .await
            .context("daemon must be running for auth — run 'crabtalk' first")?;

        // Load via protocol.
        let provider_infos = runner.list_providers().await?;
        let stats = runner.get_stats().await?;
        let mcp_infos = runner.list_mcps().await?;
        let initial_names: Vec<String> = provider_infos.iter().map(|p| p.name.clone()).collect();
        let active_model = stats.active_model;

        let state = tui::run_app_with_state(
            || AuthState::from_protocol(provider_infos, active_model, mcp_infos),
            render,
            handle_key,
        )?;

        if !state.needs_save {
            return Ok(());
        }

        // Save via protocol.
        runner.set_active_model(state.active_model.clone()).await?;

        // Delete removed providers.
        let final_names: Vec<String> = state.providers.iter().map(|p| p.name.clone()).collect();
        for name in &initial_names {
            if !final_names.contains(name) {
                let _ = runner.delete_provider(name.clone()).await;
            }
        }

        // Set all current providers.
        for p in &state.providers {
            let def = p.to_provider_def();
            let json = serde_json::to_string(&def).context("failed to serialize provider")?;
            runner.set_provider(p.name.clone(), json).await?;
        }

        // Save local MCPs.
        let local_mcps: Vec<McpInfo> = state
            .mcps
            .iter()
            .filter(|m| m.source == McpSource::Local)
            .map(McpData::to_mcp_info)
            .collect();
        runner.set_local_mcps(local_mcps).await?;

        Ok(())
    }
}

// ── Tabs ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Providers,
    Mcps,
}

const TAB_TITLES: &[&str] = &["Providers", "MCPs"];

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
    pub(crate) kind: String,
    pub(crate) models: Vec<String>,
}

impl ProviderData {
    /// Look up the preset for this provider by matching name.
    pub(crate) fn preset(&self) -> Option<&'static ProviderPreset> {
        PROVIDER_PRESETS.iter().find(|p| p.name == self.name)
    }

    /// Whether the base_url field is editable (not hardcoded by crabllm).
    pub(crate) fn base_url_editable(&self) -> bool {
        self.preset().is_none_or(|p| p.base_url_editable())
    }

    /// The display URL — fixed if hardcoded, otherwise whatever the user set.
    pub(crate) fn display_base_url(&self) -> &str {
        if let Some(preset) = self.preset()
            && !preset.fixed_base_url.is_empty()
        {
            return preset.fixed_base_url;
        }
        &self.base_url
    }

    /// Build a ProviderDef for serialization.
    pub(crate) fn to_provider_def(&self) -> wcore::ProviderDef {
        let kind: wcore::ApiStandard =
            serde_json::from_value(serde_json::Value::String(self.kind.clone()))
                .unwrap_or_default();
        wcore::ProviderDef {
            kind,
            api_key: if self.api_key.is_empty() {
                None
            } else {
                Some(self.api_key.clone())
            },
            base_url: if self.base_url.is_empty() {
                None
            } else {
                Some(self.base_url.clone())
            },
            models: self.models.clone(),
            ..Default::default()
        }
    }
}

pub(crate) const PROVIDER_FIELDS: &[&str] = &["api_key", "base_url", "kind"];

pub(crate) struct McpData {
    pub(crate) name: String,
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) env: Vec<(String, String)>,
    pub(crate) url: Option<String>,
    pub(crate) auth: bool,
    pub(crate) auto_restart: bool,
    pub(crate) source: McpSource,
}

impl McpData {
    pub(crate) fn to_mcp_info(&self) -> McpInfo {
        McpInfo {
            name: self.name.clone(),
            command: self.command.clone(),
            args: self.args.clone(),
            env: self.env.iter().cloned().collect(),
            url: self.url.clone().unwrap_or_default(),
            auth: self.auth,
            auto_restart: self.auto_restart,
            source: String::new(), // always local when saving
            enabled: true,
            source_kind: SourceKind::Local.into(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum McpSource {
    Local,
    Hub(String),
}

// ── Focus states ─────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    List,
    Editing,
    PresetSelector,
    /// Naming a custom provider after preset selection.
    NamingProvider,
    AddModel,
    AddMcp,
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
    // MCPs.
    pub(crate) mcps: Vec<McpData>,
    pub(crate) mcp_selected: usize,
    pub(crate) mcp_env_selected: usize,
    pub(crate) mcp_add_step: usize, // 0=name, 1=transport, 2=command/url, 3=args
    pub(crate) mcp_add_http: bool,  // true when adding an HTTP MCP
    // Shared.
    pub(crate) needs_save: bool,
    pub(crate) status: String,
}

impl AuthState {
    fn from_protocol(
        provider_infos: Vec<wcore::protocol::message::ProviderInfo>,
        active_model: String,
        mcp_infos: Vec<McpInfo>,
    ) -> Result<Self> {
        let mut providers = Vec::new();
        for p in provider_infos {
            if p.config.is_empty() {
                continue;
            }
            let def: wcore::ProviderDef = serde_json::from_str(&p.config)
                .with_context(|| format!("invalid provider config for '{}'", p.name))?;
            providers.push(ProviderData {
                name: p.name,
                api_key: def.api_key.unwrap_or_default(),
                base_url: def.base_url.unwrap_or_default(),
                kind: serde_json::to_value(def.kind)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| "openai".to_string()),
                models: def.models,
            });
        }

        let mcps = mcp_infos
            .into_iter()
            .map(|m| McpData {
                name: m.name,
                command: m.command,
                args: m.args,
                env: m.env.into_iter().collect(),
                url: if m.url.is_empty() { None } else { Some(m.url) },
                auth: m.auth,
                auto_restart: m.auto_restart,
                source: if m.source.is_empty() || m.source == "local" {
                    McpSource::Local
                } else {
                    McpSource::Hub(m.source)
                },
            })
            .collect();

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
            mcps,
            mcp_selected: 0,
            mcp_env_selected: 0,
            mcp_add_step: 0,
            mcp_add_http: false,
            needs_save: false,
            status: String::from("Ready"),
        })
    }

    fn mark_saved(&mut self) {
        self.needs_save = true;
        self.status = String::from("Saved!");
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
            2 => &p.kind,
            _ => "",
        }
    }

    pub(crate) fn set_provider_field(&mut self, pi: usize, field: usize, val: String) {
        let p = &mut self.providers[pi];
        match field {
            0 => p.api_key = val,
            1 => p.base_url = val,
            2 => p.kind = val,
            _ => {}
        }
    }

    pub(crate) fn add_preset(&mut self, preset: &ProviderPreset, name: Option<&str>) {
        let kind_str = serde_json::to_value(preset.kind)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "openai".to_string());
        self.providers.push(ProviderData {
            name: name.unwrap_or(preset.name).to_string(),
            api_key: String::new(),
            base_url: preset.base_url.to_string(),
            kind: kind_str,
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
        state.mark_saved();
        return Ok(None);
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Ok(Some(Ok(())));
    }

    // Tab switching only in list focus.
    if key.code == KeyCode::Tab && state.focus == Focus::List {
        state.tab = match state.tab {
            Tab::Providers => Tab::Mcps,
            Tab::Mcps => Tab::Providers,
        };
        return Ok(None);
    }

    match state.tab {
        Tab::Providers => handle_providers_key(key, state),
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
        .title(" Crabtalk Auth ")
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
        Tab::Mcps => 1,
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
        Tab::Mcps => render_mcps(frame, state, vert[1]),
    }

    render_status(frame, state, vert[2]);
}

// ── Status bar ──────────────────────────────────────────────────────

fn render_status(frame: &mut Frame, state: &AuthState, area: Rect) {
    let help = match (state.tab, state.focus) {
        (_, Focus::PresetSelector | Focus::NamingProvider) => Line::from(vec![
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
            Span::styled("n ", Style::default().fg(Color::Cyan)),
            Span::raw("New  "),
            Span::styled("Enter ", Style::default().fg(Color::Cyan)),
            Span::raw("Edit  "),
            Span::styled("d ", Style::default().fg(Color::Cyan)),
            Span::raw("Delete  "),
            Span::styled("Ctrl+S ", Style::default().fg(Color::Cyan)),
            Span::raw("Save  "),
            Span::styled("q ", Style::default().fg(Color::Cyan)),
            Span::raw("Quit  "),
            status_span(state),
        ]),
        (_, Focus::AddMcp) => Line::from(vec![
            Span::styled(" Enter ", Style::default().fg(Color::Cyan)),
            Span::raw("Next  "),
            Span::styled("Esc ", Style::default().fg(Color::Cyan)),
            Span::raw("Cancel  "),
            status_span(state),
        ]),
    };
    frame.render_widget(Paragraph::new(help), area);
}

fn status_span(state: &AuthState) -> Span<'_> {
    Span::styled(&state.status, Style::default().fg(Color::Green))
}
