use crate::{
    cmd::auth::{AuthState, Focus, PRESETS, PROVIDER_FIELDS, Tab, TreeItem, commit_provider_edit},
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
