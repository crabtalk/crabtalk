use crate::{
    cmd::config::{
        AuthState, Focus, PROVIDER_FIELDS, PROVIDER_PRESETS, ProviderData, Tab, TreeItem,
        commit_provider_edit,
    },
    tui::{border_dim, border_focused, char_to_byte, handle_text_input, mask_token},
};
use anyhow::Result;
use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

// ── Providers key handling ──────────────────────────────────────────

pub(crate) fn handle_providers_key(
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
        Focus::NamingProvider => {
            handle_naming_provider(key, state);
            Ok(None)
        }
        Focus::AddModel => {
            handle_add_model(key, state);
            Ok(None)
        }
        _ => Ok(None),
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

/// Advance to the next editable field, skipping non-editable ones.
/// Returns the next field index, or None if no more fields.
///
/// Field index 1 is `PROVIDER_FIELDS[1]` ("base_url") — skipped for
/// providers whose URL is hardcoded in crabllm.
fn next_editable_field(provider: &ProviderData, from: usize) -> Option<usize> {
    let mut next = from + 1;
    while next < PROVIDER_FIELDS.len() {
        if next == 1 && !provider.base_url_editable() {
            next += 1;
            continue;
        }
        return Some(next);
    }
    None
}

/// Move to the previous editable field, skipping non-editable ones.
/// See `next_editable_field` for the field-index convention.
fn prev_editable_field(provider: &ProviderData, from: usize) -> Option<usize> {
    let mut prev = from.checked_sub(1)?;
    loop {
        if prev == 1 && !provider.base_url_editable() {
            prev = prev.checked_sub(1)?;
            continue;
        }
        return Some(prev);
    }
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
                    toggle_kind(state, pi);
                    return;
                }
                if let Some(next) = next_editable_field(&state.providers[pi], field) {
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
                && let Some(TreeItem::Provider(pi)) = state.selected_item()
                && let Some(prev) = prev_editable_field(&state.providers[pi], field)
            {
                commit_provider_edit(state);
                state.editing_field = Some(prev);
                let val = state.provider_field_value(pi, prev).to_string();
                state.cursor = val.chars().count();
                state.edit_buf = val;
            }
        }
        KeyCode::Down => {
            if let Some(field) = state.editing_field
                && let Some(TreeItem::Provider(pi)) = state.selected_item()
                && let Some(next) = next_editable_field(&state.providers[pi], field)
            {
                commit_provider_edit(state);
                state.editing_field = Some(next);
                let val = state.provider_field_value(pi, next).to_string();
                state.cursor = val.chars().count();
                state.edit_buf = val;
            }
        }
        KeyCode::Tab => {
            if state.editing_field == Some(2)
                && let Some(TreeItem::Provider(pi)) = state.selected_item()
            {
                toggle_kind(state, pi);
            }
        }
        _ => handle_text_input(key.code, &mut state.edit_buf, &mut state.cursor),
    }
}

use wcore::ApiStandard;

const API_STANDARDS: &[ApiStandard] = &[
    ApiStandard::Openai,
    ApiStandard::Anthropic,
    ApiStandard::Google,
    ApiStandard::Bedrock,
    ApiStandard::Ollama,
    ApiStandard::Azure,
];

fn toggle_kind(state: &mut AuthState, pi: usize) {
    let p = &mut state.providers[pi];
    // Don't allow changing the kind for built-in presets — it would
    // break the preset lookup and hide the fixed URL indicator.
    if p.preset().is_some() {
        state.status = String::from("Kind is fixed for built-in providers");
        return;
    }
    let cur_kind: ApiStandard =
        serde_json::from_value(serde_json::Value::String(p.kind.clone())).unwrap_or_default();
    let cur = API_STANDARDS
        .iter()
        .position(|s| *s == cur_kind)
        .unwrap_or(0);
    let next = (cur + 1) % API_STANDARDS.len();
    let next_str = serde_json::to_value(&API_STANDARDS[next])
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "openai".to_string());
    p.kind = next_str;
    state.edit_buf.clone_from(&state.providers[pi].kind);
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
            if state.preset_idx < PROVIDER_PRESETS.len() - 1 {
                state.preset_idx += 1;
            }
        }
        KeyCode::Enter => {
            let preset = &PROVIDER_PRESETS[state.preset_idx];
            if state.providers.iter().any(|p| p.name == preset.name) {
                // Name taken — prompt for a custom name.
                state.edit_buf = preset.name.to_string();
                state.cursor = state.edit_buf.len();
                state.focus = Focus::NamingProvider;
            } else {
                state.add_preset(preset, None);
                state.status = format!("Added provider: {}", preset.name);
                state.focus = Focus::List;
            }
        }
        // Custom name for any preset.
        KeyCode::Char('n') => {
            state.edit_buf.clear();
            state.cursor = 0;
            state.focus = Focus::NamingProvider;
        }
        _ => {}
    }
}

