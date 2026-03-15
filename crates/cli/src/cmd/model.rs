//! Interactive TUI for configuring LLM providers and models.

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use std::io::Stdout;
use toml_edit::{Array, DocumentMut, Item, Table, value};

/// Configure LLM providers and models interactively.
#[derive(clap::Args, Debug)]
pub struct Model;

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

// ── Tree item addressing ────────────────────────────────────────────

/// An item in the left-panel tree: either a provider header or a model
/// nested under a provider.
#[derive(Clone)]
enum TreeItem {
    Provider(usize),
    Model(usize, usize),
}

// ── Provider data ───────────────────────────────────────────────────

struct ProviderData {
    name: String,
    api_key: String,
    base_url: String,
    standard: String,
    models: Vec<String>,
}

// ── Fields when editing a provider ──────────────────────────────────

const PROVIDER_FIELDS: &[&str] = &["api_key", "base_url", "standard"];

// ── Focus states ────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    List,
    Editing,
    PresetSelector,
    AddModel,
}

// ── Main state ──────────────────────────────────────────────────────

struct ModelState {
    focus: Focus,
    providers: Vec<ProviderData>,
    active_model: String,
    /// Index into the flattened tree.
    selected: usize,
    /// Which provider field is being edited (index into PROVIDER_FIELDS)
    /// or None when editing a model name.
    editing_field: Option<usize>,
    cursor: usize,
    edit_buf: String,
    /// Preset selector index.
    preset_idx: usize,
    status: String,
}

impl ModelState {
    fn load() -> Result<Self> {
        let config_path = wcore::paths::CONFIG_DIR.join("walrus.toml");
        let mut providers = Vec::new();
        let mut active_model = String::new();

        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)
                .with_context(|| format!("cannot read {}", config_path.display()))?;
            let doc: DocumentMut = content
                .parse()
                .with_context(|| format!("invalid TOML in {}", config_path.display()))?;

            // Read active model from [walrus] model.
            if let Some(walrus) = doc.get("walrus").and_then(|w| w.as_table())
                && let Some(m) = walrus.get("model").and_then(|v| v.as_str())
            {
                active_model = m.to_string();
            }

            // Read [provider.*] sections.
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
        }

        Ok(Self {
            focus: Focus::List,
            providers,
            active_model,
            selected: 0,
            editing_field: None,
            cursor: 0,
            edit_buf: String::new(),
            preset_idx: 0,
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

        // Write active model to [walrus].model.
        if !self.active_model.is_empty() {
            if doc.get("walrus").is_none() {
                doc.insert("walrus", Item::Table(Table::new()));
            }
            if let Some(walrus) = doc.get_mut("walrus").and_then(|w| w.as_table_mut()) {
                walrus.insert("model", value(&self.active_model));
            }
        }

        // Rebuild [provider.*] sections.
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

        std::fs::write(&config_path, doc.to_string())
            .with_context(|| format!("failed to write {}", config_path.display()))?;

        self.status = String::from("Saved!");
        Ok(())
    }

    /// Build the flattened tree of items.
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

    /// Get the field value for the currently selected provider + field index.
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
        // Select the new provider.
        let new_idx = self.tree_len().saturating_sub(1);
        self.selected = new_idx;
    }
}

// ── Char-to-byte helper ─────────────────────────────────────────────

fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

// ── Entry point ─────────────────────────────────────────────────────

impl Model {
    pub fn run(self) -> Result<()> {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen)?;

        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
            original_hook(info);
        }));

        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        let mut state = ModelState::load()?;
        let result = run_loop(&mut terminal, &mut state);

        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;

        result
    }
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    state: &mut ModelState,
) -> Result<()> {
    loop {
        terminal.draw(|frame| render(frame, state))?;
        if event::poll(std::time::Duration::from_millis(250))?
            && let Event::Key(key) = event::read()?
            && handle_key(key, state)?
        {
            return Ok(());
        }
    }
}

// ── Key handling ────────────────────────────────────────────────────

