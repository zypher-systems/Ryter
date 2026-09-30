//! The solo screen's side rail (design S2): the name, the session, the hat
//! in its color, the model and its context, what it has cost, and what the
//! last turn changed. The ledger had most of it in one bar at the bottom,
//! the hat a small chip at its left end, and no name anywhere.

use std::collections::BTreeMap;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ryter_core::Role;

use crate::chat::{MessageKind, turn_usd, wrap};
use crate::theme::Theme;
use crate::view::View;

/// Columns the rail takes, its right edge included.
pub const RAIL_W: u16 = 34;
/// Narrowest screen that keeps the rail: the conversation needs about
/// seventy-six columns beside it.
pub const RAIL_MIN_SCREEN: u16 = 110;

/// Whether the rail shows: solo mode on the ledger, the conversation on
/// screen, wide enough, and not hidden with `^b`.
pub fn shown(view: &View, width: u16) -> bool {
    !view.ui.classic()
        && !view.crew_mode()
        && view.workbench.is_none()
        && view.panel_visible
        && width >= RAIL_MIN_SCREEN
}

/// What each hat does, in a line.
fn hat_words(mode: Role) -> (&'static str, &'static str, &'static str) {
    match mode {
        Role::SoloPlan => ("PLAN", "reads and designs only", "build · review"),
        Role::SoloReview => ("REVIEW", "reads the changes, reports", "build · plan"),
        _ => ("BUILD", "edits files, runs commands", "plan · review"),
    }
}

/// Paint the rail into `area` (its full height, right edge included).
pub fn draw(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    let bg = theme.sidebar_bg;
    let edge = Rect {
        x: area.x + area.width.saturating_sub(1),
        width: 1,
        ..area
    };
    let body = Rect {
        x: area.x + 2,
        y: area.y + 1,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(2),
    };
    frame.render_widget(
        Paragraph::new("").style(Style::default().bg(bg)),
        Rect {
            width: area.width.saturating_sub(1),
            ..area
        },
    );
    frame.render_widget(
        Paragraph::new(
            (0..edge.height)
                .map(|_| {
                    Line::from(Span::styled(
                        "│",
                        Style::default().fg(theme.gauge_track).bg(theme.bg),
                    ))
                })
                .collect::<Vec<_>>(),
        ),
        edge,
    );
    let w = usize::from(body.width);
    let foot = views_line(view, theme, bg);
    // Sections in the order they read; when the screen is short, the last
    // ones go first, but the name, the hat and the spend stay.
    let sections = [
        (0, name(view, theme, bg, w)),
        (3, session(view, theme, bg, w)),
        (0, hat(view, theme, bg, w)),
        (2, model(view, theme, bg, w)),
        (1, spend(view, theme, bg, w)),
        (4, changed(view, theme, bg, w)),
    ];
    let room = usize::from(body.height).saturating_sub(2);
    let mut keep: Vec<bool> = vec![true; sections.len()];
    let height = |keep: &[bool]| -> usize {
        sections
            .iter()
            .zip(keep)
            .filter(|(_, k)| **k)
            .map(|((_, s), _)| s.len() + 1)
            .sum()
    };
    for drop in [4, 3, 2, 1] {
        if height(&keep) <= room {
            break;
        }
        for (i, (rank, _)) in sections.iter().enumerate() {
            if *rank == drop {
                keep[i] = false;
            }
        }
    }
    let mut lines: Vec<Line<'static>> = Vec::new();
    for ((_, s), k) in sections.into_iter().zip(&keep) {
        if *k {
            lines.extend(s);
            lines.push(Line::from(""));
        }
    }
    lines.truncate(room);
    while lines.len() < usize::from(body.height).saturating_sub(1) {
        lines.push(Line::from(""));
    }
    lines.push(foot);
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(theme.fg).bg(bg)),
        body,
    );
}

fn heading(label: &str, theme: Theme, bg: Color) -> Line<'static> {
    Line::from(Span::styled(
        label.to_string(),
        Style::default()
            .fg(theme.dim)
            .bg(bg)
            .add_modifier(Modifier::BOLD),
    ))
}

/// The end of `s` in at most `max` columns, `…` where it was cut: the
/// folder's own name is the part worth keeping.
pub(crate) fn tail(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    format!("…{}", s.chars().skip(n - keep).collect::<String>())
}

/// `label` then `value`, the value at a fixed column.
fn row(label: &str, value: Vec<Span<'static>>, theme: Theme, bg: Color) -> Line<'static> {
    let mut v = vec![Span::styled(
        format!("{label:<12}"),
        Style::default().fg(theme.dim).bg(bg),
    )];
    v.extend(value);
    Line::from(v)
}

fn name(view: &View, theme: Theme, bg: Color, w: usize) -> Vec<Line<'static>> {
    let project = std::path::Path::new(&view.cwd)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| view.cwd.clone());
    let place = match &view.git_branch {
        Some(b) => format!("{project} · {b}"),
        None => project,
    };
    vec![
        Line::from(Span::styled(
            "R Y T E R",
            Style::default()
                .fg(theme.fg)
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            wrap::truncate(&place, w),
            Style::default().fg(theme.dim).bg(bg),
        )),
    ]
}

