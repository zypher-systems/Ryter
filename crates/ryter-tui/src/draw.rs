//! Layout: chat | sidebar, composer separated below.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};
use ryter_core::Phase;

use crate::chat::{self, short_model};
use crate::theme::Theme;
use crate::view::{HooksPane, McpPane, Overlay, SkillsPane, View};

const COMPOSER_H: u16 = 3;

/// Draw the current view.
pub fn draw(frame: &mut Frame, view: &View, theme: Theme) {
    fill(frame, frame.area(), theme.bg, theme.fg);

    let overlay_open = view.overlay.is_some();
    let slash_h = if overlay_open {
        0
    } else {
        view.slash
            .as_ref()
            .map(|s| {
                let cap = (frame.area().height.saturating_sub(8)).clamp(8, 16);
                u16::try_from(s.matches.len()).unwrap_or(cap).min(cap)
            })
            .unwrap_or(0)
    };
    let ask_h =
        u16::from(!overlay_open && (view.permission.is_some() || view.secret_for.is_some()));

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(4),
            Constraint::Length(slash_h),
            Constraint::Length(ask_h),
            Constraint::Length(1),
            Constraint::Length(COMPOSER_H),
            Constraint::Length(1),
        ])
        .split(frame.area());

    draw_header(frame, rows[0], view, theme);
    draw_hairline(frame, rows[1], theme.bg, theme.dim);

    let side_w = sidebar_width(frame.area().width);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(20),
            Constraint::Length(1),
            Constraint::Length(side_w),
        ])
        .split(rows[2]);
    draw_scrollback(frame, cols[0], view, theme);
    draw_vbar(frame, cols[1], theme);
    draw_sidebar(frame, cols[2], view, theme);

    if slash_h > 0 {
        draw_slash(frame, rows[3], view, theme);
    }
    if ask_h > 0 {
        draw_ask(frame, rows[4], view, theme);
    }
    draw_hairline(frame, rows[5], theme.composer_bg, theme.dim);
    draw_composer(frame, rows[6], view, theme, overlay_open);
    draw_footer(frame, rows[7], view, theme);
    if overlay_open {
        draw_overlay(frame, frame.area(), view, theme);
    }
}

fn sidebar_width(total: u16) -> u16 {
    if total < 72 {
        22
    } else if total < 110 {
        26
    } else {
        30
    }
}

fn fill(frame: &mut Frame, area: Rect, bg: ratatui::style::Color, fg: ratatui::style::Color) {
    frame.render_widget(Block::default().style(Style::default().bg(bg).fg(fg)), area);
}

fn draw_header(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    let busy = if view.busy { "  …" } else { "" };
    let mut left = vec![
        Span::raw(" "),
        Span::styled("ryter", theme.muted()),
        Span::styled(" · ", theme.muted()),
        Span::styled(
            "orchestrator",
            Style::default()
                .fg(theme.fg)
                .bg(theme.bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(busy, theme.body()),
    ];
    let left_w: usize = left.iter().map(|s| s.content.chars().count()).sum();
    let mut right: Vec<Span> = Vec::new();
    if !view.cwd.is_empty() {
        right.push(Span::styled(view.cwd.clone(), theme.body()));
    }
    if let Some(b) = &view.git_branch {
        right.push(Span::styled(" (", theme.muted()));
        right.push(Span::styled(b.clone(), theme.muted()));
        right.push(Span::styled(")", theme.muted()));
    }
    let right_w: usize = right.iter().map(|s| s.content.chars().count()).sum();
    let width = area.width as usize;
    let pad = width.saturating_sub(left_w + right_w + 1);
    if pad > 0 && right_w > 0 {
        left.push(Span::styled(" ".repeat(pad), theme.body()));
        left.extend(right);
        left.push(Span::raw(" "));
    }
    frame.render_widget(Paragraph::new(Line::from(left)).style(theme.body()), area);
}

fn draw_hairline(
    frame: &mut Frame,
    area: Rect,
    bg: ratatui::style::Color,
    dim: ratatui::style::Color,
) {
    let line = "─".repeat(area.width as usize);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            line,
            Style::default().fg(dim).bg(bg),
        ))),
        area,
    );
}

fn draw_vbar(frame: &mut Frame, area: Rect, theme: Theme) {
    let rows: Vec<Line> = (0..area.height)
        .map(|_| Line::from(Span::styled("│", theme.muted())))
        .collect();
    frame.render_widget(Paragraph::new(rows).style(theme.body()), area);
}

fn draw_scrollback(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    fill(frame, area, theme.bg, theme.fg);
    let lines = chat::render(view, area.width, area.height, theme);
    frame.render_widget(Paragraph::new(lines).style(theme.body()), area);
}

