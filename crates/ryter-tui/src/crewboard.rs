//! Mission control: crew mode's board on the ledger. Tiles for spend, tasks,
//! the patch and the clock; the plan as a tree of what waits on what; one
//! lane per worker. Everything drawn comes from the agent's queue snapshots
//! (`AgentEvent::Tasks`) and the live crew rows, so the picture is the
//! queue, not a model's account of it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ryter_core::queue::TaskView;

use crate::chat::{fmt_elapsed, turn_usd, wrap};
use crate::theme::Theme;
use crate::view::View;

/// Rows the tiles take, borders included.
const TILES_H: u16 = 4;
/// Rows of a full lane card: who and what, the meter, three lines of output.
const CARD_H: u16 = 5;
/// Rows the lead's box keeps, however many lanes there are.
const LEAD_MIN: u16 = 8;

/// Whether the board shows: crew mode on the ledger, plan or not yet. It
/// used to wait for a plan, so a crew session that began with a question
/// looked like solo mode.
pub fn shown(view: &View) -> bool {
    // The workbench is a view of its own, in crew mode too.
    !view.ui.classic() && view.crew_mode() && view.workbench.is_none()
}

/// Where the lead's conversation goes inside the board.
#[derive(Debug, Clone, Copy)]
pub struct LeadAreas {
    /// The lead's chat.
    pub chat: Rect,
    /// The activity strip, when a turn runs.
    pub activity: Rect,
    /// The prompt.
    pub composer: Rect,
}

/// The crew screen, as design D lays it out: tiles across the top; the plan
/// on the left at full height with its legend; on the right the lanes, and
/// under them the lead's conversation and the prompt in one box. Returns
/// where the caller draws the chat, the activity strip, and the composer.
pub fn draw_screen(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    theme: Theme,
    activity_h: u16,
    composer_h: u16,
) -> LeadAreas {
    let area = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(2),
        ..area
    };
    let tiles = Rect {
        height: TILES_H.min(area.height),
        ..area
    };
    draw_tiles(frame, tiles, view, theme);
    let rest = Rect {
        y: area.y + tiles.height,
        height: area.height.saturating_sub(tiles.height),
        ..area
    };
    let plan_w = plan_width(rest.width);
    let plan = Rect {
        width: plan_w,
        ..rest
    };
    let right = Rect {
        x: rest.x + plan_w + 1,
        width: rest.width.saturating_sub(plan_w + 1),
        ..rest
    };
    let inner = usize::from(plan_w.saturating_sub(3));
    let cap = usize::from(plan.height.saturating_sub(5));
    boxed_with_footer(
        frame,
        plan,
        "PLAN",
        plan_view(view, theme, inner, cap),
        legend(theme, inner),
        theme,
    );
    // Lanes: a live card per worker, as tall as leaves the lead its room.
    // Cards shrink to fit: three lines of output, then fewer, then one row.
    let n = view.crew.len() as u16;
    let room = right.height.saturating_sub(LEAD_MIN);
    let per = (1..=CARD_H)
        .rev()
        .find(|h| n * h + n.saturating_sub(1) * u16::from(*h > 1) + 2 <= room)
        .unwrap_or(1);
    let rows = if per == 1 || n == 0 {
        lane_rows(view, theme, right.width)
    } else {
        lane_cards(view, theme, right.width.saturating_sub(3), per)
    };
    // As tall as what it holds: a worker not yet streaming has no output.
    let lanes_h = (rows.len() as u16 + 2).clamp(3, room.max(3));
    let lanes = Rect {
        height: lanes_h,
        ..right
    };
    boxed(frame, lanes, "LANES", rows, view, theme);
    if per > 1 && n > 0 {
        right_title(
            frame,
            lanes,
            "live · newest output at the bottom of each card",
            theme,
        );
    }
    let lead = Rect {
        y: right.y + lanes_h,
        height: right.height.saturating_sub(lanes_h),
        ..right
    };
    let b = block("LEAD", theme);
    let inner = b.inner(lead);
    frame.render_widget(b, lead);
    let inner = Rect {
        x: inner.x + 1,
        width: inner.width.saturating_sub(1),
        ..inner
    };
    let composer_h = composer_h.min(inner.height.saturating_sub(2));
    let activity_h = activity_h.min(inner.height.saturating_sub(composer_h + 2));
    let chat_h = inner.height.saturating_sub(composer_h + activity_h);
    LeadAreas {
        chat: Rect {
            height: chat_h,
            ..inner
        },
        activity: Rect {
            y: inner.y + chat_h,
            height: activity_h,
            ..inner
        },
        composer: Rect {
            y: inner.y + chat_h + activity_h,
            height: composer_h,
            ..inner
        },
    }
}

