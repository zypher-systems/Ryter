//! The sidebar (`docs/sidebar-design.md` §4): the solo screen's one column
//! beside the conversation. Every row on it is a measurement, an event or
//! a name: the hats as a ledger, what the model is doing now, the context
//! gauge, what the session and the project have cost, the files touched,
//! and what the hat may do. Nothing on it reads `off`, `none` or `idle`
//! as a steady state.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ryter_core::Role;
use ryter_core::review::Status;

use crate::chat::{fmt_elapsed, humanize, short_model, turn_usd, wrap};
use crate::theme::Theme;
use crate::view::View;

/// Columns the sidebar takes on the narrowest screen that holds it: the
/// hairline, a pad, and 27 of text.
pub const MIN_WIDTH: u16 = 30;
/// Columns it grows to on a wide screen, once the conversation's column
/// has all the width it reads well at.
pub const MAX_WIDTH: u16 = 40;
/// Narrowest screen that shows it beside the conversation.
pub const MIN_SCREEN: u16 = 100;
/// The hats, in the order the sidebar lists them: the primary hats, then
/// the specialists (`docs/specialists-design.md` §2).
pub const HATS: [Role; 4] = [
    Role::SoloPlan,
    Role::SoloBuild,
    Role::SoloAudit,
    Role::SoloScribe,
];
/// The primary row: the hats the work is done in.
pub const PRIMARY: [Role; 2] = [Role::SoloPlan, Role::SoloBuild];
/// The specialists, below the separator.
pub const SPECIALISTS: [Role; 2] = [Role::SoloAudit, Role::SoloScribe];
/// The word on the separator between the rows.
pub const SEPARATOR_LABEL: &str = "specialists";
/// Dots the `turns` row shows: the newest turns, oldest first.
pub const TURN_DOTS: usize = 15;
/// Changed files listed before the summary row says how many there are.
const FILES_SHOWN: usize = 6;
/// The column the figures start in, after a hat's name.
const FIGURES_AT: usize = 13;

/// What a screen of some width has room for beside the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// The sidebar.
    Sidebar,
    /// None: a status line at the foot.
    Narrow,
}

/// Columns the sidebar takes beside the conversation on a screen `width`
/// wide: the least on a narrow screen, growing with the screen once the
/// conversation's column has the width it reads well at, up to the most.
pub fn width_for(width: u16) -> u16 {
    width
        .saturating_sub(crate::draw::LEDGER_COLUMN + 4)
        .clamp(MIN_WIDTH, MAX_WIDTH)
}

/// The tier a screen `width` columns wide is in.
pub fn tier(width: u16) -> Tier {
    if width >= MIN_SCREEN {
        Tier::Sidebar
    } else {
        Tier::Narrow
    }
}

/// A hat's name as the screen says it: lowercase everywhere.
pub fn hat_name(hat: Role) -> &'static str {
    hat.hat().as_str()
}

/// The model `hat`'s next message goes to: its own where it has one,
/// otherwise the one every hat uses.
pub fn hat_model(view: &View, hat: Role) -> &str {
    view.specialists
        .get(hat.as_str())
        .filter(|r| r.is_override())
        .and_then(|r| r.model.as_deref())
        .unwrap_or(&view.model)
}

/// What `hat` has cost this session. A call with no price is `$?.??`, and
/// a total that leaves some out says so: never a made-up `$0.00`.
pub fn hat_spend(view: &View, hat: Role) -> String {
    match view.spend_rows_role.get(hat.as_str()) {
        Some(row) if row.unpriced && row.usd <= 0.0 => "$?.??".into(),
        Some(row) if row.unpriced => format!("≥{}", turn_usd(row.usd)),
        Some(row) => turn_usd(row.usd),
        None => "$0".into(),
    }
}

/// What the hat may do, in two or three words.
pub fn hat_may(hat: Role) -> &'static str {
    match hat {
        Role::SoloPlan => "read only",
        // What the audit changes, a checkpoint puts back.
        Role::SoloAudit => "checkpoint",
        Role::SoloScribe => "docs only",
        Role::SoloBuild | Role::Crew => "asks first",
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

/// `no cap`, or `of $N`: what the session's spend is measured against.
pub fn cap(view: &View) -> String {
    if view.budget_usd > 0.0 {
        format!("of ${:.2}", view.budget_usd)
    } else {
        "no cap".into()
    }
}

/// `left` and `right` at the two ends of a row `w` columns wide, on `bg`.
pub(crate) fn ends(
    left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    w: usize,
    bg: Color,
) -> Line<'static> {
    let used: usize = left
        .iter()
        .chain(&right)
        .map(|s| wrap::width(&s.content))
        .sum();
    let mut spans = left;
    spans.push(Span::styled(
        " ".repeat(w.saturating_sub(used)),
        Style::default().bg(bg),
    ));
    spans.extend(right);
    Line::from(spans)
}