fn draw_sidebar(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    fill(frame, area, theme.sidebar_bg, theme.fg);
    let mut rows: Vec<Line> = Vec::new();
    push_kv(&mut rows, "spend", &view.spend_label(), theme);
    let used = view.ctx_tokens.unwrap_or(0);
    let window = view
        .ctx_window
        .unwrap_or_else(|| ryter_core::window_for(&view.model));
    push_kv(
        &mut rows,
        "context",
        &ryter_core::format_context_used(used, window),
        theme,
    );
    push_kv(&mut rows, "model", short_model(&view.model), theme);
    let provider_val = if view.has_key {
        format!("{} *", view.connection)
    } else {
        view.connection.clone()
    };
    push_kv(&mut rows, "provider", &provider_val, theme);
    if !view.price_label.is_empty() {
        push_kv(&mut rows, "price", &view.price_label, theme);
    }
    push_kv(&mut rows, "phase", view.phase.as_str(), theme);
    push_kv(
        &mut rows,
        "auditor",
        if view.auditor_on { "on" } else { "off" },
        theme,
    );
    push_kv(
        &mut rows,
        "tools",
        if view.perm_mode == "always" {
            "always"
        } else {
            "ask"
        },
        theme,
    );

    rows.push(Line::from(""));
    rows.push(Line::from(Span::styled("crew", theme.side_muted())));
    if view.crew.is_empty() {
        rows.push(Line::from(Span::styled(" none", theme.side_muted())));
    } else {
        for c in view.crew.iter().take(6) {
            let phase = match c.role.as_str() {
                "planner" => Phase::Plan,
                "architect" => Phase::Architect,
                "auditor" => Phase::Audit,
                _ => Phase::Build,
            };
            rows.push(Line::from(vec![
                Span::styled(" ", theme.side()),
                Span::styled(
                    c.role.clone(),
                    Style::default().fg(theme.phase(phase)).bg(theme.sidebar_bg),
                ),
                Span::styled(
                    format!("  {}  {}", trunc(&c.label, 12), c.status),
                    theme.side(),
                ),
            ]));
        }
    }

    rows.push(Line::from(""));
    rows.push(Line::from(Span::styled("tasks", theme.side_muted())));
    if view.todos.is_empty() {
        rows.push(Line::from(Span::styled(" none yet", theme.side_muted())));
    } else {
        for t in view.todos.iter().take(8) {
            rows.push(Line::from(Span::styled(
                format!(" {}  {}", trunc(&t.status, 7), trunc(&t.title, 16)),
                theme.side(),
            )));
        }
    }

    rows.push(Line::from(""));
    rows.push(Line::from(Span::styled(
        "/provider  /models",
        theme.side_muted(),
    )));

    frame.render_widget(
        Paragraph::new(rows)
            .wrap(Wrap { trim: true })
            .style(theme.side()),
        area,
    );
}