/// What the plan's marks mean, at its foot.
fn legend(theme: Theme, width: usize) -> Vec<Line<'static>> {
    let dim = Style::default().fg(theme.dim).bg(theme.bg);
    let item = |mark: &'static str, color: Color, label: &'static str| {
        vec![
            Span::styled(mark, Style::default().fg(color).bg(theme.bg)),
            Span::styled(format!(" {label}   "), dim),
        ]
    };
    let items = [
        item("✓", theme.success, "landed"),
        item("◐", theme.accent, "building"),
        item("◑", theme.audit, "in audit"),
        item("✕", theme.error, "blocked"),
        item("○", theme.dim, "waiting"),
    ];
    // As many to a line as fit.
    let mut lines = vec![Line::from(Span::styled("legend", dim))];
    let mut row: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for it in items {
        let w: usize = it.iter().map(|s| wrap::width(&s.content)).sum();
        if used + w > width && !row.is_empty() {
            lines.push(Line::from(std::mem::take(&mut row)));
            used = 0;
        }
        used += w;
        row.extend(it);
    }
    lines.push(Line::from(row));
    lines
}

/// `boxed`, with `footer` pinned to the box's last rows.
fn boxed_with_footer(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    rows: Vec<Line<'static>>,
    footer: Vec<Line<'static>>,
    theme: Theme,
) {
    let b = block(title, theme);
    let inner = b.inner(area);
    frame.render_widget(b, area);
    let h = usize::from(inner.height);
    let foot = footer.len().min(h);
    let body_h = h - foot;
    let mut rows = rows;
    if rows.len() > body_h && body_h > 0 {
        let more = rows.len() - (body_h - 1);
        rows.truncate(body_h - 1);
        rows.push(Line::from(Span::styled(
            format!("… {more} more rows"),
            Style::default().fg(theme.dim).bg(theme.bg),
        )));
    }
    rows.truncate(body_h);
    while rows.len() < body_h {
        rows.push(Line::from(Span::styled(
            String::new(),
            Style::default().bg(theme.bg),
        )));
    }
    rows.extend(footer.into_iter().take(foot));
    frame.render_widget(
        Paragraph::new(rows).style(Style::default().bg(theme.bg)),
        Rect {
            x: inner.x + 1,
            width: inner.width.saturating_sub(1),
            ..inner
        },
    );
}

/// The plan's share of the width: the left side, as the design has it.
fn plan_width(total: u16) -> u16 {
    (total * 45 / 100).max(30).min(total.saturating_sub(40))
}

fn block(title: &str, theme: Theme) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.dim).bg(theme.bg))
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(theme.dim)
                .bg(theme.bg)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(theme.bg))
}

/// `text` on the right of a box's top edge, when it fits beside the title.
fn right_title(frame: &mut Frame, area: Rect, text: &str, theme: Theme) {
    let text = format!(" {text} ");
    let w = wrap::width(&text) as u16;
    // The title on the left takes about a dozen columns.
    if area.width < w + 16 {
        return;
    }
    frame.render_widget(
        Paragraph::new(Span::styled(
            text,
            Style::default().fg(theme.dim).bg(theme.bg),
        )),
        Rect {
            x: area.x + area.width - w - 2,
            width: w,
            height: 1,
            ..area
        },
    );
}

fn boxed(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    rows: Vec<Line<'static>>,
    _view: &View,
    theme: Theme,
) {
    let b = block(title, theme);
    let inner = b.inner(area);
    frame.render_widget(b, area);
    let h = usize::from(inner.height);
    // Too many rows: the first ones, and how many more there are. A plan
    // reads from the top; cutting it there hid what the rest waits on.
    let mut rows = rows;
    if rows.len() > h && h > 0 {
        let more = rows.len() - (h - 1);
        rows.truncate(h - 1);
        rows.push(Line::from(Span::styled(
            format!("… {more} more rows"),
            Style::default().fg(theme.dim).bg(theme.bg),
        )));
    }
    frame.render_widget(
        Paragraph::new(rows).style(Style::default().bg(theme.bg)),
        Rect {
            x: inner.x + 1,
            width: inner.width.saturating_sub(1),
            ..inner
        },
    );
}

fn draw_tiles(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    let w = area.width / 5;
    let dim = Style::default().fg(theme.dim).bg(theme.bg);
    let fg = Style::default()
        .fg(theme.fg)
        .bg(theme.bg)
        .add_modifier(Modifier::BOLD);
    let tiles: [(&str, Vec<Span<'static>>, Vec<Span<'static>>); 5] = [
        spend_tile(view, theme, dim, fg, w),
        tasks_tile(view, theme, dim, fg, w),
        checks_tile(view, theme, dim, fg),
        time_tile(view, dim, fg),
        pulse_tile(view, theme, dim, fg, w),
    ];
    for (i, (title, value, detail)) in tiles.into_iter().enumerate() {
        let x = area.x + w * i as u16;
        let width = if i == 4 {
            area.width - w * 4
        } else {
            w.saturating_sub(1)
        };
        let r = Rect { x, width, ..area };
        let b = block(title, theme);
        let inner = b.inner(r);
        frame.render_widget(b, r);
        frame.render_widget(
            Paragraph::new(vec![Line::from(value), Line::from(detail)])
                .style(Style::default().bg(theme.bg)),
            Rect {
                x: inner.x + 1,
                width: inner.width.saturating_sub(1),
                ..inner
            },
        );
    }
}