/// A block's heading: its name, dim and lowercase.
pub(crate) fn heading(label: &str, theme: Theme, bg: Color) -> Span<'static> {
    Span::styled(label.to_string(), Style::default().fg(theme.dim).bg(bg))
}

/// The end of `s` in at most `max` columns, `…` where it was cut: a
/// path's file name is the part worth keeping.
pub(crate) fn tail(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    format!("…{}", s.chars().skip(n - keep).collect::<String>())
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

/// What the column gives up, in order, when it is taller than its room
/// (`R-LAYOUT-06`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cut {
    /// The changed files, keeping the summary row.
    FilesList,
    /// The `turns` row of dots.
    TurnsRow,
    /// The rate row under what the model is doing.
    NowRate,
    /// The permissions block.
    Permissions,
    /// The project's spend.
    ProjectRow,
}

const CUTS: [Cut; 5] = [
    Cut::FilesList,
    Cut::TurnsRow,
    Cut::NowRate,
    Cut::Permissions,
    Cut::ProjectRow,
];

/// The header: the name, the session's title, where this is.
fn header(view: &View, theme: Theme, bg: Color, w: usize) -> Vec<Line<'static>> {
    let dim = Style::default().fg(theme.dim).bg(bg);
    let body = Style::default().fg(theme.fg).bg(bg);
    let title = view.session_title.trim();
    let title = if title.is_empty() {
        Span::styled("new session", dim)
    } else {
        Span::styled(wrap::truncate(title, w), body)
    };
    let folder = std::path::Path::new(&view.cwd)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| view.cwd.clone());
    let place = match &view.git_branch {
        Some(b) => format!("{folder} · {b}"),
        None => folder,
    };
    vec![
        Line::from(Span::styled("RYTER", body.add_modifier(Modifier::BOLD))),
        Line::from(title),
        Line::from(Span::styled(wrap::truncate(&place, w), dim)),
    ]
}

/// One hat's row of the ledger: its name in its color, `▸` and bold when
/// it is on, its verdict mark, then its turns and spend when it has any.
fn hat_row(view: &View, theme: Theme, hat: Role, bg: Color, w: usize) -> Line<'static> {
    let on = view.mode == hat;
    let color = theme.mode(hat);
    let name_style = if on {
        Style::default()
            .fg(color)
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(color).bg(bg)
    };
    let dim = Style::default().fg(theme.dim).bg(bg);
    let body = Style::default().fg(theme.fg).bg(bg);
    // The hat that is on is marked, and its mark spins while its model
    // works: the screen says it is alive where the eye is.
    let mark = if on && view.busy {
        format!(
            "{} ",
            crate::activity::SPINNER[view.activity.frame % crate::activity::SPINNER.len()]
        )
    } else if on {
        "▸ ".to_string()
    } else {
        "  ".to_string()
    };
    let mut left = vec![
        Span::styled(mark, name_style),
        Span::styled(hat_name(hat), name_style),
    ];
    let totals = view.rack.of(hat);
    // A specialist's latest verdict, after its name.
    if let Some(passed) = totals.last_verdict {
        let (mark, style) = if passed {
            ("✓", Style::default().fg(theme.success).bg(bg))
        } else {
            ("✗", Style::default().fg(theme.error).bg(bg))
        };
        left.push(Span::styled(" ", body));
        left.push(Span::styled(mark, style));
    }
    if totals.turns == 0 {
        return ends(left, Vec::new(), w, bg);
    }
    let used: usize = left.iter().map(|s| wrap::width(&s.content)).sum();
    left.push(Span::styled(
        " ".repeat(FIGURES_AT.saturating_sub(used).max(1)),
        body,
    ));
    left.push(Span::styled(
        format!(
            "{} turn{}",
            totals.turns,
            if totals.turns == 1 { "" } else { "s" }
        ),
        dim,
    ));
    ends(left, vec![Span::styled(hat_spend(view, hat), body)], w, bg)
}

/// The row between the primary hats and the specialists: a hairline with
/// the word on it.
fn separator(theme: Theme, bg: Color) -> Line<'static> {
    let faint = Style::default().fg(theme.faint).bg(bg);
    Line::from(vec![
        Span::styled("  ", faint),
        Span::styled("─".repeat(12), faint),
        Span::styled(format!(" {SEPARATOR_LABEL}"), faint),
    ])
}