fn push_kv(rows: &mut Vec<Line<'static>>, key: &str, value: &str, theme: Theme) {
    rows.push(Line::from(Span::styled(
        key.to_string(),
        theme.side_muted(),
    )));
    rows.push(Line::from(Span::styled(format!(" {value}"), theme.side())));
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(n.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

fn draw_overlay(frame: &mut Frame, full: Rect, view: &View, theme: Theme) {
    let wide = matches!(
        view.overlay,
        Some(
            Overlay::Mcp { .. }
                | Overlay::Skills { .. }
                | Overlay::Hooks { .. }
                | Overlay::Spend
                | Overlay::Settings { .. }
        )
    );
    let w = if wide {
        full.width.clamp(44, 72)
    } else {
        full.width.clamp(36, 58)
    };
    let h = if wide {
        full.height.clamp(14, 22)
    } else {
        full.height.clamp(10, 18)
    };
    let x = full.x + (full.width.saturating_sub(w)) / 2;
    let y = full.y + (full.height.saturating_sub(h)) / 3;
    let area = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    fill(frame, area, theme.composer_bg, theme.fg);
    draw_hairline(
        frame,
        Rect {
            x,
            y,
            width: w,
            height: 1,
        },
        theme.composer_bg,
        theme.dim,
    );
    draw_hairline(
        frame,
        Rect {
            x,
            y: y + h.saturating_sub(1),
            width: w,
            height: 1,
        },
        theme.composer_bg,
        theme.dim,
    );
    let inner = Rect {
        x: x + 1,
        y: y + 1,
        width: w.saturating_sub(2),
        height: h.saturating_sub(2),
    };
    match &view.overlay {
        Some(Overlay::Provider { selected, .. }) => {
            draw_provider_overlay(frame, inner, view, *selected, theme);
        }
        Some(Overlay::Crew { selected, save_buf }) => {
            draw_crew_overlay(frame, inner, view, *selected, save_buf.as_deref(), theme);
        }
        Some(Overlay::Agents { selected }) => {
            draw_agents_overlay(frame, inner, view, *selected, theme);
        }
        Some(Overlay::Mcp { selected, pane }) => {
            draw_mcp_overlay(frame, inner, view, *selected, pane, theme);
        }
        Some(Overlay::Skills { selected, pane }) => {
            draw_skills_overlay(frame, inner, view, *selected, pane, theme);
        }
        Some(Overlay::Hooks { selected, pane }) => {
            draw_hooks_overlay(frame, inner, view, *selected, pane, theme);
        }
        Some(Overlay::Permission { tool, summary }) => {
            draw_permission_overlay(frame, inner, tool, summary, theme);
        }
        Some(Overlay::AskUser {
            question,
            options,
            selected,
            buf,
        }) => {
            draw_ask_user_overlay(frame, inner, question, options, *selected, buf, theme);
        }
        Some(Overlay::Spend) => {
            draw_spend_overlay(frame, inner, view, theme);
        }
        Some(Overlay::Settings { selected, edit }) => {
            draw_settings_overlay(frame, inner, view, *selected, edit.as_deref(), theme);
        }
        Some(Overlay::Model {
            filter,
            selected,
            loading,
            assign_role,
            ..
        }) => {
            draw_model_overlay(
                frame,
                inner,
                view,
                filter,
                *selected,
                *loading,
                assign_role.as_deref(),
                theme,
            );
        }
        Some(Overlay::Choice {
            title, selected, ..
        }) => {
            draw_choice_overlay(frame, inner, view, title, *selected, theme);
        }
        None => {}
    }
}

fn draw_provider_overlay(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    selected: usize,
    theme: Theme,
) {
    let rows_list = view.filtered_providers();
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "provider   enter · select   esc · close",
        Style::default().fg(theme.dim).bg(theme.composer_bg),
    ))];
    if rows_list.is_empty() {
        lines.push(Line::from(Span::styled(
            " no providers",
            Style::default().fg(theme.dim).bg(theme.composer_bg),
        )));
    } else {
        for (i, c) in rows_list.iter().enumerate() {
            let mark = if i == selected { "› " } else { "  " };
            let star = if c.has_key { " *" } else { "" };
            let on = if c.name == view.connection {
                "  default"
            } else {
                ""
            };
            let style = if i == selected {
                Style::default()
                    .fg(theme.prompt)
                    .bg(theme.composer_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg).bg(theme.composer_bg)
            };
            lines.push(Line::from(Span::styled(
                format!("{mark}{}{star}  {}{on}", c.name, short_model(&c.model)),
                style,
            )));
        }
    }
    if let Some(name) = &view.secret_for {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!(" paste API key for {name}"),
            Style::default().fg(theme.warn).bg(theme.composer_bg),
        )));
        let n = view.secret_buf.chars().count();
        let bullets = if n == 0 {
            "(hidden)".into()
        } else {
            "•".repeat(n)
        };
        lines.push(Line::from(Span::styled(
            format!(" {bullets}"),
            Style::default().fg(theme.fg).bg(theme.composer_bg),
        )));
    }
    let cursor_row = if view.secret_for.is_some() {
        Some((lines.len().saturating_sub(1)) as u16)
    } else {
        None
    };
    let cursor_col = if view.secret_for.is_some() {
        1 + view.secret_buf.chars().count() as u16
    } else {
        0
    };
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.composer_bg).fg(theme.fg)),
        area,
    );
    if let Some(row) = cursor_row {
        paint_block_cursor(
            frame,
            area.x + cursor_col.min(area.width.saturating_sub(1)),
            area.y + row.min(area.height.saturating_sub(1)),
            theme,
        );
    }
}

fn mcp_row_style(selected: bool, theme: Theme) -> Style {
    if selected {
        Style::default()
            .fg(theme.prompt)
            .bg(theme.composer_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg).bg(theme.composer_bg)
    }
}

fn mask_token(t: &str) -> String {
    if t.len() <= 12 {
        return "••••".into();
    }
    format!("{}…{}", &t[..8], &t[t.len() - 4..])
}

