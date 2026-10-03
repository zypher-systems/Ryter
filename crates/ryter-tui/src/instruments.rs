//! The instruments (`docs/hat-rack-design.md` §6): the solo screen's right
//! column. What is true of the session as a whole: the model in use, how
//! full its context is, what the session and the project have cost, what
//! the hat may do, and what is uncommitted. What each hat did is the
//! rack's.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ryter_core::Role;
use ryter_core::review::Status;

use crate::chat::{short_model, turn_usd, wrap};
use crate::rail::{ends, hat_model, heading, tail, vline};
use crate::theme::Theme;
use crate::view::View;

/// Columns the instruments take beside the rack, the hairline on their
/// left included.
pub const WIDE_W: u16 = 34;
/// Columns they take on a screen too narrow for the rack.
pub const MID_W: u16 = 30;
/// Changed files listed before `+N more`.
const FILES_SHOWN: usize = 6;

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

/// What the hat on may do, in two or three words.
pub fn hat_may(hat: Role, condensed: bool) -> &'static str {
    match hat {
        Role::SoloPlan => "read only",
        // What the audit changes, a checkpoint puts back.
        Role::SoloAudit if condensed => "checkpoint",
        Role::SoloAudit => "checkpoint, restored",
        Role::SoloBuild | Role::Crew if condensed => "asks first",
        Role::SoloBuild | Role::Crew => "edits ask first",
    }
}

/// The context gauge's color: the hat's, until it is full enough to warn.
pub fn gauge_color(view: &View, theme: Theme) -> Color {
    let c = crate::panel::widgets::gauge_color(view.ctx_frac(), theme);
    if c == theme.success {
        theme.mode(view.mode)
    } else {
        c
    }
}

/// The session's cost and the color it is said in: yellow at the warning,
/// red at the budget.
pub fn session_spend(view: &View, theme: Theme) -> (String, Color) {
    let text = match view.spend {
        None if !view.spend_unknown => "$0".to_string(),
        Some(s) if !view.spend_unknown => turn_usd(s),
        _ => view.spend_label(),
    };
    let color = match view.spend {
        Some(s) if view.budget_usd > 0.0 && s >= view.budget_usd => theme.error,
        Some(s) if view.warn_usd > 0.0 && s >= view.warn_usd => theme.warn,
        _ => theme.fg,
    };
    (text, color)
}

/// `off`, or the cap.
pub fn budget(view: &View) -> String {
    if view.budget_usd > 0.0 {
        format!("${:.2}", view.budget_usd)
    } else {
        "off".into()
    }
}

/// The connection the hat's model is reached through, and whether it has
/// a key to reach it with.
fn connection(view: &View) -> (String, bool) {
    let name = view
        .specialists
        .get(view.mode.as_str())
        .filter(|r| r.is_override())
        .and_then(|r| r.connection.clone())
        .unwrap_or_else(|| view.connection.clone());
    let keyed = view
        .connections
        .iter()
        .find(|c| c.name == name)
        .map_or(view.has_key, |c| c.has_key);
    (name, keyed)
}

fn model(view: &View, theme: Theme, bg: Color, w: usize, condensed: bool) -> Vec<Line<'static>> {
    let dim = Style::default().fg(theme.dim).bg(bg);
    let body = Style::default().fg(theme.fg).bg(bg);
    let hat = Style::default().fg(theme.mode(view.mode)).bg(bg);
    let name = short_model(hat_model(view, view.mode)).to_string();
    if condensed {
        let label = view.mode_label();
        let room = w.saturating_sub(wrap::width(label) + 2);
        return vec![
            Line::from(heading("MODEL", theme, bg)),
            ends(
                vec![Span::styled(label, body)],
                vec![Span::styled(wrap::truncate(&name, room), hat)],
                w,
                bg,
            ),
        ];
    }
    let (conn, keyed) = connection(view);
    let dot = Style::default()
        .fg(if keyed { theme.success } else { theme.error })
        .bg(bg);
    vec![
        Line::from(heading("MODEL", theme, bg)),
        Line::from(Span::styled(wrap::truncate(&name, w), hat)),
        ends(
            vec![Span::styled("connection", body)],
            vec![
                Span::styled(
                    format!("{} ", wrap::truncate(&conn, w.saturating_sub(13))),
                    if keyed { body } else { dim },
                ),
                Span::styled(if keyed { "●" } else { "○" }, dot),
            ],
            w,
            bg,
        ),
        ends(
            vec![Span::styled("reasoning", body)],
            vec![Span::styled(
                view.reasoning_label(hat_model(view, view.mode)),
                body,
            )],
            w,
            bg,
        ),
    ]
}