/// The hats as a ledger (`R-HATS-*`).
fn hats(view: &View, theme: Theme, bg: Color, w: usize, turns_row: bool) -> Vec<Line<'static>> {
    let dim = Style::default().fg(theme.dim).bg(bg);
    let mut lines = vec![Line::from(heading("hats", theme, bg))];
    for hat in HATS {
        if hat == SPECIALISTS[0] {
            lines.push(separator(theme, bg));
        }
        lines.push(hat_row(view, theme, hat, bg, w));
    }
    let turns = view.rack.turn_hats();
    if turns_row && !turns.is_empty() {
        let mut spans = vec![Span::styled(format!("{:<FIGURES_AT$}", "  turns"), dim)];
        let room = w.saturating_sub(FIGURES_AT).min(TURN_DOTS);
        for hat in turns.iter().rev().take(room).rev() {
            spans.push(Span::styled(
                "●",
                Style::default().fg(theme.mode(*hat)).bg(bg),
            ));
        }
        lines.push(Line::from(spans));
    }
    // The files every hat reads, when they exist: never `none`.
    for (name, label) in [
        ("plan.md", view.plan_file.as_deref()),
        (
            "audit.md",
            if view.audit_writing {
                Some("writing…")
            } else {
                view.audit_file.as_deref()
            },
        ),
    ] {
        if let Some(l) = label {
            let left = format!("  {name} ");
            let room = w.saturating_sub(wrap::width(&left));
            // Short of room, what the file is matters more than its day.
            let fit = match l.split_once(' ') {
                Some((day, rest))
                    if wrap::width(l) > room
                        && day.len() == 10
                        && day.chars().filter(|c| *c == '-').count() == 2 =>
                {
                    wrap::truncate(rest, room)
                }
                _ => wrap::truncate(l, room),
            };
            lines.push(Line::from(vec![
                Span::styled(left, dim),
                Span::styled(fit, dim),
            ]));
        }
    }
    lines
}

/// What the model is doing now (`R-NOW-*`): its name, the turn's verb as
/// the status row says it, and the rate. In the warning color while a
/// question waits on the user.
fn now(view: &View, theme: Theme, bg: Color, w: usize, rate_row: bool) -> Vec<Line<'static>> {
    let dim = Style::default().fg(theme.dim).bg(bg);
    let body = Style::default().fg(theme.fg).bg(bg);
    let hat = Style::default().fg(theme.mode(view.mode)).bg(bg);
    let warn = Style::default().fg(theme.warn).bg(bg);
    let asking = view.busy && view.activity.ask.is_some();
    let mut lines = vec![
        Line::from(heading("now", theme, bg)),
        Line::from(Span::styled(
            format!(
                "  {}",
                wrap::truncate(short_model(hat_model(view, view.mode)), w.saturating_sub(2))
            ),
            body,
        )),
    ];
    if view.busy {
        let verb = view.activity.verb_text();
        let elapsed = fmt_elapsed(view.activity.elapsed_ms / 1000);
        // The verb is the status row's words and keeps them; the time
        // gives way when the two don't fit side by side.
        let both = 2 + wrap::width(&verb) + 2 + wrap::width(&elapsed) <= w;
        let right = if both {
            vec![Span::styled(elapsed, if asking { warn } else { dim })]
        } else {
            Vec::new()
        };
        lines.push(ends(
            vec![Span::styled(
                format!("  {}", wrap::truncate(&verb, w.saturating_sub(2))),
                if asking { warn } else { hat },
            )],
            right,
            w,
            bg,
        ));
    } else {
        lines.push(Line::from(Span::styled("  idle", dim)));
    }
    if rate_row {
        let third = if asking {
            let ask = view.activity.ask.clone().unwrap_or_default();
            let text = match view.activity.ask_about.as_deref() {
                Some(about) => format!("{ask}  {about}"),
                None => ask,
            };
            Span::styled(
                format!("  {}", wrap::truncate(&text, w.saturating_sub(2))),
                warn,
            )
        } else if view.busy {
            let mut parts = Vec::new();
            if let Some(rate) = view.pulse.rate(view.now_ms) {
                parts.push(format!("{rate} tok/s"));
            }
            if view.activity.tokens > 0 {
                parts.push(format!("{} tokens", humanize(view.activity.tokens)));
            }
            Span::styled(format!("  {}", parts.join(" · ")), dim)
        } else {
            Span::styled("", dim)
        };
        lines.push(Line::from(third));
    }
    lines
}