fn draw_mcp_overlay(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    selected: usize,
    pane: &McpPane,
    theme: Theme,
) {
    let dim = Style::default().fg(theme.dim).bg(theme.composer_bg);
    let mut lines: Vec<Line> = Vec::new();
    match pane {
        McpPane::Home => {
            lines.push(Line::from(Span::styled(
                "mcp   enter · select   backspace · remove server   esc · close",
                dim,
            )));
            let on = if view.mcp_inbound { "on" } else { "off" };
            let unix = view.mcp_listen.as_deref().unwrap_or("not listening");
            let mark = if selected == 0 { "› " } else { "  " };
            lines.push(Line::from(Span::styled(
                format!("{mark}inbound  {on}"),
                mcp_row_style(selected == 0, theme),
            )));
            lines.push(Line::from(Span::styled(format!("    {unix}"), dim)));
            let names = view.mcp_server_names();
            for (i, name) in names.iter().enumerate() {
                let idx = i + 1;
                let mark = if selected == idx { "› " } else { "  " };
                let srv = &view.mcp_servers[name];
                let state = if srv.enabled { "on" } else { "off" };
                lines.push(Line::from(Span::styled(
                    format!("{mark}{name}  {state}"),
                    mcp_row_style(selected == idx, theme),
                )));
                let cmd = if srv.args.is_empty() {
                    srv.command.clone()
                } else {
                    format!("{} {}", srv.command, srv.args.join(" "))
                };
                lines.push(Line::from(Span::styled(
                    format!(
                        "    {}",
                        trunc(&cmd, (area.width as usize).saturating_sub(6))
                    ),
                    dim,
                )));
            }
            let add_i = 1 + names.len();
            let mark = if selected == add_i { "› " } else { "  " };
            lines.push(Line::from(Span::styled(
                format!("{mark}add outbound"),
                mcp_row_style(selected == add_i, theme),
            )));
        }
        McpPane::Inbound => {
            lines.push(Line::from(Span::styled(
                "inbound   enter · toggle / copy   esc · back",
                dim,
            )));
            let rows = [
                format!("listen    {}", if view.mcp_inbound { "on" } else { "off" }),
                format!("stdio     {}", view.mcp_stdio_cmd()),
                format!(
                    "unix      {}",
                    view.mcp_unix_uri()
                        .unwrap_or_else(|| "not listening".into())
                ),
                format!(
                    "tcp       {}",
                    view.mcp_tcp_listen
                        .as_deref()
                        .or(view.mcp_bind.as_deref())
                        .unwrap_or("off")
                ),
                {
                    if view.mcp_tokens.is_empty() {
                        "token     none  · enter to create".into()
                    } else {
                        let (name, tok) = &view.mcp_tokens[0];
                        let shown = if view.mcp_reveal.as_deref() == Some(name.as_str()) {
                            tok.clone()
                        } else {
                            mask_token(tok)
                        };
                        format!("token     {name}  {shown}  · enter to rotate")
                    }
                },
                "snippet   cursor / claude mcp.json".into(),
            ];
            for (i, row) in rows.iter().enumerate() {
                let mark = if i == selected { "› " } else { "  " };
                lines.push(Line::from(Span::styled(
                    format!("{mark}{row}"),
                    mcp_row_style(i == selected, theme),
                )));
            }
        }
        McpPane::AddName { buf } => {
            lines.push(Line::from(Span::styled(
                "outbound name   enter · next   esc · back",
                dim,
            )));
            lines.push(Line::from(Span::styled(
                format!(" name: {buf}"),
                mcp_row_style(true, theme),
            )));
        }
        McpPane::AddCommand { name, buf } => {
            lines.push(Line::from(Span::styled(
                format!("{name} command   enter · next   esc · back"),
                dim,
            )));
            lines.push(Line::from(Span::styled(
                format!(" command: {buf}"),
                mcp_row_style(true, theme),
            )));
        }
        McpPane::AddArgs { name, command, buf } => {
            lines.push(Line::from(Span::styled(
                format!("{name} args   ({command})   enter · save   esc · back"),
                dim,
            )));
            lines.push(Line::from(Span::styled(
                format!(" args: {buf}"),
                mcp_row_style(true, theme),
            )));
        }
    }
    let cursor = match pane {
        McpPane::AddName { buf } => Some((" name: ".chars().count() + buf.chars().count()) as u16),
        McpPane::AddCommand { buf, .. } => {
            Some((" command: ".chars().count() + buf.chars().count()) as u16)
        }
        McpPane::AddArgs { buf, .. } => {
            Some((" args: ".chars().count() + buf.chars().count()) as u16)
        }
        _ => None,
    };
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.composer_bg).fg(theme.fg)),
        area,
    );
    if let Some(col) = cursor {
        paint_block_cursor(
            frame,
            area.x + col.min(area.width.saturating_sub(1)),
            area.y + 1,
            theme,
        );
    }
}