type Tile = (&'static str, Vec<Span<'static>>, Vec<Span<'static>>);

fn spend_tile(view: &View, theme: Theme, dim: Style, fg: Style, w: u16) -> Tile {
    let spent = view.spend.unwrap_or(0.0);
    let mut value = vec![Span::styled(turn_usd(spent), fg)];
    let detail = if view.budget_usd > 0.0 {
        value.push(Span::styled(format!(" of ${:.2}", view.budget_usd), dim));
        let cells = usize::from(w.saturating_sub(4)).max(4);
        let frac = (spent / view.budget_usd).clamp(0.0, 1.0);
        let filled = (frac * cells as f64).round() as usize;
        let color = crate::panel::widgets::gauge_color(frac, theme);
        vec![
            Span::styled("▰".repeat(filled), Style::default().fg(color).bg(theme.bg)),
            Span::styled("▱".repeat(cells - filled), dim),
        ]
    } else {
        value.push(Span::styled(" no budget", dim));
        // What the last minute cost: a run that is spending shows it moving.
        let recent: f64 = view.spend_log.iter().map(|(_, usd)| usd).sum();
        if recent > 0.0 {
            vec![Span::styled(
                format!("+{} in the last minute", turn_usd(recent)),
                dim,
            )]
        } else {
            vec![Span::styled("nothing in the last minute", dim)]
        }
    };
    ("SPEND", value, detail)
}

/// How fast the crew is producing, and how long since it last said anything:
/// a crew that is working shows it, and a stalled one shows that too.
fn pulse_tile(view: &View, theme: Theme, dim: Style, fg: Style, w: u16) -> Tile {
    let working = view.crew.len();
    if working == 0 {
        return ("PULSE", vec![Span::styled("idle", dim)], Vec::new());
    }
    let rate = view.crew_rate().round() as u64;
    let value = vec![
        Span::styled(format!("{rate} tok/s"), fg),
        Span::styled(format!(" · {working} working"), dim),
    ];
    let quiet = view
        .crew_last_ms()
        .map(|t| view.now_ms.saturating_sub(t) as f64 / 1000.0);
    let heard = match quiet {
        Some(q) if q < 10.0 => format!(" last byte {q:.1}s"),
        Some(q) => format!(" quiet {}", fmt_elapsed(q as u64)),
        None => " starting".into(),
    };
    let cells = usize::from(w.saturating_sub(4)).saturating_sub(wrap::width(&heard));
    let spark = sparkline(&view.pulse, cells);
    let heard_style = if quiet.is_some_and(|q| q >= 30.0) {
        Style::default().fg(theme.warn).bg(theme.bg)
    } else {
        dim
    };
    (
        "PULSE",
        value,
        vec![
            Span::styled(spark, Style::default().fg(theme.accent).bg(theme.bg)),
            Span::styled(heard, heard_style),
        ],
    )
}

/// The last `cells` samples as bars, scaled to the largest.
fn sparkline(samples: &[u64], cells: usize) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let from = samples.len().saturating_sub(cells);
    let shown = &samples[from..];
    let top = shown.iter().copied().max().unwrap_or(0).max(1);
    shown
        .iter()
        .map(|&v| BARS[((v * 7) / top) as usize])
        .collect()
}

fn tasks_tile(view: &View, theme: Theme, dim: Style, fg: Style, w: u16) -> Tile {
    let builders: Vec<&TaskView> = view.tasks.iter().filter(|t| t.role == "builder").collect();
    let count = |s: &str| builders.iter().filter(|t| t.status == s).count();
    let (done, running, blocked) = (count("done"), count("running"), count("blocked"));
    let total = builders.len();
    let value = vec![
        Span::styled(format!("{done}"), fg),
        Span::styled(format!(" of {total} landed"), dim),
    ];
    // One cell per share of the tasks, in their state's color: real counts.
    let cells = usize::from(w.saturating_sub(4)).max(4);
    let part = |n: usize| {
        if total == 0 {
            0
        } else {
            (n * cells).div_ceil(total).min(cells)
        }
    };
    let mut left = cells;
    let mut detail = Vec::new();
    for (n, color) in [
        (done, theme.success),
        (running, theme.accent),
        (blocked, theme.error),
    ] {
        let k = part(n).min(left);
        left -= k;
        detail.push(Span::styled(
            "▰".repeat(k),
            Style::default().fg(color).bg(theme.bg),
        ));
    }
    detail.push(Span::styled("▱".repeat(left), dim));
    ("TASKS", value, detail)
}

fn checks_tile(view: &View, theme: Theme, dim: Style, fg: Style) -> Tile {
    if view.crew_checks.is_empty() {
        return (
            "CHECKS",
            vec![Span::styled("none set", dim)],
            vec![Span::styled("each auditor tests it", dim)],
        );
    }
    let first = view.crew_checks[0].clone();
    let more = view.crew_checks.len() - 1;
    let value = if more == 0 {
        first
    } else {
        format!("{first} +{more}")
    };
    (
        "CHECKS",
        vec![
            Span::styled("✓ ", Style::default().fg(theme.success).bg(theme.bg)),
            Span::styled(value, fg),
        ],
        vec![Span::styled("each task + the patch", dim)],
    )
}