fn context(view: &View, theme: Theme, bg: Color, w: usize) -> Vec<Line<'static>> {
    let dim = Style::default().fg(theme.dim).bg(bg);
    let frac = view.ctx_frac();
    let pct = format!("{}%", (frac * 100.0).round() as u32);
    // Two cells in, three before the figure.
    let cells = w.saturating_sub(2 + 3 + wrap::width(&pct));
    let filled = ((frac * cells as f64).round() as usize).min(cells);
    vec![
        Line::from(heading("context", theme, bg)),
        ends(
            vec![
                Span::styled("  ", dim),
                Span::styled(
                    "━".repeat(filled),
                    Style::default().fg(gauge_color(view, theme)).bg(bg),
                ),
                Span::styled(
                    "─".repeat(cells - filled),
                    Style::default().fg(theme.rule).bg(bg),
                ),
            ],
            vec![Span::styled(pct, Style::default().fg(theme.fg).bg(bg))],
            w,
            bg,
        ),
        Line::from(Span::styled(
            format!(
                "  {} of {}",
                short_count(view.ctx_tokens.unwrap_or(0)),
                short_count(view.ctx_window_or_default())
            ),
            dim,
        )),
    ]
}

fn spend(view: &View, theme: Theme, bg: Color, project_row: bool) -> Vec<Line<'static>> {
    let dim = Style::default().fg(theme.dim).bg(bg);
    let body = Style::default().fg(theme.fg).bg(bg);
    let (session, color) = session_spend(view, theme);
    let cap_style = if color == theme.fg {
        dim
    } else {
        Style::default().fg(color).bg(bg)
    };
    let mut lines = vec![
        Line::from(heading("spend", theme, bg)),
        Line::from(vec![
            Span::styled(format!("{:<FIGURES_AT$}", "  session"), dim),
            Span::styled(
                session,
                Style::default()
                    .fg(color)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {}", cap(view)), cap_style),
        ]),
    ];
    // Across this repository's sessions, as `ryter spend --project` says.
    if let (true, Some(p)) = (project_row, &view.project_spend) {
        let known = turn_usd(p.total_usd);
        lines.push(Line::from(vec![
            Span::styled(format!("{:<FIGURES_AT$}", "  project"), dim),
            Span::styled(
                if p.unpriced_calls > 0 {
                    format!("≥{known}")
                } else {
                    known
                },
                body,
            ),
        ]));
    }
    lines
}

fn changes(view: &View, theme: Theme, bg: Color, w: usize, list: bool) -> Vec<Line<'static>> {
    let dim = Style::default().fg(theme.dim).bg(bg);
    let body = Style::default().fg(theme.fg).bg(bg);
    let good = Style::default().fg(theme.success).bg(bg);
    let bad = Style::default().fg(theme.error).bg(bg);
    let mut lines = vec![Line::from(heading("changes", theme, bg))];
    let counts = |added: u32, removed: u32| {
        vec![
            Span::styled(format!("+{added}"), good),
            Span::styled(format!(" −{removed}"), bad),
        ]
    };
    match &view.uncommitted {
        // Nothing to compare the files with.
        None => lines.push(Line::from(Span::styled("  no repository here", dim))),
        Some(files) if files.is_empty() => {
            lines.push(Line::from(Span::styled("  nothing uncommitted", dim)));
        }
        Some(files) => {
            if list {
                for f in files.iter().take(FILES_SHOWN) {
                    let what = match f.status {
                        Status::Added => vec![Span::styled("new", good)],
                        Status::Deleted => vec![Span::styled("deleted", bad)],
                        Status::Modified if f.binary => vec![Span::styled("binary", dim)],
                        Status::Modified => counts(f.added, f.removed),
                    };
                    let what_w: usize = what.iter().map(|s| wrap::width(&s.content)).sum();
                    lines.push(ends(
                        vec![
                            Span::styled("  ", body),
                            Span::styled(tail(&f.path, w.saturating_sub(what_w + 3)), body),
                        ],
                        what,
                        w,
                        bg,
                    ));
                }
            }
            let n = files.len();
            let summary = format!("  {n} file{} uncommitted", if n == 1 { "" } else { "s" });
            if list && n > FILES_SHOWN {
                lines.push(Line::from(vec![
                    Span::styled(format!("  +{} more · ", n - FILES_SHOWN), dim),
                    Span::styled(summary.trim_start().to_string(), dim),
                ]));
            } else {
                lines.push(Line::from(Span::styled(summary, dim)));
            }
        }
    }
    lines
}