fn handle_key(key: event::KeyEvent, state: &mut ModelState) -> Result<bool> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
        if let Err(e) = state.save() {
            state.status = format!("Error: {e}");
        }
        return Ok(false);
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Ok(true);
    }

    match state.focus {
        Focus::List => handle_list_key(key, state),
        Focus::Editing => Ok(handle_editing(key, state)),
        Focus::PresetSelector => Ok(handle_preset(key, state)),
        Focus::AddModel => Ok(handle_add_model(key, state)),
    }
}

fn handle_list_key(key: event::KeyEvent, state: &mut ModelState) -> Result<bool> {
    let tree_len = state.tree_len();
    match key.code {
        KeyCode::Char('q') => return Ok(true),
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
            // Add model to the provider containing the selection.
            if let Some(item) = state.selected_item() {
                let pi = match item {
                    TreeItem::Provider(pi) => pi,
                    TreeItem::Model(pi, _) => pi,
                };
                state.providers[pi].models.push(String::new());
                // Select the new model.
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
            // Set active model.
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
    Ok(false)
}

fn handle_editing(key: event::KeyEvent, state: &mut ModelState) -> bool {
    match key.code {
        KeyCode::Esc => {
            state.focus = Focus::List;
        }
        KeyCode::Enter => {
            // Commit edit buffer to the field.
            commit_edit(state);
            // If editing a provider field, cycle to next or return to list.
            if let Some(field) = state.editing_field
                && let Some(TreeItem::Provider(pi)) = state.selected_item()
            {
                if field == 2 {
                    // Toggle standard on Enter for the standard field.
                    let p = &mut state.providers[pi];
                    p.standard = if p.standard == "anthropic" {
                        "openai".to_string()
                    } else {
                        "anthropic".to_string()
                    };
                    state.edit_buf = state.providers[pi].standard.clone();
                    state.cursor = state.edit_buf.chars().count();
                    return false;
                }
                let next = field + 1;
                if next < PROVIDER_FIELDS.len() {
                    state.editing_field = Some(next);
                    let val = state.provider_field_value(pi, next).to_string();
                    state.cursor = val.chars().count();
                    state.edit_buf = val;
                    return false;
                }
            }
            state.focus = Focus::List;
        }
        KeyCode::Up => {
            if let Some(field) = state.editing_field
                && field > 0
            {
                commit_edit(state);
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
                commit_edit(state);
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
            // Toggle standard field if editing it.
            if state.editing_field == Some(2)
                && let Some(TreeItem::Provider(pi)) = state.selected_item()
            {
                let p = &mut state.providers[pi];
                p.standard = if p.standard == "anthropic" {
                    "openai".to_string()
                } else {
                    "anthropic".to_string()
                };
                state.edit_buf = state.providers[pi].standard.clone();
                state.cursor = state.edit_buf.chars().count();
            }
        }
        _ => handle_text_input(key.code, &mut state.edit_buf, &mut state.cursor),
    }
    false
}

fn handle_preset(key: event::KeyEvent, state: &mut ModelState) -> bool {
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
    false
}

fn handle_add_model(key: event::KeyEvent, state: &mut ModelState) -> bool {
    match key.code {
        KeyCode::Esc => {
            // Cancel — remove the empty model we added.
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
                commit_edit(state);
            } else {
                // Remove empty model on Enter with empty buffer.
                if let Some(TreeItem::Model(pi, mi)) = state.selected_item() {
                    state.providers[pi].models.remove(mi);
                    let tree_len = state.tree_len();
                    if state.selected >= tree_len && tree_len > 0 {
                        state.selected = tree_len - 1;
                    }
                }
            }
            state.focus = Focus::List;
        }
        _ => handle_text_input(key.code, &mut state.edit_buf, &mut state.cursor),
    }
    false
}

fn commit_edit(state: &mut ModelState) {
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

fn handle_text_input(code: KeyCode, buf: &mut String, cursor: &mut usize) {
    match code {
        KeyCode::Backspace => {
            if *cursor > 0 {
                let start = char_to_byte(buf, *cursor - 1);
                let end = char_to_byte(buf, *cursor);
                buf.drain(start..end);
                *cursor -= 1;
            }
        }
        KeyCode::Delete => {
            let char_count = buf.chars().count();
            if *cursor < char_count {
                let start = char_to_byte(buf, *cursor);
                let end = char_to_byte(buf, *cursor + 1);
                buf.drain(start..end);
            }
        }
        KeyCode::Left => {
            *cursor = cursor.saturating_sub(1);
        }
        KeyCode::Right => {
            let char_count = buf.chars().count();
            if *cursor < char_count {
                *cursor += 1;
            }
        }
        KeyCode::Home => {
            *cursor = 0;
        }
        KeyCode::End => {
            *cursor = buf.chars().count();
        }
        KeyCode::Char(c) => {
            let byte_pos = char_to_byte(buf, *cursor);
            buf.insert(byte_pos, c);
            *cursor += 1;
        }
        _ => {}
    }
}

// ── Rendering ───────────────────────────────────────────────────────

fn render(frame: &mut Frame, state: &ModelState) {
    let area = frame.area();

    let outer = Block::default()
        .title(" Walrus Model ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let vert = Layout::vertical([Constraint::Min(4), Constraint::Length(2)]).split(inner);
    let content_area = vert[0];
    let status_area = vert[1];

    if state.focus == Focus::PresetSelector {
        render_presets(frame, state, content_area);
    } else {
        let horiz = Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)])
            .split(content_area);
        render_tree(frame, state, horiz[0]);
        render_detail(frame, state, horiz[1]);
    }

    render_status(frame, state, status_area);
}

fn render_tree(frame: &mut Frame, state: &ModelState, area: Rect) {
    let focused = state.focus == Focus::List;
    let block = Block::default()
        .title(" Providers ")
        .borders(Borders::ALL)
        .border_style(if focused {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::DarkGray)
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

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn render_detail(frame: &mut Frame, state: &ModelState, area: Rect) {
    let editing = matches!(state.focus, Focus::Editing | Focus::AddModel);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if editing {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::DarkGray)
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
                            // Mask api_key.
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

fn render_presets(frame: &mut Frame, state: &ModelState, area: Rect) {
    let block = Block::default()
        .title(" Select Provider Preset ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

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

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn render_status(frame: &mut Frame, state: &ModelState, area: Rect) {
    let help = match state.focus {
        Focus::PresetSelector => Line::from(vec![
            Span::styled(" Enter ", Style::default().fg(Color::Cyan)),
            Span::raw("Select  "),
            Span::styled("Esc ", Style::default().fg(Color::Cyan)),
            Span::raw("Cancel  "),
            Span::styled("| ", Style::default().fg(Color::DarkGray)),
            Span::styled(&state.status, Style::default().fg(Color::Green)),
        ]),
        Focus::AddModel => Line::from(vec![
            Span::styled(" Enter ", Style::default().fg(Color::Cyan)),
            Span::raw("Confirm  "),
            Span::styled("Esc ", Style::default().fg(Color::Cyan)),
            Span::raw("Cancel  "),
            Span::styled("| ", Style::default().fg(Color::DarkGray)),
            Span::styled(&state.status, Style::default().fg(Color::Green)),
        ]),
        Focus::Editing => Line::from(vec![
            Span::styled(" Enter ", Style::default().fg(Color::Cyan)),
            Span::raw("Next  "),
            Span::styled("Up/Dn ", Style::default().fg(Color::Cyan)),
            Span::raw("Field  "),
            Span::styled("Esc ", Style::default().fg(Color::Cyan)),
            Span::raw("Back  "),
            Span::styled("Ctrl+S ", Style::default().fg(Color::Cyan)),
            Span::raw("Save  "),
            Span::styled("| ", Style::default().fg(Color::DarkGray)),
            Span::styled(&state.status, Style::default().fg(Color::Green)),
        ]),
        Focus::List => Line::from(vec![
            Span::styled(" n ", Style::default().fg(Color::Cyan)),
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
            Span::styled("| ", Style::default().fg(Color::DarkGray)),
            Span::styled(&state.status, Style::default().fg(Color::Green)),
        ]),
    };
    frame.render_widget(Paragraph::new(help), area);
}

/// Mask a token for display — show first 4 and last 4 ASCII chars.
fn mask_token(token: &str) -> String {
    let chars: Vec<char> = token.chars().collect();
    if chars.len() <= 8 {
        "*".repeat(chars.len())
    } else {
        let head: String = chars[..4].iter().collect();
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("{head}{}{tail}", "*".repeat(chars.len() - 8))
    }
}