fn session(view: &View, theme: Theme, bg: Color, w: usize) -> Vec<Line<'static>> {
    let title = view.session_title.trim();
    let title = if title.is_empty() {
        "new session"
    } else {
        title
    };
    let mut lines = vec![heading("SESSION", theme, bg)];
    let mut wrapped = wrap::wrap_plain(title, w);
    if wrapped.len() > 3 {
        wrapped.truncate(3);
        let last = wrapped[2].clone();
        wrapped[2] = wrap::truncate(&format!("{last}…"), w);
    }
    for l in wrapped {
        lines.push(Line::from(Span::styled(
            l,
            Style::default()
                .fg(theme.fg)
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        )));
    }
    let turns = view
        .messages
        .iter()
        .filter(|m| matches!(m.kind, MessageKind::User))
        .count();
    let since = view
        .messages
        .first()
        .map(|m| format!("since {} · ", m.at.hhmm()))
        .unwrap_or_default();
    lines.push(Line::from(Span::styled(
        format!("{since}{turns} turn{}", if turns == 1 { "" } else { "s" }),
        Style::default().fg(theme.dim).bg(bg),
    )));
    lines
}

/// The hat, as a block in its color: the thing to find at a glance.
fn hat(view: &View, theme: Theme, bg: Color, w: usize) -> Vec<Line<'static>> {
    let (label, does, others) = hat_words(view.mode);
    let color = theme.mode(view.mode);
    let block = |text: String, bold: bool| {
        let mut style = Style::default().fg(theme.bg).bg(color);
        if bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        Line::from(Span::styled(wrap::pad_right(&text, w), style))
    };
    vec![
        block(format!(" {label}"), true),
        block(
            format!(" {}", wrap::truncate(does, w.saturating_sub(1))),
            false,
        ),
        Line::from(vec![
            Span::styled(
                format!("{others}   "),
                Style::default().fg(theme.dim).bg(bg),
            ),
            Span::styled(
                "tab",
                Style::default()
                    .fg(theme.accent)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" switch", Style::default().fg(theme.dim).bg(bg)),
        ]),
    ]
}

/// `24k`, `1.2M`.
fn short_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1000 {
        format!("{}k", n / 1000)
    } else {
        n.to_string()
    }
}

fn model(view: &View, theme: Theme, bg: Color, w: usize) -> Vec<Line<'static>> {
    let dim = Style::default().fg(theme.dim).bg(bg);
    let name = crate::chat::short_model(&view.model).to_string();
    let reasoning = format!(" · reasoning {}", view.reasoning_label(&view.model));
    let name = wrap::truncate(&name, w);
    let reasoning = wrap::truncate(&reasoning, w.saturating_sub(wrap::width(&name)));
    let frac = view.ctx_frac();
    let pct = format!(" {}%", (frac * 100.0).round() as u32);
    let cells = w.saturating_sub(4 + pct.len());
    let filled = ((frac * cells as f64).round() as usize).min(cells);
    let window = view.ctx_window_or_default();
    vec![
        heading("MODEL", theme, bg),
        Line::from(vec![
            Span::styled(name, Style::default().fg(theme.fg).bg(bg)),
            Span::styled(reasoning, dim),
        ]),
        Line::from(vec![
            Span::styled("ctx ", dim),
            Span::styled(
                "▰".repeat(filled),
                Style::default()
                    .fg(crate::panel::widgets::gauge_color(frac, theme))
                    .bg(bg),
            ),
            Span::styled("▱".repeat(cells - filled), dim),
            Span::styled(pct, dim),
        ]),
        Line::from(Span::styled(
            format!(
                "{} of {} tokens",
                short_count(view.ctx_tokens.unwrap_or(0)),
                short_count(window)
            ),
            dim,
        )),
    ]
}

/// What each turn cost, oldest first.
fn turn_costs(view: &View) -> Vec<f64> {
    let mut by_turn: BTreeMap<u64, f64> = BTreeMap::new();
    for m in &view.messages {
        if let Some(c) = m.meta.cost {
            *by_turn.entry(m.turn).or_insert(0.0) += c;
        }
    }
    by_turn.into_values().collect()
}

