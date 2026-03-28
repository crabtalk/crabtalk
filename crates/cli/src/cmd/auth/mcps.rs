use crate::{
    cmd::auth::{AuthState, Focus, McpData, McpSource, Tab},
    tui::{border_dim, border_focused, char_to_byte, handle_text_input, mask_token},
};
use anyhow::Result;
use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

// ── Helpers ──────────────────────────────────────────────────────────

fn selected_is_hub(state: &AuthState) -> bool {
    state
        .mcps
        .get(state.mcp_selected)
        .is_some_and(|m| matches!(m.source, McpSource::Hub(_)))
}

fn has_token(name: &str) -> bool {
    wcore::paths::TOKENS_DIR
        .join(format!("{name}.json"))
        .exists()
}

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
                    if selected_is_hub(state) {
                        state.status = String::from("Hub packages are read-only");
                    } else if let Some(mcp) = state.mcps.get(state.mcp_selected)
                        && !mcp.env.is_empty()
                    {
                        state.mcp_env_selected = 0;
                        let val = mcp.env[0].1.clone();
                        state.cursor = val.chars().count();
                        state.edit_buf = val;
                        state.focus = Focus::Editing;
                    }
                }
                KeyCode::Char('n') => {
                    state.mcp_add_step = 0;
                    state.mcp_add_http = false;
                    state.edit_buf = String::new();
                    state.cursor = 0;
                    state.focus = Focus::AddMcp;
                }
                KeyCode::Char('d') | KeyCode::Delete => {
                    if selected_is_hub(state) {
                        state.status = String::from("Use `hub uninstall` to remove");
                    } else if !state.mcps.is_empty() {
                        state.mcps.remove(state.mcp_selected);
                        if state.mcp_selected >= state.mcps.len() && !state.mcps.is_empty() {
                            state.mcp_selected = state.mcps.len() - 1;
                        }
                        state.mcp_env_selected = 0;
                        state.status = String::from("MCP deleted");
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
        Focus::AddMcp => {
            handle_add_mcp(key, state);
            Ok(None)
        }
        _ => Ok(None),
    }
}

fn handle_add_mcp(key: crossterm::event::KeyEvent, state: &mut AuthState) {
    match key.code {
        KeyCode::Esc => {
            // Cancel: remove partially added MCP if we already created one.
            if state.mcp_add_step > 1 {
                state.mcps.pop();
                if state.mcp_selected >= state.mcps.len() && !state.mcps.is_empty() {
                    state.mcp_selected = state.mcps.len() - 1;
                }
            }
            state.focus = Focus::List;
        }
        KeyCode::Enter => {
            let buf = state.edit_buf.trim().to_string();
            match state.mcp_add_step {
                0 => {
                    // Name step.
                    if buf.is_empty() {
                        state.status = String::from("Name cannot be empty");
                        return;
                    }
                    state.mcp_add_step = 1;
                    state.edit_buf = buf;
                    state.cursor = 0;
                    state.status = String::from("Transport: s=stdio, h=http");
                }
                1 => {
                    // Transport step — handled by char keys below.
                    state.status = String::from("Press s for stdio or h for http");
                }
                2 if state.mcp_add_http => {
                    // URL step.
                    if buf.is_empty() {
                        state.status = String::from("URL cannot be empty");
                        return;
                    }
                    if let Some(mcp) = state.mcps.last_mut() {
                        mcp.url = Some(buf);
                    }
                    state.focus = Focus::List;
                    state.status = String::from("MCP added");
                }
                2 => {
                    // Command step (stdio).
                    if let Some(mcp) = state.mcps.last_mut() {
                        mcp.command = buf;
                    }
                    state.mcp_add_step = 3;
                    state.edit_buf = String::new();
                    state.cursor = 0;
                    state.status = String::from("Enter args (space-separated)");
                }
                3 => {
                    // Args step (stdio).
                    if let Some(mcp) = state.mcps.last_mut() {
                        mcp.args = if buf.is_empty() {
                            Vec::new()
                        } else {
                            buf.split_whitespace().map(String::from).collect()
                        };
                    }
                    state.focus = Focus::List;
                    state.status = String::from("MCP added");
                }
                _ => {}
            }
        }
        KeyCode::Char('s') | KeyCode::Char('h') if state.mcp_add_step == 1 => {
            let is_http = key.code == KeyCode::Char('h');
            state.mcp_add_http = is_http;
            let name = state.edit_buf.clone();
            state.mcps.push(McpData {
                name,
                command: String::new(),
                args: Vec::new(),
                env: Vec::new(),
                url: None,
                auth: false,
                source: McpSource::Local,
            });
            state.mcp_selected = state.mcps.len() - 1;
            state.mcp_add_step = 2;
            state.edit_buf = String::new();
            state.cursor = 0;
            state.status = if is_http {
                String::from("Enter URL")
            } else {
                String::from("Enter command")
            };
        }
        _ if state.mcp_add_step != 1 => {
            handle_text_input(key.code, &mut state.edit_buf, &mut state.cursor);
        }
        _ => {}
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
    if state.focus == Focus::AddMcp {
        render_add_mcp(frame, state, area);
        return;
    }
    if state.mcps.is_empty() {
        let block = Block::default()
            .title(" MCP Servers ")
            .borders(Borders::ALL)
            .border_style(border_dim());
        let text = Paragraph::new(vec![
            Line::raw(""),
            Line::styled("No MCPs configured.", Style::default().fg(Color::DarkGray)),
            Line::raw(""),
            Line::styled(
                "Press n to add one, or find one at",
                Style::default().fg(Color::DarkGray),
            ),
            Line::styled(
                "https://crabtalk.ai/hub?type=mcp",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::ITALIC),
            ),
        ])
        .alignment(Alignment::Center)
        .block(block);
        frame.render_widget(text, area);
        return;
    }
    let horiz =
        Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)]).split(area);
    render_mcp_list(frame, state, horiz[0]);
    render_mcp_detail(frame, state, horiz[1]);
}