fn permissions(view: &View, theme: Theme, bg: Color, w: usize) -> Vec<Line<'static>> {
    let dim = Style::default().fg(theme.dim).bg(bg);
    let hat = Style::default().fg(theme.mode(view.mode)).bg(bg);
    let profile = if view.sandbox_profile.trim().is_empty() {
        "off"
    } else {
        view.sandbox_profile.trim()
    };
    let mut spans = vec![
        Span::styled("  ", dim),
        Span::styled(hat_may(view.mode), hat),
    ];
    if view.perm_mode == "yolo" {
        spans.push(Span::styled(
            " · yolo",
            Style::default().fg(theme.warn).bg(bg),
        ));
    }
    let sandbox = format!(" · sandbox {profile}");
    let used: usize = spans.iter().map(|s| wrap::width(&s.content)).sum();
    let mut lines = vec![Line::from(heading("permissions", theme, bg))];
    // The sandbox on the same row where it fits, under it where it doesn't:
    // a profile's name is not cut.
    if used + wrap::width(&sandbox) <= w {
        spans.push(Span::styled(sandbox, dim));
        lines.push(Line::from(spans));
    } else {
        lines.push(Line::from(spans));
        lines.push(Line::from(Span::styled(
            format!(
                "  {}",
                wrap::truncate(sandbox.trim_start_matches(" · "), w.saturating_sub(2))
            ),
            dim,
        )));
    }
    lines
}

/// The column's rows for `w` columns of text, with the cuts in `cuts`
/// made. The header first, then each block after a blank row.
fn assemble(view: &View, theme: Theme, w: usize, cuts: &[Cut]) -> Vec<Line<'static>> {
    let bg = theme.sidebar_bg;
    let cut = |c: Cut| cuts.contains(&c);
    let mut blocks = vec![
        hats(view, theme, bg, w, !cut(Cut::TurnsRow)),
        now(view, theme, bg, w, !cut(Cut::NowRate)),
        context(view, theme, bg, w),
        spend(view, theme, bg, !cut(Cut::ProjectRow)),
        changes(view, theme, bg, w, !cut(Cut::FilesList)),
    ];
    if !cut(Cut::Permissions) {
        blocks.push(permissions(view, theme, bg, w));
    }
    let mut lines = header(view, theme, bg, w);
    for block in blocks {
        lines.push(Line::from(""));
        lines.extend(block);
    }
    lines
}

/// The column's rows for a column `w` wide with `room` rows: whole, or
/// with as many of the cuts as it takes to fit. `None` when even that
/// doesn't: the sidebar folds away.
pub fn lines(view: &View, theme: Theme, w: usize, room: usize) -> Option<Vec<Line<'static>>> {
    (0..=CUTS.len())
        .map(|n| assemble(view, theme, w, &CUTS[..n]))
        .find(|v| v.len() <= room)
}

/// Every row, uncut, for the `^b` panel.
pub fn panel_lines(view: &View, theme: Theme, w: usize) -> Vec<Line<'static>> {
    // The panel's title says where this is; the name's row is left to it.
    // A cell of pad inside the border, as the column has beside the
    // hairline.
    assemble(view, theme, w.saturating_sub(1), &[])
        .into_iter()
        .skip(1)
        .map(|l| {
            let mut spans = vec![Span::styled(" ", Style::default().bg(theme.sidebar_bg))];
            spans.extend(l.spans);
            Line::from(spans)
        })
        .collect()
}

/// Whether the sidebar has room in a column `width` wide and `height`
/// rows tall.
pub fn fits(view: &View, theme: Theme, width: u16, height: u16) -> bool {
    lines(
        view,
        theme,
        usize::from(width.saturating_sub(3)),
        usize::from(height),
    )
    .is_some()
}

/// Paint the sidebar into `area`: its full height, the hairline on its
/// left edge included.
pub fn draw(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
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
    if let Some(rows) = lines(
        view,
        theme,
        usize::from(body.width),
        usize::from(body.height),
    ) {
        frame.render_widget(
            Paragraph::new(rows).style(Style::default().fg(theme.fg).bg(bg)),
            body,
        );
    }
    vline(frame, area.x, area, theme);
}

/// A hairline down the column at `x`, for the height of `area`.
pub(crate) fn vline(frame: &mut Frame, x: u16, area: Rect, theme: Theme) {
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        if x < buf.area.width && y < buf.area.height {
            let c = &mut buf[(x, y)];
            c.set_symbol("│");
            c.set_style(Style::default().fg(theme.rule).bg(theme.bg));
        }
    }
}