fn time_tile(view: &View, dim: Style, fg: Style) -> Tile {
    let secs = view
        .crew_started_ms
        .map(|s| view.now_ms.saturating_sub(s) / 1000);
    let value = match secs {
        Some(s) => vec![Span::styled(fmt_elapsed(s), fg)],
        None => vec![Span::styled("idle", dim)],
    };
    let retries: u32 = view.tasks.iter().map(|t| t.retries).sum();
    (
        "ELAPSED",
        value,
        vec![Span::styled(
            format!(
                "{} calls · {retries} retr{}",
                view.turn_calls,
                if retries == 1 { "y" } else { "ies" }
            ),
            dim,
        )],
    )
}

/// The lane working on a task, if any: matched by the task's title, which
/// is what the crew row carries.
fn lane_for<'a>(view: &'a View, t: &TaskView) -> Option<&'a crate::view::CrewRow> {
    view.crew.iter().find(|c| c.label == t.title)
}

/// A task's mark, color and state words.
fn state(view: &View, t: &TaskView, theme: Theme) -> (&'static str, Color, String) {
    match t.status.as_str() {
        "done" if t.role == "builder" => ("✓", theme.success, "landed".into()),
        "done" => ("✓", theme.success, "done".into()),
        "blocked" => (
            "✕",
            theme.error,
            if t.reason.is_empty() {
                "blocked".into()
            } else {
                format!("blocked: {}", t.reason)
            },
        ),
        "running" => {
            let lane = lane_for(view, t);
            let secs = lane.map(|l| view.now_ms.saturating_sub(l.started_ms) / 1000);
            let clock = secs
                .map(|s| format!(" {}", fmt_elapsed(s)))
                .unwrap_or_default();
            match lane.map(|l| l.acting.as_str()) {
                Some("auditor") => ("◑", theme.audit, format!("in audit{clock}")),
                _ if t.role == "architect" => ("◐", theme.accent, format!("designing{clock}")),
                _ => ("◐", theme.accent, format!("building{clock}")),
            }
        }
        "proposed" => ("◇", theme.warn, "proposed · waits for your go-ahead".into()),
        _ => (
            "○",
            theme.dim,
            if t.reason.is_empty() {
                "next".into()
            } else {
                t.reason.clone()
            },
        ),
    }
}

/// The plan as a tree: each task under the first task it waits on, marked
/// with its state; a task that waits on more than one says so. Then where
/// the patch stands.
fn plan_rows(view: &View, theme: Theme, width: usize) -> Vec<Line<'static>> {
    let tasks = &view.tasks;
    let has = |id: &str| tasks.iter().any(|t| t.id == id);
    let parents = |id: &str| -> Vec<&str> {
        view.task_edges
            .iter()
            .filter(|(a, b)| b == id && has(a))
            .map(|(a, _)| a.as_str())
            .collect()
    };
    let mut rows = Vec::new();
    if tasks.is_empty() {
        let dim = Style::default().fg(theme.dim).bg(theme.bg);
        rows.push(Line::from(Span::styled(
            "no plan yet",
            Style::default().fg(theme.fg).bg(theme.bg),
        )));
        for l in wrap::wrap_plain(
            "ask the lead for work: the tasks it or the architect queue appear here, with what each waits on",
            width.max(20),
        ) {
            rows.push(Line::from(Span::styled(l, dim)));
        }
        return rows;
    }
    let mut shown: Vec<&str> = Vec::new();
    #[allow(clippy::too_many_arguments)]
    fn walk<'a>(
        view: &'a View,
        t: &'a TaskView,
        prefix: String,
        last: bool,
        root: bool,
        shown: &mut Vec<&'a str>,
        rows: &mut Vec<Line<'static>>,
        theme: Theme,
        width: usize,
    ) {
        shown.push(&t.id);
        let (mark, color, words) = state(view, t, theme);
        let branch = if root {
            String::new()
        } else if last {
            "└▶ ".into()
        } else {
            "├▶ ".into()
        };
        let also: Vec<&str> = view
            .task_edges
            .iter()
            .filter(|(a, b)| *b == t.id && view.tasks.iter().any(|x| x.id == *a))
            .map(|(a, _)| a.as_str())
            .skip(1)
            .collect();
        let words = if also.is_empty() {
            words
        } else {
            format!("{words} · also after {}", also.join(", "))
        };
        let head = format!("{prefix}{branch}{mark} {}", t.id);
        let dim = Style::default().fg(theme.dim).bg(theme.bg);
        let mut line = vec![
            Span::styled(format!("{prefix}{branch}"), dim),
            Span::styled(format!("{mark} "), Style::default().fg(color).bg(theme.bg)),
            Span::styled(
                t.id.clone(),
                Style::default()
                    .fg(theme.fg)
                    .bg(theme.bg)
                    .add_modifier(Modifier::BOLD),
            ),
        ];
        // What a task waits on, or why it is blocked, is the point: it wraps
        // under the task rather than being cut at the box's edge.
        let room = width.saturating_sub(wrap::width(&head) + 2);
        let words_style = if color == theme.error {
            Style::default().fg(theme.error).bg(theme.bg)
        } else {
            dim
        };
        if wrap::width(&words) <= room {
            line.push(Span::styled(format!("  {words}"), words_style));
            rows.push(Line::from(line));
        } else {
            rows.push(Line::from(line));
            let indent = format!("{prefix}{}  ", if root { "" } else { "   " });
            for part in wrap::wrap_plain(&words, width.saturating_sub(wrap::width(&indent)).max(12))
            {
                rows.push(Line::from(vec![
                    Span::styled(indent.clone(), dim),
                    Span::styled(part, words_style),
                ]));
            }
        }
        let children: Vec<&TaskView> = view
            .tasks
            .iter()
            .filter(|c| {
                !shown.contains(&c.id.as_str())
                    && view
                        .task_edges
                        .iter()
                        .find(|(a, b)| *b == c.id && view.tasks.iter().any(|x| x.id == *a))
                        .is_some_and(|(a, _)| *a == t.id)
            })
            .collect();
        let deeper = if root {
            String::new()
        } else if last {
            format!("{prefix}   ")
        } else {
            format!("{prefix}│  ")
        };
        for (i, c) in children.iter().enumerate() {
            walk(
                view,
                c,
                deeper.clone(),
                i + 1 == children.len(),
                false,
                shown,
                rows,
                theme,
                width,
            );
        }
    }
    for t in tasks.iter().filter(|t| parents(&t.id).is_empty()) {
        if !shown.contains(&t.id.as_str()) {
            walk(
                view,
                t,
                String::new(),
                true,
                true,
                &mut shown,
                &mut rows,
                theme,
                width,
            );
        }
    }
    // Anything left (a cycle, or a parent that is gone) still shows.
    for t in tasks {
        if !shown.contains(&t.id.as_str()) {
            walk(
                view,
                t,
                String::new(),
                true,
                true,
                &mut shown,
                &mut rows,
                theme,
                width,
            );
        }
    }
    let dim = Style::default().fg(theme.dim).bg(theme.bg);
    rows.push(Line::from(Span::styled(String::new(), dim)));
    rows.extend(patch_line(view, theme));
    rows
}

