//! The hat rack (`docs/hat-rack-design.md` §5): the solo screen's left
//! column. One block a hat, always in the same order and always the same
//! height: the hat's model, how many turns it has had, what it has cost,
//! and the one or two figures that mean something for that hat. It never
//! lists turns: the conversation is the list of turns.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ryter_core::Role;

use crate::chat::{turn_usd, wrap};
use crate::theme::Theme;
use crate::view::View;

/// Columns the rack takes, the hairline on its right included.
pub const RACK_W: u16 = 30;
/// Narrowest screen that shows the rack and the instruments together.
pub const BOTH_MIN: u16 = 132;
/// Narrowest screen that shows the instruments.
pub const INSTRUMENTS_MIN: u16 = 100;
/// The hats, in the order the rack shows them: the primary row, then the
/// specialists (`docs/specialists-design.md` §5).
pub const HATS: [Role; 3] = [Role::SoloPlan, Role::SoloBuild, Role::SoloAudit];
/// The primary row: the hats the work is done in.
pub const PRIMARY: [Role; 2] = [Role::SoloPlan, Role::SoloBuild];
/// The specialists, below the separator.
pub const SPECIALISTS: [Role; 1] = [Role::SoloAudit];
/// The word on the separator between the rows.
pub const SEPARATOR_LABEL: &str = "specialists";

/// What a screen of some width has room for beside the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// The rack and the instruments.
    Wide,
    /// The instruments, condensed.
    Mid,
    /// Neither: a status line at the foot.
    Narrow,
}

/// The tier a screen `width` columns wide is in.
pub fn tier(width: u16) -> Tier {
    if width >= BOTH_MIN {
        Tier::Wide
    } else if width >= INSTRUMENTS_MIN {
        Tier::Mid
    } else {
        Tier::Narrow
    }
}

/// A hat's name as the screen says it.
pub fn hat_name(hat: Role) -> &'static str {
    match hat {
        Role::SoloPlan => "PLAN",
        Role::SoloAudit => "AUDIT",
        Role::SoloBuild | Role::Crew => "BUILD",
    }
}

/// The mark beside a hat's name: the one on, one that has had a turn, one
/// that has not. It tells them apart where color can't.
pub fn hat_mark(view: &View, hat: Role) -> &'static str {
    if view.mode == hat {
        "◆"
    } else if view.rack.worn(hat) {
        "●"
    } else {
        "○"
    }
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

/// A card's heading: its name, dim.
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

/// `(success, warning, failure)` from a test run's summary line, as the
/// tools report one: `13 passed, 2 skipped`, `test result: ok. 446 passed;
/// 0 failed; 3 ignored`, `Tests: 1 failed, 5 passed, 6 total`. `None`
/// when the line has no counts to read.
pub fn check_counts(summary: &str) -> Option<(u32, u32, u32)> {
    let (mut ok, mut warn, mut bad) = (0u32, 0u32, 0u32);
    let mut any = false;
    let words: Vec<&str> = summary.split_whitespace().collect();
    for pair in words.windows(2) {
        let digits: String = pair[0].chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() || pair[0].chars().any(|c| !c.is_ascii_digit() && c != ',') {
            continue;
        }
        let Ok(n) = digits.parse::<u32>() else {
            continue;
        };
        let word = pair[1]
            .trim_matches(|c: char| !c.is_ascii_alphabetic())
            .to_ascii_lowercase();
        let slot = match word.as_str() {
            "passed" | "passing" | "pass" | "ok" | "succeeded" => &mut ok,
            "failed" | "failing" | "fail" | "error" | "errors" | "broken" => &mut bad,
            "skipped" | "ignored" | "warning" | "warnings" | "xfailed" | "pending" | "todo"
            | "flaky" => &mut warn,
            _ => continue,
        };
        *slot += n;
        any = true;
    }
    any.then_some((ok, warn, bad))
}

/// The three rows a block's checks take: how many went well, how many are
/// a warning (skipped, not reached), how many failed. A count of nothing
/// is dim, so the eye lands on the one that isn't.
fn check_rows(
    (ok, warn, bad): (u32, u32, u32),
    theme: Theme,
    bg: Color,
    w: usize,
) -> Vec<Line<'static>> {
    let dim = Style::default().fg(theme.dim).bg(bg);
    [
        ("success", ok, theme.success),
        ("warning", warn, theme.warn),
        ("failure", bad, theme.error),
    ]
    .into_iter()
    .map(|(label, n, color)| {
        let style = if n == 0 {
            dim
        } else {
            Style::default().fg(color).bg(bg)
        };
        ends(
            vec![Span::styled(format!("  {label}"), style)],
            vec![Span::styled(n.to_string(), style)],
            w,
            bg,
        )
    })
    .collect()
}

