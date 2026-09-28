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

/// Whether the board shows: crew mode on the ledger, plan or not yet. It
/// used to wait for a plan, so a crew session that began with a question
/// looked like solo mode.
pub fn shown(view: &View) -> bool {
    // The workbench is a view of its own, in crew mode too.
    !view.ui.classic() && view.crew_mode() && view.workbench.is_none()
}

/// Rows the board wants out of `avail`, leaving the lead's chat room.
pub fn height(view: &View, avail: u16, width: u16, theme: Theme) -> u16 {
    if !shown(view) {
        return 0;
    }
    let inner = usize::from(plan_width(width).saturating_sub(3));
    let cap = usize::from(avail.saturating_sub(10 + TILES_H + 3));
    let plan = plan_view(view, theme, inner, cap).len() as u16 + 1;
    let lanes = view.crew.len().max(1) as u16 + 1;
    let want = TILES_H + plan.max(lanes) + 2;
    want.min(avail.saturating_sub(10))
        .max(TILES_H + 4)
        .min(avail)
}

/// Paint the board into `area`.
pub fn draw(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    if area.height < TILES_H + 3 {
        return;
    }
    let tiles = Rect {
        height: TILES_H,
        ..area
    };
    draw_tiles(frame, tiles, view, theme);
    let rest = Rect {
        y: area.y + TILES_H,
        height: area.height - TILES_H,
        ..area
    };
    let plan_w = plan_width(rest.width);
    let plan = Rect {
        width: plan_w,
        ..rest
    };
    let lanes = Rect {
        x: rest.x + plan_w + 1,
        width: rest.width.saturating_sub(plan_w + 1),
        ..rest
    };
    let inner = usize::from(plan_w.saturating_sub(3));
    let cap = usize::from(plan.height.saturating_sub(3));
    boxed(
        frame,
        plan,
        "PLAN",
        plan_view(view, theme, inner, cap),
        view,
        theme,
    );
    boxed(
        frame,
        lanes,
        "LANES",
        lane_rows(view, theme, lanes.width),
        view,
        theme,
    );
}

/// The plan's share of the board's width.
fn plan_width(total: u16) -> u16 {
    (total * 11 / 20).max(30).min(total.saturating_sub(24))
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
    let w = area.width / 4;
    let dim = Style::default().fg(theme.dim).bg(theme.bg);
    let fg = Style::default()
        .fg(theme.fg)
        .bg(theme.bg)
        .add_modifier(Modifier::BOLD);
    let tiles: [(&str, Vec<Span<'static>>, Vec<Span<'static>>); 4] = [
        spend_tile(view, theme, dim, fg, w),
        tasks_tile(view, theme, dim, fg, w),
        patch_tile(view, theme, dim, fg),
        time_tile(view, dim, fg),
    ];
    for (i, (title, value, detail)) in tiles.into_iter().enumerate() {
        let x = area.x + w * i as u16;
        let width = if i == 3 {
            area.width - w * 3
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
        vec![Span::styled("no budget · $ for detail", dim)]
    };
    ("SPEND", value, detail)
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

fn patch_tile(view: &View, _theme: Theme, dim: Style, fg: Style) -> Tile {
    match &view.patch_view {
        None => (
            "PATCH",
            vec![Span::styled("none open", dim)],
            vec![Span::styled("none yet", dim)],
        ),
        Some(p) => {
            let short = p.branch.rsplit('-').next().unwrap_or(&p.branch);
            let waiting: Vec<&str> = p
                .tasks
                .iter()
                .filter(|t| !p.landed.contains(t))
                .map(String::as_str)
                .collect();
            let detail = if waiting.is_empty() {
                format!("{} landed · lands next", p.landed.len())
            } else {
                format!("waits on {}", waiting.join(", "))
            };
            (
                "PATCH",
                vec![
                    Span::styled(format!("patch-{short}"), fg),
                    Span::styled(format!(" → {}", p.target), dim),
                ],
                vec![Span::styled(detail, dim)],
            )
        }
    }
}

fn time_tile(view: &View, dim: Style, fg: Style) -> Tile {
    let secs = view
        .crew_started_ms
        .map(|s| view.now_ms.saturating_sub(s) / 1000);
    let value = match secs {
        Some(s) => vec![Span::styled(fmt_elapsed(s), fg)],
        None => vec![Span::styled("idle", dim)],
    };
    (
        "TIME",
        value,
        vec![Span::styled(
            format!("{} model calls this turn", view.turn_calls),
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
    let open: Vec<&str> = p
        .tasks
        .iter()
        .filter(|t| !p.landed.contains(t))
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

/// Box rows, and the columns between a box and the next: `──┬─▶`.
const BOX_H: usize = 4;
const GAP: usize = 5;

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
    if box_w < 16 {
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
        let said = wrap::truncate(&format!("   {words}"), inner);
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
        if t.status == "blocked" && wrap::width(&format!("   {words}")) > inner {
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
                    "──┬─▶"
                } else {
                    "────▶"
                },
                line,
            );
            let _ = first;
            for (k, &c) in ks.iter().enumerate().skip(1) {
                let cy = row[c] + 1;
                let glyph = if c == *last {
                    "  └─▶"
                } else {
                    "  ├─▶"
                };
                put(&mut grid, cy, x0, glyph, line);
                // The trunk down to it.
                let from = row[ks[k - 1]] + 2;
                for yy in from..cy {
                    put(&mut grid, yy, x0 + 2, "│", line);
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

/// One row per worker: who is acting, on what, doing what, for how long.
fn lane_rows(view: &View, theme: Theme, width: u16) -> Vec<Line<'static>> {
    let dim = Style::default().fg(theme.dim).bg(theme.bg);
    if view.crew.is_empty() {
        return vec![Line::from(Span::styled("no one is working right now", dim))];
    }
    let spin = crate::activity::SPINNER[view.activity.frame % crate::activity::SPINNER.len()];
    view.crew
        .iter()
        .map(|c| {
            let who = if c.acting.is_empty() {
                c.role.as_str()
            } else {
                c.acting.as_str()
            };
            let secs = view.now_ms.saturating_sub(c.started_ms) / 1000;
            let clock = format!("  {}", fmt_elapsed(secs));
            let task = wrap::truncate(&c.label, 18);
            let room = usize::from(width)
                .saturating_sub(10 + 2 + wrap::width(&task) + 2 + wrap::width(&clock) + 4);
            let doing = wrap::truncate(&c.status, room);
            let pad = room.saturating_sub(wrap::width(&doing));
            Line::from(vec![
                Span::styled(
                    format!("{:<9} ", wrap::truncate(who, 9)),
                    Style::default().fg(theme.role(who)).bg(theme.bg),
                ),
                Span::styled(
                    format!("{spin} "),
                    Style::default().fg(theme.accent).bg(theme.bg),
                ),
                Span::styled(task, Style::default().fg(theme.fg).bg(theme.bg)),
                Span::styled(format!("  {doing}"), dim),
                Span::styled(" ".repeat(pad), dim),
                Span::styled(clock, dim),
            ])
        })
        .collect()
}