/// Where the patch stands, under the plan.
fn patch_line(view: &View, theme: Theme) -> Option<Line<'static>> {
    let dim = Style::default().fg(theme.dim).bg(theme.bg);
    let p = view.patch_view.as_ref()?;
    // What the queue says, not the patch's own list: a task retried after it
    // landed is open again.
    let done = |id: &str| view.tasks.iter().any(|t| t.id == id && t.status == "done");
    let open: Vec<&str> = p
        .tasks
        .iter()
        .filter(|t| !done(t))
        .map(String::as_str)
        .collect();
    let note = if open.is_empty() {
        "lands when the crew finishes".to_string()
    } else {
        format!("lands when {} land", open.join(", "))
    };
    Some(Line::from(vec![
        Span::styled("patch ▸ ", dim),
        Span::styled(p.target.clone(), Style::default().fg(theme.fg).bg(theme.bg)),
        Span::styled(format!("  {note}"), dim),
    ]))
}

/// The plan as a drawing when it fits in `width × cap`, else as the tree.
fn plan_view(view: &View, theme: Theme, width: usize, cap: usize) -> Vec<Line<'static>> {
    if view.tasks.is_empty() {
        return plan_rows(view, theme, width);
    }
    plan_drawing(view, theme, width, cap).unwrap_or_else(|| plan_rows(view, theme, width))
}

/// Box rows, and the columns between a box and the next: `─┬─▶`.
const BOX_H: usize = 4;
const GAP: usize = 4;