/// One hat's block. `figures` adds the rows that are that hat's own; a
/// short screen leaves them off every block at once.
fn block(view: &View, theme: Theme, hat: Role, w: usize, figures: bool) -> Vec<Line<'static>> {
    let on = view.mode == hat;
    let worn = view.rack.worn(hat);
    let bg = if on {
        theme.rack_tint(hat)
    } else {
        theme.sidebar_bg
    };
    let color = theme.mode(hat);
    let dim = Style::default().fg(theme.dim).bg(bg);
    let body = Style::default().fg(theme.fg).bg(bg);
    let pad = |spans: Vec<Span<'static>>| ends(spans, Vec::new(), w, bg);
    let row = |label: &str, value: Vec<Span<'static>>, label_style: Style| {
        ends(
            vec![Span::styled(format!("  {label}"), label_style)],
            value,
            w,
            bg,
        )
    };
    let mark_style = if on || worn {
        Style::default().fg(color).bg(bg)
    } else {
        dim
    };
    let name_style = if on {
        Style::default()
            .fg(color)
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    } else if worn {
        body.add_modifier(Modifier::BOLD)
    } else {
        dim.add_modifier(Modifier::BOLD)
    };
    let mut lines = vec![
        pad(vec![
            Span::styled(format!("{} ", hat_mark(view, hat)), mark_style),
            Span::styled(hat_name(hat), name_style),
        ]),
        pad(vec![Span::styled(
            format!(
                "  {}",
                wrap::truncate(
                    crate::chat::short_model(hat_model(view, hat)),
                    w.saturating_sub(2)
                )
            ),
            dim,
        )]),
    ];
    let totals = view.rack.of(hat);
    if !worn {
        lines.push(pad(vec![Span::styled("  not worn yet", dim)]));
        return lines;
    }
    lines.push(row(
        &format!(
            "{} turn{}",
            totals.turns,
            if totals.turns == 1 { "" } else { "s" }
        ),
        vec![Span::styled(hat_spend(view, hat), body)],
        body,
    ));
    if !figures {
        return lines;
    }
    let good = Style::default().fg(theme.success).bg(bg);
    let bad = Style::default().fg(theme.error).bg(bg);
    // `✓ 2 pass`, `✗ 1 fail`: only the kinds that happened. Both have
    // happened: the counts alone, which is what fits beside the label.
    let tally = |passed: u32, failed: u32, pass: &str, fail: &str| {
        let both = passed > 0 && failed > 0;
        let said = |n: u32, mark: char, word: &str| {
            if both {
                format!("{mark} {n}")
            } else {
                format!("{mark} {n} {word}")
            }
        };
        let mut v = Vec::new();
        if passed > 0 {
            v.push(Span::styled(said(passed, '✓', pass), good));
        }
        if both {
            v.push(Span::styled("  ", dim));
        }
        if failed > 0 {
            v.push(Span::styled(said(failed, '✗', fail), bad));
        }
        v
    };
    match hat {
        Role::SoloPlan => {
            if totals.plans_approved > 0 {
                lines.push(row(
                    "plans",
                    vec![Span::styled(
                        format!("{} approved", totals.plans_approved),
                        body,
                    )],
                    dim,
                ));
            }
            if totals.plans_rejected > 0 {
                lines.push(row(
                    if totals.plans_approved > 0 {
                        ""
                    } else {
                        "plans"
                    },
                    vec![Span::styled(
                        format!("{} rejected", totals.plans_rejected),
                        body,
                    )],
                    dim,
                ));
            }
        }
        Role::SoloBuild | Role::Crew => {
            if !totals.files.is_empty() {
                lines.push(row(
                    "files",
                    vec![Span::styled(totals.files.len().to_string(), body)],
                    dim,
                ));
                lines.push(row(
                    "lines",
                    vec![
                        Span::styled(format!("+{}", totals.added), good),
                        Span::styled(format!(" −{}", totals.removed), bad),
                    ],
                    dim,
                ));
            }
            // The latest run of the project's tests, whichever hat ran
            // them: its counts, or its words where it gave none.
            if let Some(t) = &view.last_tests {
                match check_counts(t) {
                    Some(counts) => lines.extend(check_rows(counts, theme, bg, w)),
                    None => {
                        let style = if t.starts_with('✗') { bad } else { good };
                        lines.push(row(
                            "checks",
                            vec![Span::styled(wrap::truncate(t, w.saturating_sub(10)), style)],
                            dim,
                        ));
                    }
                }
            }
        }
        Role::SoloAudit => {
            // `✗ 1 fail  ✓ 2 pass`: what failed first, where the eye goes.
            let both = totals.verdicts_failed > 0 && totals.verdicts_passed > 0;
            let mut v = Vec::new();
            // Both at once: the counts alone, which is what fits.
            if totals.verdicts_failed > 0 {
                let n = totals.verdicts_failed;
                v.push(Span::styled(
                    if both {
                        format!("✗ {n}")
                    } else {
                        format!("✗ {n} fail")
                    },
                    bad,
                ));
            }
            if both {
                v.push(Span::styled("  ", dim));
            }
            if totals.verdicts_passed > 0 {
                let n = totals.verdicts_passed;
                v.push(Span::styled(
                    if both {
                        format!("✓ {n}")
                    } else {
                        format!("✓ {n} pass")
                    },
                    good,
                ));
            }
            // The tally helper is the other hats' shape.
            let _ = tally;
            if !v.is_empty() {
                lines.push(row("audits", v, dim));
            }
        }
    }
    lines
}