fn spend(view: &View, theme: Theme, bg: Color, w: usize) -> Vec<Line<'static>> {
    let dim = Style::default().fg(theme.dim).bg(bg);
    let strong = |fg: Color| Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD);
    let turn = view
        .spend
        .map(|now| now - view.turn_spend_from.unwrap_or(0.0))
        .map_or_else(|| "$0".into(), turn_usd);
    let session = match view.spend {
        None if !view.spend_unknown => "$0".to_string(),
        Some(s) if !view.spend_unknown => turn_usd(s),
        _ => view.spend_label(),
    };
    // The session's cost turns yellow at the warning and red at the budget,
    // as the bottom bar's did.
    let session_color = match view.spend {
        Some(s) if view.budget_usd > 0.0 && s >= view.budget_usd => theme.error,
        Some(s) if view.warn_usd > 0.0 && s >= view.warn_usd => theme.warn,
        _ => theme.fg,
    };
    let mut lines = vec![
        heading("SPEND", theme, bg),
        row(
            "this turn",
            vec![Span::styled(turn, strong(theme.fg))],
            theme,
            bg,
        ),
        row(
            "session",
            vec![Span::styled(session, strong(session_color))],
            theme,
            bg,
        ),
    ];
    if let Some(p) = &view.project_spend {
        let known = turn_usd(p.total_usd);
        let label = if p.unpriced_calls > 0 {
            format!("≥{known}")
        } else {
            known
        };
        lines.push(row(
            "project",
            vec![Span::styled(label, strong(theme.fg))],
            theme,
            bg,
        ));
        // Counted for a repository around this folder: say which.
        if let Some(root) = &view.project_root {
            let room = w.saturating_sub(12 + 3);
            lines.push(row(
                "",
                vec![Span::styled(format!("in {}", tail(root, room)), dim)],
                theme,
                bg,
            ));
        }
    }
    let budget = if view.budget_usd > 0.0 {
        format!("${:.2}", view.budget_usd)
    } else {
        "off".into()
    };
    lines.push(row("budget", vec![Span::styled(budget, dim)], theme, bg));
    let costs = turn_costs(view);
    if costs.len() > 1 {
        const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let cells = w.saturating_sub(12);
        let shown = &costs[costs.len().saturating_sub(cells)..];
        let top = shown.iter().copied().fold(0.0_f64, f64::max).max(1e-9);
        let bars: String = shown
            .iter()
            .map(|c| BARS[((c / top) * 7.0).round() as usize])
            .collect();
        lines.push(row(
            "by turn",
            vec![Span::styled(
                bars,
                Style::default().fg(theme.success).bg(bg),
            )],
            theme,
            bg,
        ));
    }
    lines
}

/// What the last turn that changed files changed, a file a line.
fn changed(view: &View, theme: Theme, bg: Color, w: usize) -> Vec<Line<'static>> {
    let dim = Style::default().fg(theme.dim).bg(bg);
    let last = view
        .messages
        .iter()
        .filter(|m| m.meta.diff.is_some())
        .map(|m| m.turn)
        .max();
    let mut files: Vec<(String, usize, usize)> = Vec::new();
    for m in view.messages.iter().filter(|m| Some(m.turn) == last) {
        if let Some(d) = &m.meta.diff {
            match files.iter_mut().find(|(p, ..)| *p == d.path) {
                Some(f) => {
                    f.1 += d.added;
                    f.2 += d.removed;
                }
                None => files.push((d.path.clone(), d.added, d.removed)),
            }
        }
    }
    let mut lines = vec![heading("CHANGED", theme, bg)];
    if files.is_empty() {
        lines.push(Line::from(Span::styled("nothing yet", dim)));
        return lines;
    }
    const SHOWN: usize = 4;
    for (path, added, removed) in files.iter().take(SHOWN) {
        let counts = format!("+{added} −{removed}");
        let room = w.saturating_sub(wrap::width(&counts) + 1);
        let shown = wrap::truncate(path, room);
        let pad = w.saturating_sub(wrap::width(&shown) + wrap::width(&counts));
        lines.push(Line::from(vec![
            Span::styled(shown, Style::default().fg(theme.fg).bg(bg)),
            Span::styled(" ".repeat(pad), dim),
            Span::styled(
                format!("+{added}"),
                Style::default().fg(theme.success).bg(bg),
            ),
            Span::styled(
                format!(" −{removed}"),
                Style::default().fg(theme.error).bg(bg),
            ),
        ]));
    }
    if files.len() > SHOWN {
        lines.push(Line::from(Span::styled(
            format!("and {} more", files.len() - SHOWN),
            dim,
        )));
    }
    let key = Style::default()
        .fg(theme.accent)
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    lines.push(Line::from(vec![
        Span::styled("/undo", key),
        Span::styled("  ", dim),
        Span::styled("^t", key),
        Span::styled(" all changes", dim),
    ]));
    lines
}

/// The views there are, the one on screen lit: the ledger's strip, moved.
fn views_line(view: &View, theme: Theme, bg: Color) -> Line<'static> {
    let _ = view;
    let on = Style::default()
        .fg(theme.fg)
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    let off = Style::default().fg(theme.dim).bg(bg);
    Line::from(vec![
        Span::styled("chat", on),
        Span::styled("  changes ^t  crew /crew", off),
    ])
}