/// The plan drawn: a box per task in its state's color, left to right by
/// what waits on what, each joined to the tasks after it. `None` when it
/// won't fit, and the tree is drawn instead. A task that waits on more than
/// one sits under the first; the others, and a blocked task's full reason,
/// are listed under the drawing.
fn plan_drawing(view: &View, theme: Theme, width: usize, cap: usize) -> Option<Vec<Line<'static>>> {
    let tasks = &view.tasks;
    let has = |id: &str| tasks.iter().any(|t| t.id == id);
    let parents = |id: &str| -> Vec<String> {
        view.task_edges
            .iter()
            .filter(|(a, b)| b == id && has(a) && a != id)
            .map(|(a, _)| a.clone())
            .collect()
    };
    // The first parent carries the box; a cycle or a lost parent makes a root.
    let n = tasks.len();
    let mut primary: Vec<Option<usize>> = vec![None; n];
    for (i, t) in tasks.iter().enumerate() {
        primary[i] = parents(&t.id)
            .first()
            .and_then(|p| tasks.iter().position(|x| x.id == *p));
    }
    for i in 0..n {
        // Walk up; a loop back to `i` is a cycle, so `i` becomes a root.
        let mut seen = vec![i];
        let mut at = primary[i];
        while let Some(p) = at {
            if seen.contains(&p) {
                primary[i] = None;
                break;
            }
            seen.push(p);
            at = primary[p];
        }
    }
    let children = |p: usize| -> Vec<usize> { (0..n).filter(|&c| primary[c] == Some(p)).collect() };
    let depth = |mut i: usize| {
        let mut d = 0;
        while let Some(p) = primary[i] {
            d += 1;
            i = p;
        }
        d
    };
    let levels = (0..n).map(depth).max().unwrap_or(0) + 1;
    let box_w = width.saturating_sub((levels - 1) * GAP) / levels;
    let box_w = box_w.min(30);
    if box_w < 15 {
        return None;
    }
    // Rows: each task at the top of the block its tasks after it fill.
    let mut row = vec![0usize; n];
    fn place(i: usize, at: usize, row: &mut [usize], kids: &dyn Fn(usize) -> Vec<usize>) -> usize {
        row[i] = at;
        let ks = kids(i);
        if ks.is_empty() {
            return at + BOX_H;
        }
        let mut r = at;
        for k in ks {
            r = place(k, r, row, kids);
        }
        r
    }
    let mut total = 0;
    for i in (0..n).filter(|&i| primary[i].is_none()) {
        total = place(i, total, &mut row, &children);
    }
    if total > cap.max(BOX_H) {
        return None;
    }
    let base = Style::default().fg(theme.dim).bg(theme.bg);
    let mut grid: Vec<Vec<(char, Style)>> = vec![vec![(' ', base); width]; total];
    let put = |grid: &mut Vec<Vec<(char, Style)>>, y: usize, x: usize, s: &str, st: Style| {
        for (k, ch) in s.chars().enumerate() {
            if let Some(cell) = grid.get_mut(y).and_then(|r| r.get_mut(x + k)) {
                *cell = (ch, st);
            }
        }
    };
    let mut notes: Vec<Line<'static>> = Vec::new();
    for (i, t) in tasks.iter().enumerate() {
        let (mark, color, words) = state(view, t, theme);
        let x = depth(i) * (box_w + GAP);
        let y = row[i];
        let edge = Style::default().fg(color).bg(theme.bg);
        let inner = box_w - 2;
        put(&mut grid, y, x, &format!("┌{}┐", "─".repeat(inner)), edge);
        let title = wrap::truncate(&format!(" {mark} {}", t.id), inner);
        put(&mut grid, y + 1, x, "│", edge);
        put(
            &mut grid,
            y + 1,
            x + 1,
            &format!("{title:<inner$}"),
            Style::default()
                .fg(theme.fg)
                .bg(theme.bg)
                .add_modifier(Modifier::BOLD),
        );
        put(&mut grid, y + 1, x + box_w - 1, "│", edge);
        let said = wrap::truncate(&format!("  {words}"), inner);
        put(&mut grid, y + 2, x, "│", edge);
        put(&mut grid, y + 2, x + 1, &format!("{said:<inner$}"), base);
        put(&mut grid, y + 2, x + box_w - 1, "│", edge);
        put(
            &mut grid,
            y + 3,
            x,
            &format!("└{}┘", "─".repeat(inner)),
            edge,
        );
        // What the box can't hold goes under the drawing, in full.
        if t.status == "blocked" && wrap::width(&format!("  {words}")) > inner {
            let red = Style::default().fg(theme.error).bg(theme.bg);
            for (k, part) in wrap::wrap_plain(&format!("✕ {}  {words}", t.id), width.max(20))
                .into_iter()
                .enumerate()
            {
                let part = if k == 0 { part } else { format!("  {part}") };
                notes.push(Line::from(Span::styled(part, red)));
            }
        }
        let more = parents(&t.id);
        if more.len() > 1 {
            notes.push(Line::from(Span::styled(
                format!("{} also waits on {}", t.id, more[1..].join(", ")),
                base,
            )));
        }
        // Connectors to the tasks after this one.
        let ks = children(i);
        if let (Some(first), Some(last)) = (ks.first(), ks.last()) {
            let x0 = x + box_w;
            let mid = y + 1;
            let line = Style::default().fg(theme.dim).bg(theme.bg);
            put(
                &mut grid,
                mid,
                x0,
                if ks.len() > 1 {
                    "─┬─▶"
                } else {
                    "───▶"
                },
                line,
            );
            let _ = first;
            for (k, &c) in ks.iter().enumerate().skip(1) {
                let cy = row[c] + 1;
                let glyph = if c == *last {
                    " └─▶"
                } else {
                    " ├─▶"
                };
                put(&mut grid, cy, x0, glyph, line);
                // The trunk down to it.
                let from = row[ks[k - 1]] + 2;
                for yy in from..cy {
                    put(&mut grid, yy, x0 + 1, "│", line);
                }
            }
        }
    }
    let mut rows: Vec<Line<'static>> = grid
        .into_iter()
        .map(|cells| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut text = String::new();
            let mut style = base;
            for (ch, st) in cells {
                if st != style && !text.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut text), style));
                }
                style = st;
                text.push(ch);
            }
            spans.push(Span::styled(text.trim_end().to_string(), style));
            Line::from(spans)
        })
        .collect();
    if !notes.is_empty() {
        rows.push(Line::from(Span::styled(String::new(), base)));
        for l in notes {
            rows.push(l);
        }
    }
    if let Some(p) = patch_line(view, theme) {
        rows.push(Line::from(Span::styled(String::new(), base)));
        rows.push(p);
    }
    Some(rows)
}

