use crate::{
    cmd::auth::{AuthState, Focus, Tab},
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

// ── MCPs key handling ───────────────────────────────────────────────

pub(crate) fn handle_mcps_key(
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

// ── MCPs rendering ──────────────────────────────────────────────────

pub(crate) fn render_mcps(frame: &mut Frame, state: &AuthState, area: Rect) {
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