fn render_add_mcp(frame: &mut Frame, state: &AuthState, area: Rect) {
    let step_label = match (state.mcp_add_step, state.mcp_add_http) {
        (0, _) => "Name",
        (1, _) => "Transport (s=stdio, h=http)",
        (2, true) => "URL",
        (2, false) => "Command",
        (3, _) => "Args (space-separated)",
        _ => "",
    };
    let block = Block::default()
        .title(format!(" Add MCP — {step_label} "))
        .borders(Borders::ALL)
        .border_style(border_focused());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.mcp_add_step == 1 {
        // Transport selector — no text input, just s/h keys.
        let line = Line::from(vec![
            Span::styled(" Press ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "s",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" for stdio, ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "h",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" for http", Style::default().fg(Color::DarkGray)),
        ]);
        frame.render_widget(Paragraph::new(line), inner);
    } else {
        let byte_pos = char_to_byte(&state.edit_buf, state.cursor);
        let mut s = state.edit_buf.clone();
        s.insert(byte_pos, '|');

        let line = Line::from(vec![
            Span::styled(
                format!(" {step_label}: "),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(s, Style::default().fg(Color::Green)),
        ]);
        frame.render_widget(Paragraph::new(line), inner);
    }
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

            // Auth/status indicator.
            let indicator = if mcp.url.is_some() {
                if has_token(&mcp.name) {
                    " \u{2713}" // ✓
                } else if mcp.auth {
                    " !" // needs login
                } else {
                    ""
                }
            } else if mcp.env.iter().any(|(_, v)| !v.is_empty()) {
                " *"
            } else {
                ""
            };

            let hub_tag = if matches!(mcp.source, McpSource::Hub(_)) {
                " [hub]"
            } else {
                ""
            };

            let text = format!("{marker}{}{indicator}{hub_tag}", mcp.name);
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
            Paragraph::new("Press n to add an MCP server").block(block),
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

    let label_style = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    let dim_style = Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::ITALIC);

    let mut lines: Vec<Line> = Vec::new();

    // Source.
    let source_text = match &mcp.source {
        McpSource::Local => "local",
        McpSource::Hub(pkg) => pkg.as_str(),
    };
    lines.push(Line::from(vec![
        Span::styled("  source: ", label_style),
        Span::styled(source_text, Style::default().fg(Color::DarkGray)),
    ]));

    // Transport.
    if let Some(ref url) = mcp.url {
        lines.push(Line::from(vec![
            Span::styled("     url: ", label_style),
            Span::styled(url, Style::default().fg(Color::White)),
        ]));

        // Auth status.
        let (auth_text, auth_color) = if has_token(&mcp.name) {
            ("\u{2713} logged in", Color::Green)
        } else if mcp.auth {
            ("! not authenticated", Color::Red)
        } else {
            ("none", Color::DarkGray)
        };
        lines.push(Line::from(vec![
            Span::styled("    auth: ", label_style),
            Span::styled(auth_text, Style::default().fg(auth_color)),
        ]));
    } else {
        // Command.
        let cmd_val = if mcp.command.is_empty() {
            Span::styled("(none)", dim_style)
        } else {
            Span::styled(&mcp.command, Style::default().fg(Color::White))
        };
        lines.push(Line::from(vec![
            Span::styled(" command: ", label_style),
            cmd_val,
        ]));

        // Args.
        let args_display = if mcp.args.is_empty() {
            Span::styled("(none)", dim_style)
        } else {
            Span::styled(mcp.args.join(" "), Style::default().fg(Color::White))
        };
        lines.push(Line::from(vec![
            Span::styled("    args: ", label_style),
            args_display,
        ]));
    }

    // Separator.
    lines.push(Line::raw(""));

    // Env vars.
    if mcp.env.is_empty() {
        lines.push(Line::from(Span::styled("  (no env vars)", dim_style)));
    } else {
        for (ei, (key, val)) in mcp.env.iter().enumerate() {
            let is_editing = editing && ei == state.mcp_env_selected;
            let label_span = Span::styled(format!(" {key}: "), label_style);

            let value_span = if is_editing {
                let byte_pos = char_to_byte(&state.edit_buf, state.cursor);
                let mut s = state.edit_buf.clone();
                s.insert(byte_pos, '|');
                Span::styled(s, Style::default().fg(Color::Green))
            } else if val.is_empty() {
                Span::styled("(empty)", dim_style)
            } else {
                Span::styled(mask_token(val), Style::default().fg(Color::White))
            };

            let indicator = if is_editing { " <" } else { "" };
            lines.push(Line::from(vec![
                label_span,
                value_span,
                Span::styled(indicator, Style::default().fg(Color::Yellow)),
            ]));
        }
    }

    frame.render_widget(Paragraph::new(lines), inner);
}