/// About how many tokens: `812 tok`, `1.9k tok`.
fn tokens_label(n: u64) -> String {
    if n < 1000 {
        format!("~{n} tok")
    } else {
        format!("~{:.1}k tok", n as f64 / 1000.0)
    }
}

/// A live card per worker (design C1): who, on what, what it is doing and
/// for how long; a meter; then the last lines of what it is producing. `per`
/// is rows a card gets, two to five.
fn lane_cards(view: &View, theme: Theme, width: u16, per: u16) -> Vec<Line<'static>> {
    use ryter_core::LivePhase;
    let width = usize::from(width);
    let base = Style::default().bg(theme.bg);
    let mut rows = Vec::new();
    for (i, c) in view.crew.iter().enumerate() {
        if i > 0 {
            rows.push(Line::from(Span::styled(String::new(), base)));
        }
        let who = if c.acting.is_empty() {
            c.role.clone()
        } else {
            c.acting.clone()
        };
        let selected = view.lane_selected == Some(i);
        let bg = if selected {
            theme.selection_bg
        } else {
            theme.bg
        };
        let on = |fg: Color| Style::default().fg(fg).bg(bg);
        let task = view.tasks.iter().find(|t| t.title == c.label);
        let task_id = task.map_or(c.label.clone(), |t| t.id.clone());
        let model = view
            .specialists
            .get(who.as_str())
            .and_then(|r| r.model.clone())
            .unwrap_or_else(|| view.model.clone());
        let model = crate::chat::short_model(&model).to_string();
        let cost = task
            .and_then(|t| view.task_spend.get(&t.id))
            .map(|u| format!("task {}", turn_usd(*u)));
        let live = c.live.as_ref();
        // The chip: what it is doing, in a color you can find across the board.
        let (chip, chip_bg) = match live.map(|l| l.phase) {
            Some(LivePhase::Waiting) => ("WAITING", theme.warn),
            Some(LivePhase::Thinking) => ("THINKING", theme.dim),
            Some(LivePhase::Writing) => ("WRITING", theme.success),
            Some(LivePhase::Running) => ("RUNNING", theme.accent),
            None => ("WORKING", theme.dim),
        };
        let target = match live {
            Some(l) => match l.target.strip_prefix("bash ") {
                Some(cmd) => format!("$ {cmd}"),
                None => l.target.clone(),
            },
            None => String::new(),
        };
        let since = live.map_or(c.started_ms, |l| l.since_ms);
        let clock = fmt_elapsed(view.now_ms.saturating_sub(since) / 1000);
        // Header: role, task, model … chip, target, clock.
        let left_w = wrap::width(&who) + 1 + wrap::width(&task_id) + 2 + wrap::width(&model);
        let right_min = chip.len() + 2 + 2 + clock.len();
        let target_room = width.saturating_sub(left_w + right_min + 3);
        let target = wrap::truncate(&target, target_room.max(8));
        let right_w = chip.len() + 2 + 1 + wrap::width(&target) + 2 + clock.len();
        let show_model = left_w + right_w + 2 <= width;
        let used = wrap::width(&who)
            + 1
            + wrap::width(&task_id)
            + if show_model {
                2 + wrap::width(&model)
            } else {
                0
            };
        let pad = width.saturating_sub(used + right_w);
        let mut header = vec![
            Span::styled(
                who.clone(),
                on(theme.role(&who)).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", on(theme.fg)),
            Span::styled(task_id, on(theme.fg).add_modifier(Modifier::BOLD)),
        ];
        if show_model {
            header.push(Span::styled(format!("  {model}"), on(theme.dim)));
        }
        header.push(Span::styled(" ".repeat(pad), on(theme.dim)));
        header.push(Span::styled(
            format!(" {chip} "),
            Style::default()
                .fg(theme.bg)
                .bg(chip_bg)
                .add_modifier(Modifier::BOLD),
        ));
        if !target.is_empty() {
            header.push(Span::styled(format!(" {target}"), on(theme.fg)));
        }
        header.push(Span::styled(format!("  {clock}"), on(theme.dim)));
        rows.push(Line::from(header));

        // The meter: how much, how fast, what it has cost.
        let mut meter: Vec<String> = Vec::new();
        match live {
            Some(l) if l.phase == LivePhase::Waiting => {
                meter.push("no reply from the model yet".into());
                if view.now_ms.saturating_sub(l.since_ms) > 20_000 {
                    meter.push("the provider is slow".into());
                }
            }
            Some(l) if l.phase == LivePhase::Running => {}
            Some(l) => {
                if l.lines > 0 {
                    meter.push(format!("{} lines so far", l.lines));
                }
                meter.push(tokens_label(l.tokens));
                if l.rate >= 1.0 {
                    meter.push(format!("{} tok/s", l.rate.round() as u64));
                }
            }
            None => meter.push(wrap::truncate(&c.status, 40)),
        }
        if let Some(cost) = cost {
            meter.push(cost);
        }
        if c.tools > 0 {
            meter.push(format!(
                "{} tool{} so far",
                c.tools,
                if c.tools == 1 { "" } else { "s" }
            ));
        }
        rows.push(Line::from(Span::styled(
            wrap::truncate(&meter.join(" · "), width),
            on(theme.dim),
        )));

        // What it is producing: its reasoning, the file it is writing, a
        // command's output. The newest line last.
        let tail_rows = usize::from(per.saturating_sub(2));
        if tail_rows == 0 || live.is_none() {
            continue;
        }
        let inset = theme.code_bg;
        let (style, mark) = match live.map(|l| (l.phase, l.lines > 0)) {
            Some((LivePhase::Thinking, _)) => (
                Style::default()
                    .fg(theme.dim)
                    .bg(inset)
                    .add_modifier(Modifier::ITALIC),
                "",
            ),
            Some((LivePhase::Writing, true)) => {
                (Style::default().fg(theme.success).bg(inset), "+ ")
            }
            Some((LivePhase::Writing, false)) => (Style::default().fg(theme.fg).bg(inset), ""),
            _ => (Style::default().fg(theme.dim).bg(inset), ""),
        };
        let thinking = live.is_some_and(|l| l.phase == LivePhase::Thinking);
        let tail: Vec<String> = if thinking && view.lanes_hide_reasoning {
            vec!["reasoning hidden · ^r shows it".into()]
        } else {
            live.map(|l| l.tail.clone()).unwrap_or_default()
        };
        // Long lines of prose wrap; code is cut at the edge.
        let inner = width.saturating_sub(2 + mark.len());
        let mut shown: Vec<String> = if thinking {
            tail.iter()
                .flat_map(|t| wrap::wrap_plain(t, inner.max(10)))
                .collect()
        } else {
            tail.iter().map(|t| wrap::truncate(t, inner)).collect()
        };
        let from = shown.len().saturating_sub(tail_rows);
        shown.drain(..from);
        let blank_rows = tail_rows - shown.len();
        for _ in 0..blank_rows {
            rows.push(Line::from(Span::styled(" ".repeat(width), style)));
        }
        let last = shown.len().saturating_sub(1);
        for (j, t) in shown.into_iter().enumerate() {
            // The line still being written ends in a cursor.
            let hidden = thinking && view.lanes_hide_reasoning;
            let cursor = if j == last && !hidden { "▌" } else { "" };
            let text = format!(" {mark}{t}{cursor}");
            let pad = width.saturating_sub(wrap::width(&text));
            rows.push(Line::from(vec![
                Span::styled(text, style),
                Span::styled(" ".repeat(pad), style),
            ]));
        }
    }
    rows
}