fn handle_naming_provider(key: crossterm::event::KeyEvent, state: &mut AuthState) {
    match key.code {
        KeyCode::Esc => {
            state.focus = Focus::PresetSelector;
        }
        KeyCode::Enter => {
            let name = state.edit_buf.trim().to_string();
            if name.is_empty() {
                state.status = String::from("Provider name is required");
                return;
            }
            if state.providers.iter().any(|p| p.name == name) {
                state.status = format!("Provider '{}' already exists", name);
                return;
            }
            let preset = &PROVIDER_PRESETS[state.preset_idx];
            state.add_preset(preset, Some(&name));
            state.status = format!("Added provider: {}", name);
            state.focus = Focus::List;
        }
        _ => handle_text_input(key.code, &mut state.edit_buf, &mut state.cursor),
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

// ── Providers rendering ─────────────────────────────────────────────

pub(crate) fn render_providers(frame: &mut Frame, state: &AuthState, area: Rect) {
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
                    let is_fixed = fi == 1 && !p.base_url_editable();
                    let label_style = Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD);
                    let label_span = Span::styled(format!(" {:>10}: ", label), label_style);

                    let value = if is_fixed {
                        // Show the hardcoded URL as read-only.
                        Span::styled(
                            p.display_base_url(),
                            Style::default()
                                .fg(Color::DarkGray)
                                .add_modifier(Modifier::ITALIC),
                        )
                    } else if is_editing {
                        if fi == 0 {
                            // Mask API key while editing — show * for each char, cursor as |.
                            let char_count = state.edit_buf.chars().count();
                            let mut s = "*".repeat(char_count);
                            let byte_pos = state.cursor.min(char_count);
                            s.insert(byte_pos, '|');
                            Span::styled(s, Style::default().fg(Color::Green))
                        } else {
                            let byte_pos = char_to_byte(&state.edit_buf, state.cursor);
                            let mut s = state.edit_buf.clone();
                            s.insert(byte_pos, '|');
                            Span::styled(s, Style::default().fg(Color::Green))
                        }
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

                    let indicator = if is_fixed {
                        " (fixed)"
                    } else if is_editing {
                        " <"
                    } else {
                        ""
                    };
                    let indicator_color = if is_fixed {
                        Color::DarkGray
                    } else {
                        Color::Yellow
                    };
                    Line::from(vec![
                        label_span,
                        value,
                        Span::styled(indicator, Style::default().fg(indicator_color)),
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
    if state.focus == Focus::NamingProvider {
        render_naming_provider(frame, state, area);
        return;
    }

    let block = Block::default()
        .title(" Select Provider Preset ")
        .borders(Borders::ALL)
        .border_style(border_focused());

    let lines: Vec<Line> = PROVIDER_PRESETS
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
            let url = if preset.fixed_base_url.is_empty() {
                preset.base_url
            } else {
                preset.fixed_base_url
            };
            let detail = if url.is_empty() {
                String::new()
            } else {
                format!("  ({})", url)
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

fn render_naming_provider(frame: &mut Frame, state: &AuthState, area: Rect) {
    let block = Block::default()
        .title(" Name Your Provider ")
        .borders(Borders::ALL)
        .border_style(border_focused());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let byte_pos = char_to_byte(&state.edit_buf, state.cursor);
    let mut s = state.edit_buf.clone();
    s.insert(byte_pos, '|');

    let lines = vec![
        Line::from(vec![
            Span::styled(
                " Provider name: ",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(s, Style::default().fg(Color::Green)),
        ]),
        Line::from(Span::styled(
            " (used as [provider.<name>] in config.toml)",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )),
    ];

    frame.render_widget(Paragraph::new(lines), inner);
}