fn context(view: &View, theme: Theme, bg: Color, w: usize) -> Vec<Line<'static>> {
    let dim = Style::default().fg(theme.dim).bg(bg);
    let frac = view.ctx_frac();
    let pct = format!(" {}%", (frac * 100.0).round() as u32);
    let cells = w.saturating_sub(wrap::width(&pct));
    let filled = ((frac * cells as f64).round() as usize).min(cells);
    vec![
        ends(vec![heading("CONTEXT", theme, bg)], Vec::new(), w, bg),
        Line::from(vec![
            Span::styled(
                "━".repeat(filled),
                Style::default().fg(gauge_color(view, theme)).bg(bg),
            ),
            Span::styled(
                "─".repeat(cells - filled),
                Style::default().fg(theme.rule).bg(bg),
            ),
            Span::styled(pct, Style::default().fg(theme.fg).bg(bg)),
        ]),
        Line::from(Span::styled(
            format!(
                "{} / {} tokens",
                short_count(view.ctx_tokens.unwrap_or(0)),
                short_count(view.ctx_window_or_default())
            ),
            dim,
        )),
    ]
}

/// The bars of a history, tallest for the busiest second.
fn sparkline(history: &[u64]) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let top = history.iter().copied().max().unwrap_or(0).max(1);
    history
        .iter()
        .map(|&n| {
            if n == 0 {
                BARS[0]
            } else {
                BARS[((n * 7).div_ceil(top) as usize).clamp(1, 7)]
            }
        })
        .collect()
}

/// How fast the model is writing: tokens a second now, and the last
/// eight seconds as bars. Idle between turns.
fn pulse(view: &View, theme: Theme, bg: Color, w: usize) -> Vec<Line<'static>> {
    let dim = Style::default().fg(theme.dim).bg(bg);
    let now = view.now_ms;
    let bars = sparkline(&view.pulse.history(now));
    let (bars_style, right) = match view.pulse.rate(now) {
        Some(rate) => (
            Style::default().fg(theme.mode(view.mode)).bg(bg),
            vec![
                Span::styled(rate.to_string(), Style::default().fg(theme.fg).bg(bg)),
                Span::styled(" tok/s", dim),
            ],
        ),
        None => (dim, vec![Span::styled("idle", dim)]),
    };
    vec![
        Line::from(heading("PULSE", theme, bg)),
        ends(vec![Span::styled(bars, bars_style)], right, w, bg),
    ]
}

fn spend(view: &View, theme: Theme, bg: Color, w: usize) -> Vec<Line<'static>> {
    let dim = Style::default().fg(theme.dim).bg(bg);
    let body = Style::default().fg(theme.fg).bg(bg);
    let (session, color) = session_spend(view, theme);
    let mut lines = vec![
        Line::from(heading("SPEND", theme, bg)),
        ends(
            vec![Span::styled("session", body)],
            vec![Span::styled(
                session,
                Style::default()
                    .fg(color)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            )],
            w,
            bg,
        ),
    ];
    // Across this repository's sessions, as `ryter spend --project` says.
    if let Some(p) = &view.project_spend {
        let known = turn_usd(p.total_usd);
        lines.push(ends(
            vec![Span::styled("project", body)],
            vec![Span::styled(
                if p.unpriced_calls > 0 {
                    format!("≥{known}")
                } else {
                    known
                },
                body,
            )],
            w,
            bg,
        ));
    }
    lines.push(ends(
        vec![Span::styled("budget", body)],
        vec![Span::styled(
            budget(view),
            if view.budget_usd > 0.0 { body } else { dim },
        )],
        w,
        bg,
    ));
    lines
}

fn guard(view: &View, theme: Theme, bg: Color, w: usize, condensed: bool) -> Vec<Line<'static>> {
    let body = Style::default().fg(theme.fg).bg(bg);
    let profile = if view.sandbox_profile.trim().is_empty() {
        "off"
    } else {
        view.sandbox_profile.trim()
    };
    vec![
        Line::from(heading("GUARD", theme, bg)),
        ends(
            vec![Span::styled("sandbox", body)],
            vec![Span::styled(profile.to_string(), body)],
            w,
            bg,
        ),
        ends(
            vec![Span::styled("this hat", body)],
            vec![Span::styled(
                hat_may(view.mode, condensed),
                Style::default().fg(theme.mode(view.mode)).bg(bg),
            )],
            w,
            bg,
        ),
        file_row("plan.md", view.plan_file.as_deref(), theme, bg, w),
        file_row(
            "audit.md",
            if view.audit_writing {
                Some("writing…")
            } else {
                view.audit_file.as_deref()
            },
            theme,
            bg,
            w,
        ),
    ]
}