fn draw_skills_overlay(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    selected: usize,
    pane: &SkillsPane,
    theme: Theme,
) {
    let dim = Style::default().fg(theme.dim).bg(theme.composer_bg);
    let mut lines: Vec<Line> = Vec::new();
    let mut cursor: Option<u16> = None;
    match pane {
        SkillsPane::Home => {
            lines.push(Line::from(Span::styled(
                "skills   enter · run   backspace · remove   esc · close",
                dim,
            )));
            let mut i = 0usize;
            for sk in &view.catalog.skills {
                let mark = if i == selected { "› " } else { "  " };
                let flag = if sk.user_invocable { "slash" } else { "hidden" };
                lines.push(Line::from(Span::styled(
                    format!("{mark}/{}  {flag}", sk.name),
                    mcp_row_style(i == selected, theme),
                )));
                if !sk.description.is_empty() {
                    lines.push(Line::from(Span::styled(
                        format!(
                            "    {}",
                            trunc(&sk.description, (area.width as usize).saturating_sub(6))
                        ),
                        dim,
                    )));
                }
                i += 1;
            }
            for c in &view.catalog.commands {
                let mark = if i == selected { "› " } else { "  " };
                lines.push(Line::from(Span::styled(
                    format!("{mark}/{}  command", c.name),
                    mcp_row_style(i == selected, theme),
                )));
                if !c.description.is_empty() {
                    lines.push(Line::from(Span::styled(
                        format!(
                            "    {}",
                            trunc(&c.description, (area.width as usize).saturating_sub(6))
                        ),
                        dim,
                    )));
                }
                i += 1;
            }
            for (label, idx) in [("add skill", i), ("add command", i + 1)] {
                let mark = if selected == idx { "› " } else { "  " };
                lines.push(Line::from(Span::styled(
                    format!("{mark}{label}"),
                    mcp_row_style(selected == idx, theme),
                )));
            }
        }
        SkillsPane::Args { name, buf } => {
            lines.push(Line::from(Span::styled(
                format!("/{name} args   enter · run   esc · back"),
                dim,
            )));
            lines.push(Line::from(Span::styled(
                format!(" args: {buf}"),
                mcp_row_style(true, theme),
            )));
            cursor = Some((" args: ".chars().count() + buf.chars().count()) as u16);
        }
        SkillsPane::AddSkillName { buf } => {
            lines.push(Line::from(Span::styled(
                "new skill name   enter · next   esc · back",
                dim,
            )));
            lines.push(Line::from(Span::styled(
                format!(" name: {buf}"),
                mcp_row_style(true, theme),
            )));
            cursor = Some((" name: ".chars().count() + buf.chars().count()) as u16);
        }
        SkillsPane::AddSkillDesc { name, buf } => {
            lines.push(Line::from(Span::styled(
                format!("/{name} description   enter · write stub   esc · back"),
                dim,
            )));
            lines.push(Line::from(Span::styled(
                format!(" desc: {buf}"),
                mcp_row_style(true, theme),
            )));
            cursor = Some((" desc: ".chars().count() + buf.chars().count()) as u16);
        }
        SkillsPane::AddCmdName { buf } => {
            lines.push(Line::from(Span::styled(
                "new command name   enter · write stub   esc · back",
                dim,
            )));
            lines.push(Line::from(Span::styled(
                format!(" name: {buf}"),
                mcp_row_style(true, theme),
            )));
            cursor = Some((" name: ".chars().count() + buf.chars().count()) as u16);
        }
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.composer_bg).fg(theme.fg)),
        area,
    );
    if let Some(col) = cursor {
        paint_block_cursor(
            frame,
            area.x + col.min(area.width.saturating_sub(1)),
            area.y + 1,
            theme,
        );
    }
}

fn draw_hooks_overlay(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    selected: usize,
    pane: &HooksPane,
    theme: Theme,
) {
    let dim = Style::default().fg(theme.dim).bg(theme.composer_bg);
    let mut lines: Vec<Line> = Vec::new();
    let mut cursor: Option<u16> = None;
    match pane {
        HooksPane::Home => {
            lines.push(Line::from(Span::styled(
                "hooks   enter · select   backspace · remove   esc · close",
                dim,
            )));
            if view.hooks.is_empty() {
                lines.push(Line::from(Span::styled("  none configured", dim)));
            }
            for (i, h) in view.hooks.iter().enumerate() {
                let mark = if i == selected { "› " } else { "  " };
                let target = h.command.as_deref().or(h.url.as_deref()).unwrap_or("?");
                lines.push(Line::from(Span::styled(
                    format!("{mark}{}  {target}", h.event),
                    mcp_row_style(i == selected, theme),
                )));
                if let Some(m) = &h.matcher {
                    lines.push(Line::from(Span::styled(format!("    matcher {m}"), dim)));
                }
            }
            let add_i = view.hooks.len();
            let mark = if selected == add_i { "› " } else { "  " };
            lines.push(Line::from(Span::styled(
                format!("{mark}add hook"),
                mcp_row_style(selected == add_i, theme),
            )));
        }
        HooksPane::AddEvent => {
            lines.push(Line::from(Span::styled(
                "hook event   enter · next   esc · back",
                dim,
            )));
            for (i, ev) in ryter_core::HookEvent::all().iter().enumerate() {
                let mark = if i == selected { "› " } else { "  " };
                lines.push(Line::from(Span::styled(
                    format!("{mark}{}", ev.as_str()),
                    mcp_row_style(i == selected, theme),
                )));
            }
        }
        HooksPane::AddTarget { event, buf } => {
            lines.push(Line::from(Span::styled(
                format!("{event} command or url   enter · next   esc · back"),
                dim,
            )));
            lines.push(Line::from(Span::styled(
                format!(" target: {buf}"),
                mcp_row_style(true, theme),
            )));
            cursor = Some((" target: ".chars().count() + buf.chars().count()) as u16);
        }
        HooksPane::AddMatcher { event, buf, .. } => {
            lines.push(Line::from(Span::styled(
                format!("{event} matcher (optional glob)   enter · save   esc · back"),
                dim,
            )));
            lines.push(Line::from(Span::styled(
                format!(" matcher: {buf}"),
                mcp_row_style(true, theme),
            )));
            cursor = Some((" matcher: ".chars().count() + buf.chars().count()) as u16);
        }
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.composer_bg).fg(theme.fg)),
        area,
    );
    if let Some(col) = cursor {
        paint_block_cursor(
            frame,
            area.x + col.min(area.width.saturating_sub(1)),
            area.y + 1,
            theme,
        );
    }
}