/// One row per worker: who is acting, on what, doing what, for how long.
/// The board falls back to it when there is no room for cards.
fn lane_rows(view: &View, theme: Theme, width: u16) -> Vec<Line<'static>> {
    let dim = Style::default().fg(theme.dim).bg(theme.bg);
    if view.crew.is_empty() {
        return vec![Line::from(Span::styled("no one is working right now", dim))];
    }
    let spin = crate::activity::SPINNER[view.activity.frame % crate::activity::SPINNER.len()];
    view.crew
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let who = if c.acting.is_empty() {
                c.role.as_str()
            } else {
                c.acting.as_str()
            };
            let selected = view.lane_selected == Some(i);
            let bg = if selected {
                theme.selection_bg
            } else {
                theme.bg
            };
            let on = |fg| Style::default().fg(fg).bg(bg);
            let secs = view.now_ms.saturating_sub(c.started_ms) / 1000;
            // What the task has cost so far, as the meter charged it.
            let cost = view
                .tasks
                .iter()
                .find(|t| t.title == c.label)
                .and_then(|t| view.task_spend.get(&t.id))
                .map(|u| format!("  {}", turn_usd(*u)))
                .unwrap_or_default();
            let clock = format!("  {}{cost}", fmt_elapsed(secs));
            // The task's id, as the plan names it; its title when unknown.
            let task = view
                .tasks
                .iter()
                .find(|t| t.title == c.label)
                .map_or(c.label.as_str(), |t| t.id.as_str());
            let task = wrap::truncate(task, 18);
            let room = usize::from(width)
                .saturating_sub(10 + 2 + wrap::width(&task) + 2 + wrap::width(&clock) + 4);
            let doing = wrap::truncate(&c.status, room);
            let pad = room.saturating_sub(wrap::width(&doing));
            Line::from(vec![
                Span::styled(
                    format!("{:<9} ", wrap::truncate(who, 9)),
                    on(theme.role(who)),
                ),
                Span::styled(format!("{spin} "), on(theme.accent)),
                Span::styled(task, on(theme.fg)),
                Span::styled(format!("  {doing}"), on(theme.dim)),
                Span::styled(" ".repeat(pad), on(theme.dim)),
                Span::styled(clock, on(theme.dim)),
            ])
        })
        .collect()
}