/// A row of the guard card for a file every hat reads (`.ryter/plan.md`,
/// `.ryter/audit.md`): what it is, or `none`.
fn file_row(name: &str, label: Option<&str>, theme: Theme, bg: Color, w: usize) -> Line<'static> {
    let body = Style::default().fg(theme.fg).bg(bg);
    let dim = Style::default().fg(theme.dim).bg(bg);
    let room = w.saturating_sub(name.len() + 1);
    // Short of room, what the file is matters more than its day.
    let fit = |l: &str| -> String {
        if crate::chat::wrap::width(l) > room {
            if let Some((day, rest)) = l.split_once(' ') {
                if day.len() == 10 && day.chars().filter(|c| *c == '-').count() == 2 {
                    return crate::chat::wrap::truncate(rest, room);
                }
            }
        }
        crate::chat::wrap::truncate(l, room)
    };
    match label {
        Some(l) => ends(
            vec![Span::styled(name.to_string(), body)],
            vec![Span::styled(fit(l), body)],
            w,
            bg,
        ),
        None => ends(
            vec![Span::styled(name.to_string(), body)],
            vec![Span::styled("none", dim)],
            w,
            bg,
        ),
    }
}

fn changes(view: &View, theme: Theme, bg: Color, w: usize, condensed: bool) -> Vec<Line<'static>> {
    let dim = Style::default().fg(theme.dim).bg(bg);
    let body = Style::default().fg(theme.fg).bg(bg);
    let good = Style::default().fg(theme.success).bg(bg);
    let bad = Style::default().fg(theme.error).bg(bg);
    let mut lines = vec![ends(
        vec![heading("CHANGES", theme, bg)],
        if condensed {
            Vec::new()
        } else {
            vec![Span::styled("uncommitted", dim)]
        },
        w,
        bg,
    )];
    let counts = |added: u32, removed: u32| {
        vec![
            Span::styled(format!("+{added}"), good),
            Span::styled(format!(" −{removed}"), bad),
        ]
    };
    match &view.uncommitted {
        // Nothing to compare the files with.
        None => lines.push(Line::from(Span::styled("no repository here", dim))),
        Some(files) if files.is_empty() => {
            lines.push(Line::from(Span::styled("nothing uncommitted", dim)));
        }
        Some(files) if condensed => {
            let (a, r) = files
                .iter()
                .fold((0, 0), |(a, r), f| (a + f.added, r + f.removed));
            lines.push(ends(
                vec![Span::styled(
                    format!(
                        "{} file{}",
                        files.len(),
                        if files.len() == 1 { "" } else { "s" }
                    ),
                    body,
                )],
                counts(a, r),
                w,
                bg,
            ));
        }
        Some(files) => {
            for f in files.iter().take(FILES_SHOWN) {
                let what = match f.status {
                    Status::Added => vec![Span::styled("new", good)],
                    Status::Deleted => vec![Span::styled("deleted", bad)],
                    Status::Modified if f.binary => vec![Span::styled("binary", dim)],
                    Status::Modified => counts(f.added, f.removed),
                };
                let what_w: usize = what.iter().map(|s| wrap::width(&s.content)).sum();
                lines.push(ends(
                    vec![Span::styled(
                        tail(&f.path, w.saturating_sub(what_w + 1)),
                        body,
                    )],
                    what,
                    w,
                    bg,
                ));
            }
            if files.len() > FILES_SHOWN {
                lines.push(Line::from(Span::styled(
                    format!("+{} more", files.len() - FILES_SHOWN),
                    dim,
                )));
            }
        }
    }
    lines
}

/// The column's rows for `w` columns and `room` rows. A short screen
/// loses cards from the end: the changes, then the guard.
pub fn lines(
    view: &View,
    theme: Theme,
    w: usize,
    room: usize,
    condensed: bool,
) -> Vec<Line<'static>> {
    let bg = theme.sidebar_bg;
    let mut cards = vec![
        model(view, theme, bg, w, condensed),
        context(view, theme, bg, w),
        pulse(view, theme, bg, w),
        spend(view, theme, bg, w),
        guard(view, theme, bg, w, condensed),
        changes(view, theme, bg, w, condensed),
    ];
    let height = |cards: &[Vec<Line<'static>>]| cards.iter().map(|c| c.len() + 1).sum::<usize>();
    while height(&cards) > room && cards.len() > 1 {
        cards.pop();
    }
    let mut lines = Vec::new();
    for card in cards {
        lines.push(Line::from(""));
        lines.extend(card);
    }
    lines.truncate(room);
    lines
}

/// Paint the instruments into `area`: its full height, the hairline on
/// its left edge included.
pub fn draw(frame: &mut Frame, area: Rect, view: &View, theme: Theme, condensed: bool) {
    let bg = theme.sidebar_bg;
    let panel = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(1),
        ..area
    };
    frame.render_widget(Paragraph::new("").style(Style::default().bg(bg)), panel);
    let body = Rect {
        x: area.x + 2,
        width: area.width.saturating_sub(3),
        ..area
    };
    let rows = lines(
        view,
        theme,
        usize::from(body.width),
        usize::from(body.height),
        condensed,
    );
    frame.render_widget(
        Paragraph::new(rows).style(Style::default().fg(theme.fg).bg(bg)),
        body,
    );
    vline(frame, area.x, area, theme);
}