fn draw_permission_overlay(frame: &mut Frame, area: Rect, tool: &str, summary: &str, theme: Theme) {
    let dim = Style::default().fg(theme.dim).bg(theme.composer_bg);
    let lines = vec![
        Line::from(Span::styled(
            "permission   y · allow   n · deny   a · always   esc · deny",
            dim,
        )),
        Line::from(Span::styled(format!(" {tool}"), mcp_row_style(true, theme))),
        Line::from(Span::styled(format!(" {summary}"), dim)),
    ];
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.composer_bg).fg(theme.fg)),
        area,
    );
}

fn draw_ask_user_overlay(
    frame: &mut Frame,
    area: Rect,
    question: &str,
    options: &[String],
    selected: usize,
    buf: &str,
    theme: Theme,
) {
    let dim = Style::default().fg(theme.dim).bg(theme.composer_bg);
    let mut lines = vec![Line::from(Span::styled(
        if options.is_empty() {
            "ask   type an answer   enter · send   esc · skip"
        } else {
            "ask   enter · choose   esc · skip"
        },
        dim,
    ))];
    lines.push(Line::from(Span::styled(
        format!(" {question}"),
        mcp_row_style(false, theme),
    )));
    if options.is_empty() {
        lines.push(Line::from(Span::styled(
            format!(" answer: {buf}"),
            mcp_row_style(true, theme),
        )));
    } else {
        for (i, o) in options.iter().enumerate() {
            let mark = if i == selected { "› " } else { "  " };
            lines.push(Line::from(Span::styled(
                format!("{mark}{o}"),
                mcp_row_style(i == selected, theme),
            )));
        }
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.composer_bg).fg(theme.fg)),
        area,
    );
    if options.is_empty() {
        let col = (" answer: ".chars().count() + buf.chars().count()) as u16;
        paint_block_cursor(
            frame,
            area.x + col.min(area.width.saturating_sub(1)),
            area.y + 2,
            theme,
        );
    }
}

fn draw_spend_overlay(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    let dim = Style::default().fg(theme.dim).bg(theme.composer_bg);
    let mut lines = vec![Line::from(Span::styled("spend   esc · close", dim))];
    lines.push(Line::from(Span::styled(
        format!(" session  {}", view.spend_label()),
        mcp_row_style(false, theme),
    )));
    if view.spend_by_role.is_empty() && view.spend_by_conn.is_empty() {
        lines.push(Line::from(Span::styled("  no priced turns yet", dim)));
    } else {
        lines.push(Line::from(Span::styled(" by role", dim)));
        for (k, v) in &view.spend_by_role {
            lines.push(Line::from(Span::styled(
                format!("  {k:12}  {}", ryter_core::format_usd(Some(*v))),
                mcp_row_style(false, theme),
            )));
        }
        lines.push(Line::from(Span::styled(" by provider", dim)));
        for (k, v) in &view.spend_by_conn {
            lines.push(Line::from(Span::styled(
                format!("  {k:12}  {}", ryter_core::format_usd(Some(*v))),
                mcp_row_style(false, theme),
            )));
        }
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.composer_bg).fg(theme.fg)),
        area,
    );
}

fn draw_settings_overlay(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    selected: usize,
    edit: Option<&str>,
    theme: Theme,
) {
    let dim = Style::default().fg(theme.dim).bg(theme.composer_bg);
    let rows = [
        format!("budget    ${:.2}  (0 = none)", view.budget_usd),
        format!("warn      ${:.2}", view.warn_usd),
        format!("max       {}", view.max_crew),
        format!("sandbox   {}", view.sandbox_profile),
        format!("inbound   {}", if view.mcp_inbound { "on" } else { "off" }),
        format!("web       {}", if view.web { "on" } else { "off" }),
    ];
    let mut lines = vec![Line::from(Span::styled(
        "settings   enter · edit   esc · close",
        dim,
    ))];
    for (i, row) in rows.iter().enumerate() {
        let mark = if i == selected { "› " } else { "  " };
        lines.push(Line::from(Span::styled(
            format!("{mark}{row}"),
            mcp_row_style(i == selected, theme),
        )));
    }
    if let Some(buf) = edit {
        lines.push(Line::from(Span::styled(
            format!(" value: {buf}"),
            mcp_row_style(true, theme),
        )));
    }
    let cursor_row = lines.len().saturating_sub(1) as u16;
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.composer_bg).fg(theme.fg)),
        area,
    );
    if let Some(buf) = edit {
        let col = (" value: ".chars().count() + buf.chars().count()) as u16;
        paint_block_cursor(
            frame,
            area.x + col.min(area.width.saturating_sub(1)),
            area.y + cursor_row,
            theme,
        );
    }
}

fn draw_agents_overlay(frame: &mut Frame, area: Rect, view: &View, selected: usize, theme: Theme) {
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "agents   enter · kill   esc · close",
        Style::default().fg(theme.dim).bg(theme.composer_bg),
    ))];
    if view.crew.is_empty() {
        lines.push(Line::from(Span::styled(
            "  none running",
            Style::default().fg(theme.dim).bg(theme.composer_bg),
        )));
    } else {
        for (i, c) in view.crew.iter().enumerate() {
            let mark = if i == selected { "› " } else { "  " };
            let style = if i == selected {
                Style::default()
                    .fg(theme.prompt)
                    .bg(theme.composer_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg).bg(theme.composer_bg)
            };
            lines.push(Line::from(Span::styled(
                format!("{mark}{}  {}  {}", c.role, c.label, c.status),
                style,
            )));
        }
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.composer_bg).fg(theme.fg)),
        area,
    );
}