/// The row between the primary hats and the specialists: a hairline
/// across the column, with the word at its right end.
fn separator(theme: Theme, w: usize) -> Line<'static> {
    let bg = theme.sidebar_bg;
    let label = format!(" {SEPARATOR_LABEL}");
    let rule = w.saturating_sub(wrap::width(&label));
    Line::from(vec![
        Span::styled("─".repeat(rule), Style::default().fg(theme.faint).bg(bg)),
        Span::styled(label, Style::default().fg(theme.dim).bg(bg)),
    ])
}

/// The rack's rows for a column `w` wide with `room` rows: every hat's
/// block with its own figures, or without them when that is what fits.
/// `None` when even that doesn't: the rack folds away.
pub fn lines(view: &View, theme: Theme, w: usize, room: usize) -> Option<Vec<Line<'static>>> {
    let bg = theme.sidebar_bg;
    let build = |figures: bool| {
        let mut v = vec![Line::from(""), Line::from(heading("HAT RACK", theme, bg))];
        for hat in PRIMARY {
            v.push(Line::from(""));
            v.extend(block(view, theme, hat, w, figures));
        }
        v.push(Line::from(""));
        v.push(separator(theme, w));
        for hat in SPECIALISTS {
            v.push(Line::from(""));
            v.extend(block(view, theme, hat, w, figures));
        }
        v
    };
    [true, false]
        .into_iter()
        .map(build)
        .find(|v| v.len() <= room)
}

/// Whether the rack has room in a column `height` rows tall.
pub fn fits(view: &View, theme: Theme, height: u16) -> bool {
    lines(
        view,
        theme,
        usize::from(RACK_W.saturating_sub(3)),
        usize::from(height),
    )
    .is_some()
}

/// Paint the rack into `area`: its full height, the hairline on its right
/// edge included.
pub fn draw(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    let bg = theme.sidebar_bg;
    let panel = Rect {
        width: area.width.saturating_sub(1),
        ..area
    };
    frame.render_widget(Paragraph::new("").style(Style::default().bg(bg)), panel);
    let body = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(3),
        ..area
    };
    if let Some(rows) = lines(
        view,
        theme,
        usize::from(body.width),
        usize::from(body.height),
    ) {
        // A tinted block runs to both edges of the column, so each row is
        // drawn across the padding as well as the text.
        for (i, line) in rows.into_iter().enumerate() {
            let y = area.y + i as u16;
            let row_bg = line.spans.first().and_then(|s| s.style.bg).unwrap_or(bg);
            frame.render_widget(
                Paragraph::new("").style(Style::default().bg(row_bg)),
                Rect {
                    y,
                    height: 1,
                    ..panel
                },
            );
            frame.render_widget(
                Paragraph::new(line).style(Style::default().fg(theme.fg).bg(row_bg)),
                Rect {
                    y,
                    height: 1,
                    ..body
                },
            );
        }
    }
    vline(frame, area.x + area.width.saturating_sub(1), area, theme);
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