fn draw_crew_overlay(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    selected: usize,
    save_buf: Option<&str>,
    theme: Theme,
) {
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "crew   enter · assign model   esc · close",
        Style::default().fg(theme.dim).bg(theme.composer_bg),
    ))];
    for (i, role) in crate::view::CREW_ROLES.iter().enumerate() {
        let mark = if i == selected { "› " } else { "  " };
        let style = if i == selected {
            Style::default()
                .fg(theme.prompt)
                .bg(theme.composer_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg).bg(theme.composer_bg)
        };
        let label = crate::view::crew_role_label(view, role);
        lines.push(Line::from(Span::styled(format!("{mark}{role}"), style)));
        lines.push(Line::from(Span::styled(
            format!("    {label}"),
            Style::default().fg(theme.dim).bg(theme.composer_bg),
        )));
    }
    let save_i = crate::view::CREW_ROLES.len();
    let load_i = save_i + 1;
    lines.push(Line::from(""));
    for (i, label) in [(save_i, "save preset"), (load_i, "load preset")] {
        let mark = if i == selected { "› " } else { "  " };
        let style = if i == selected {
            Style::default()
                .fg(theme.prompt)
                .bg(theme.composer_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg).bg(theme.composer_bg)
        };
        lines.push(Line::from(Span::styled(format!("{mark}{label}"), style)));
    }
    if let Some(buf) = save_buf {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!(" name: {buf}"),
            Style::default().fg(theme.prompt).bg(theme.composer_bg),
        )));
    }
    let cursor_y = save_buf.map(|_| (lines.len().saturating_sub(1)) as u16);
    let cursor_col = save_buf.map(|buf| (" name: ".chars().count() + buf.chars().count()) as u16);
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.composer_bg).fg(theme.fg)),
        area,
    );
    if let (Some(y), Some(col)) = (cursor_y, cursor_col) {
        paint_block_cursor(
            frame,
            area.x + col.min(area.width.saturating_sub(1)),
            area.y + y.min(area.height.saturating_sub(1)),
            theme,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_model_overlay(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    filter: &str,
    selected: usize,
    loading: bool,
    assign_role: Option<&str>,
    theme: Theme,
) {
    let items = view.filtered_models();
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        match assign_role {
            Some(role) => format!("{role} model   type to filter   enter · select   esc · back"),
            None => "model   type to filter   enter · select   esc · close".into(),
        },
        Style::default().fg(theme.dim).bg(theme.composer_bg),
    ))];
    lines.push(Line::from(vec![
        Span::styled(
            " filter: ",
            Style::default().fg(theme.dim).bg(theme.composer_bg),
        ),
        Span::styled(
            filter.to_string(),
            Style::default().fg(theme.prompt).bg(theme.composer_bg),
        ),
    ]));
    if loading && items.is_empty() {
        lines.push(Line::from(Span::styled(
            " loading…",
            Style::default().fg(theme.dim).bg(theme.composer_bg),
        )));
    } else if items.is_empty() {
        lines.push(Line::from(Span::styled(
            " no matches",
            Style::default().fg(theme.dim).bg(theme.composer_bg),
        )));
    } else {
        let max = area.height.saturating_sub(3) as usize;
        let start = selected.saturating_sub(max.saturating_sub(1));
        for (i, m) in items.iter().enumerate().skip(start).take(max) {
            let mark = if i == selected { "› " } else { "  " };
            let style = if i == selected {
                Style::default()
                    .fg(theme.prompt)
                    .bg(theme.composer_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg).bg(theme.composer_bg)
            };
            if m.id.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("{mark}default ({})", short_model(&view.model)),
                    style,
                )));
                continue;
            }
            let win = m
                .context_length
                .map(ryter_core::format_tokens)
                .unwrap_or_default();
            let price = match (m.input_per_million, m.output_per_million) {
                (Some(i), Some(o)) => format!("  {}", ryter_core::format_rates_short(i, o)),
                _ => String::new(),
            };
            let on = if m.id == view.model { "  on" } else { "" };
            let prov = m
                .connection
                .as_deref()
                .map(|c| format!("  {c}"))
                .unwrap_or_default();
            lines.push(Line::from(Span::styled(
                format!("{mark}{}  {win}{price}{prov}{on}", short_model(&m.id)),
                style,
            )));
        }
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.composer_bg).fg(theme.fg)),
        area,
    );
    let col = (" filter: ".chars().count() + filter.chars().count()) as u16;
    paint_block_cursor(
        frame,
        area.x + col.min(area.width.saturating_sub(1)),
        area.y + 1,
        theme,
    );
}

fn draw_choice_overlay(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    title: &str,
    selected: usize,
    theme: Theme,
) {
    let rows_list = view.filtered_choices();
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        format!("{title}   enter · select   esc · close"),
        Style::default().fg(theme.dim).bg(theme.composer_bg),
    ))];
    if rows_list.is_empty() {
        lines.push(Line::from(Span::styled(
            " no matches",
            Style::default().fg(theme.dim).bg(theme.composer_bg),
        )));
    } else {
        let max = area.height.saturating_sub(2) as usize;
        let start = selected.saturating_sub(max.saturating_sub(1));
        for (i, c) in rows_list.iter().enumerate().skip(start).take(max) {
            let mark = if i == selected { "› " } else { "  " };
            let style = if i == selected {
                Style::default()
                    .fg(theme.prompt)
                    .bg(theme.composer_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg).bg(theme.composer_bg)
            };
            lines.push(Line::from(Span::styled(
                format!("{mark}{}", c.label),
                style,
            )));
        }
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.composer_bg).fg(theme.fg)),
        area,
    );
}

fn draw_slash(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    let Some(slash) = &view.slash else { return };
    fill(frame, area, theme.bg, theme.fg);
    let vis = area.height as usize;
    let start = slash.selected.saturating_sub(vis.saturating_sub(1));
    let rows: Vec<Line> = slash
        .matches
        .iter()
        .enumerate()
        .skip(start)
        .take(vis)
        .map(|(i, name)| {
            let selected = i == slash.selected;
            let style = if selected {
                Style::default()
                    .fg(theme.prompt)
                    .bg(theme.bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                theme.muted()
            };
            let mark = if selected { "› " } else { "  " };
            Line::from(Span::styled(format!("{mark}/{name}"), style))
        })
        .collect();
    frame.render_widget(Paragraph::new(rows).style(theme.body()), area);
}

fn draw_ask(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    fill(frame, area, theme.bg, theme.fg);
    let msg = if let Some(name) = &view.secret_for {
        format!(" paste API key for {name}  · Enter save · Esc cancel")
    } else if let Some(msg) = &view.permission {
        format!(" {msg}  y/n/a")
    } else {
        return;
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            msg,
            Style::default().fg(theme.warn).bg(theme.bg),
        ))),
        area,
    );
}

fn draw_composer(frame: &mut Frame, area: Rect, view: &View, theme: Theme, overlay_open: bool) {
    fill(frame, area, theme.composer_bg, theme.fg);
    let (prefix, text) = if view.secret_for.is_some() && overlay_open {
        (" key ", String::new())
    } else if view.secret_for.is_some() {
        let n = view.secret_buf.chars().count();
        (" key ", "•".repeat(n))
    } else if view.handoff_to.is_some() {
        (" → ", view.composer.clone())
    } else {
        (" › ", view.composer.clone())
    };
    let style = theme.compose();
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                prefix,
                Style::default().fg(theme.prompt).bg(theme.composer_bg),
            ),
            Span::styled(text, style),
        ]))
        .style(theme.compose()),
        area,
    );
    if overlay_open {
        return;
    }
    // Cursor sits after the prompt, not after the placeholder hint.
    let typed = if view.secret_for.is_some() {
        view.secret_buf.chars().count()
    } else {
        view.composer.chars().count()
    };
    let col = (prefix.chars().count() + typed) as u16;
    paint_block_cursor(
        frame,
        area.x + col.min(area.width.saturating_sub(1)),
        area.y,
        theme,
    );
}

/// Wide white block. Native terminal cursor is hidden so it cannot invert to black.
fn paint_block_cursor(frame: &mut Frame, x: u16, y: u16, theme: Theme) {
    let buf = frame.buffer_mut();
    if x >= buf.area.x + buf.area.width || y >= buf.area.y + buf.area.height {
        return;
    }
    let cell = &mut buf[(x, y)];
    cell.set_symbol("█");
    cell.set_style(
        Style::default()
            .fg(theme.prompt)
            .bg(theme.composer_bg)
            .add_modifier(Modifier::BOLD),
    );
}

fn draw_footer(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    let handoff = view
        .handoff_to
        .map(|p| format!("  handoff → {p}"))
        .unwrap_or_default();
    let secret = if view.secret_for.is_some() {
        "  set-key"
    } else {
        ""
    };
    let busy = if view.busy { "  esc · cancel" } else { "" };
    let line = format!(" {handoff}{secret}{busy}");
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(line, theme.muted()))).style(theme.body()),
        area,
    );
}

/// Render `view` to a plain string (tests).
pub fn render_to_string(view: &View, width: u16, height: u16) -> String {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|f| draw(f, view, Theme::truecolor_dark()))
        .expect("draw");
    buffer_to_string(terminal.backend().buffer())
}

fn buffer_to_string(buf: &ratatui::buffer::Buffer) -> String {
    let mut out = String::new();
    for y in 0..buf.area.height {
        let mut line = String::new();
        for x in 0..buf.area.width {
            line.push_str(buf[(x, y)].symbol());
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}
