//! Document assembly and viewport slicing (`R-SCROLL-*`, `R-PERF-02/04`).

use std::rc::Rc;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::cache::{Entry, Key};
use super::{MessageKind, RenderOpts, SystemLevel, lang_hint_from_tool, render_message, wrap};
use crate::theme::Theme;
use crate::view::View;
use crate::view::scroll::Resolved;

/// Columns the ledger's timeline takes on the left: `01:42  ●  `.
pub const GUTTER: usize = 10;

/// What the ledger's gutter shows beside a message.
#[derive(Debug, Clone, PartialEq)]
enum Gutter {
    /// Classic layout: no gutter.
    None,
    /// A user message: the time and `●` on its first row.
    User(String),
    /// A model's first row: `◆` in its color.
    Speaker(ratatui::style::Color),
    /// A tool step: `├─`.
    Tool,
    /// A notice: `·`, `!` for a warning, `✕` for an error, in its color.
    Note(&'static str, ratatui::style::Color),
    /// More of the same speaker: the spine only.
    Line,
    /// The turn's closing line: `└─`.
    End,
    /// A folded turn: its time and `●`, dimmed.
    Folded(String, ratatui::style::Color),
    /// The reasoning pane after a finished turn: the gutter's width, blank.
    Blank,
}

/// One rendered message with its document offset.
struct Placed {
    start: usize,
    separator: bool,
    /// On the ledger, the separator row carries the spine: the turn goes on.
    spine: bool,
    gutter: Gutter,
    entry: Rc<Entry>,
}

/// A frame of the chat pane.
#[derive(Debug)]
pub struct ChatFrame {
    /// Exactly `height` rows.
    pub lines: Vec<Line<'static>>,
    /// Sticky header rows to paint over rows 0 and 1.
    pub sticky: Vec<Line<'static>>,
    /// Scroll resolution.
    pub resolved: Resolved,
    /// Total document rows.
    pub doc_rows: usize,
    /// The document row of the live status row, while a turn runs.
    pub status_row: Option<usize>,
    /// The document row of the reasoning pane's header, when it is open.
    pub pane_header: Option<usize>,
}

/// What `place` laid out: the rows, their count, and where the status row
/// and the reasoning pane's header are.
struct Laid {
    placed: Vec<Placed>,
    rows: usize,
    status_at: Option<usize>,
    pane_at: Option<usize>,
}

fn flags(opts: &RenderOpts) -> u64 {
    let mut f = 0u64;
    if opts.timestamps {
        f |= 1;
    }
    if opts.line_numbers {
        f |= 2;
    }
    if opts.continuation {
        f |= 4;
    }
    if opts.diff_rows == usize::MAX {
        f |= 8;
    }
    if opts.ledger {
        f |= 16;
    }
    if let Some(h) = &opts.lang_hint {
        let mut hash: u64 = 1469;
        for b in h.bytes() {
            hash = hash.wrapping_mul(31).wrapping_add(u64::from(b));
        }
        f |= hash << 8;
    }
    let mut uh: u64 = 7;
    for b in opts.username.bytes() {
        uh = uh.wrapping_mul(131).wrapping_add(u64::from(b));
    }
    f ^ (uh << 3)
}

/// Whether a finished turn shows as one line: on the ledger, every turn
/// but the latest, unless `^O` shows everything whole.
fn folds(view: &View, turn: u64, latest: u64) -> bool {
    !view.ui.classic()
        && !view.diffs_expanded
        && turn != latest
        && turn != 0
        && view.messages.iter().any(|m| {
            m.turn == turn
                && matches!(
                    m.kind,
                    MessageKind::System {
                        level: SystemLevel::Receipt
                    }
                )
        })
}

/// A folded turn: what was asked, led by dots to what it came to.
fn fold_line(view: &View, turn: u64, width: usize, theme: Theme) -> Line<'static> {
    let asked = view
        .messages
        .iter()
        .find(|m| m.turn == turn && matches!(m.kind, MessageKind::User))
        .map(|m| m.collapsed(width))
        .unwrap_or_default();
    let came = view
        .messages
        .iter()
        .rev()
        .find(|m| {
            m.turn == turn
                && matches!(
                    m.kind,
                    MessageKind::System {
                        level: SystemLevel::Receipt
                    }
                )
        })
        .map(|m| m.body.trim().to_string())
        .unwrap_or_default();
    let came = format!("{}  ▸", wrap::truncate(&came, width / 2));
    let came_w = wrap::width(&came);
    let asked = wrap::truncate(&asked, width.saturating_sub(came_w + 6));
    let dots = width.saturating_sub(wrap::width(&asked) + came_w + 2);
    Line::from(vec![
        Span::styled(asked, theme.muted()),
        Span::styled(
            format!(" {} ", "·".repeat(dots)),
            Style::default().fg(theme.dim).bg(theme.bg),
        ),
        Span::styled(came, theme.muted()),
    ])
}

fn place(view: &View, width: usize, theme: Theme, pane_cap: usize) -> Laid {
    let mut placed = Vec::with_capacity(view.messages.len());
    let mut row = 0usize;
    let mut cache = view.cache.borrow_mut();
    let mut tops = view.scroll.turn_tops.borrow_mut();
    tops.clear();
    let mut last_hint: Option<String> = None;
    let ledger = !view.ui.classic();
    let width = if ledger {
        width.saturating_sub(GUTTER).max(12)
    } else {
        width
    };
    let latest = view.messages.iter().map(|m| m.turn).max().unwrap_or(0);
    let mut folded_done: Vec<u64> = Vec::new();
    for (i, msg) in view.messages.iter().enumerate() {
        if folds(view, msg.turn, latest) {
            if folded_done.contains(&msg.turn) {
                continue;
            }
            folded_done.push(msg.turn);
            tops.push((msg.turn, row));
            let time = view
                .messages
                .iter()
                .find(|m| m.turn == msg.turn && matches!(m.kind, MessageKind::User))
                .map(|m| stamp(view, m))
                .unwrap_or_default();
            placed.push(Placed {
                start: row,
                separator: false,
                spine: false,
                // In the color of the hat the turn ran in.
                gutter: Gutter::Folded(
                    time,
                    view.messages
                        .iter()
                        .find(|m| m.turn == msg.turn && !matches!(m.kind, MessageKind::User))
                        .and_then(|m| m.meta.hat)
                        .map_or(theme.dim, |h| theme.mode(h)),
                ),
                entry: Rc::new(Entry {
                    lines: vec![fold_line(view, msg.turn, width, theme)],
                    bytes: 0,
                }),
            });
            row += 1;
            continue;
        }
        // A model step that only called tools streams no text: nothing to
        // show, not an empty header.
        if matches!(msg.kind, MessageKind::Assistant { .. }) && msg.body.trim().is_empty() {
            continue;
        }
        let prev = i.checked_sub(1).map(|p| &view.messages[p]);
        let continuation = prev.is_some_and(|p| p.same_speaker(msg));
        // On the ledger a model is named when it starts to speak; its
        // later steps keep the `◆` mark in the gutter but not the header.
        // Another model speaking in between (a hat with its own model)
        // names it again: unnamed, its words read as the other's. Each
        // step's cost is in the turn's closing line.
        let speaks = |m: &super::Message| {
            matches!(m.kind, MessageKind::Assistant { .. }) && !m.body.trim().is_empty()
        };
        let named_before = ledger
            && speaks(msg)
            && view.messages[..i]
                .iter()
                .rev()
                .take_while(|m| m.turn == msg.turn)
                .find(|m| speaks(m))
                .is_some_and(|m| m.same_speaker(msg));
        let both_tools = prev.is_some_and(|p| {
            matches!(p.kind, MessageKind::Tool { .. })
                && matches!(msg.kind, MessageKind::Tool { .. })
        });
        let receipt = matches!(
            msg.kind,
            MessageKind::System {
                level: SystemLevel::Receipt
            }
        );
        // The spine runs through the turn; a new question starts clear of it.
        let spine = ledger && !matches!(msg.kind, MessageKind::User);
        let separator = i > 0 && !continuation && !both_tools && (!ledger || !receipt);
        if separator {
            row += 1;
        }
        if matches!(msg.kind, MessageKind::User) || !tops.iter().any(|(t, _)| *t == msg.turn) {
            if !tops.iter().any(|(t, _)| *t == msg.turn) {
                tops.push((msg.turn, row));
            } else if matches!(msg.kind, MessageKind::User) {
                if let Some(slot) = tops.iter_mut().find(|(t, _)| *t == msg.turn) {
                    slot.1 = row;
                }
            }
        }
        let opts = RenderOpts {
            width,
            timestamps: view.ui.timestamps,
            line_numbers: view.ui.line_numbers,
            username: view.username.clone(),
            lang_hint: last_hint.clone(),
            continuation: continuation || named_before,
            diff_rows: if view.diffs_expanded {
                usize::MAX
            } else {
                super::diff::DEFAULT_ROWS
            },
            ledger,
        };
        let key = Key {
            id: msg.id,
            rev: msg.rev,
            width: u16::try_from(width).unwrap_or(u16::MAX),
            generation: theme.generation ^ view.theme_generation,
            flags: flags(&opts),
        };
        let entry = cache.get_or_insert(key, || render_message(msg, &opts, theme));
        let gutter = if !ledger {
            Gutter::None
        } else {
            match &msg.kind {
                MessageKind::User => Gutter::User(stamp(view, msg)),
                MessageKind::Tool { .. } => Gutter::Tool,
                MessageKind::System {
                    level: SystemLevel::Receipt,
                } => Gutter::End,
                MessageKind::System { level } => Gutter::Note(
                    match level {
                        SystemLevel::Warn => "!",
                        SystemLevel::Error => "✕",
                        SystemLevel::Report { .. } => "▣",
                        _ => "·",
                    },
                    msg.accent(theme),
                ),
                _ if continuation => Gutter::Line,
                _ if named_before => Gutter::Speaker(msg.accent(theme)),
                _ => Gutter::Speaker(msg.accent(theme)),
            }
        };
        placed.push(Placed {
            start: row,
            separator,
            spine,
            gutter,
            entry: entry.clone(),
        });
        row += entry.rows();
        last_hint = lang_hint_from_tool(msg).or(last_hint);
        if matches!(msg.kind, MessageKind::Assistant { .. } | MessageKind::User) {
            last_hint = None;
        }
    }
    // While a turn runs, the conversation ends on a row that moves: the
    // spinner and what the model is doing, where the eye is. The turn's
    // closing line takes its place when it ends.
    let mut status_at = None;
    let mut pane_at = None;
    if view.busy {
        let line = status_row(view, width, theme);
        status_at = Some(row);
        placed.push(Placed {
            start: row,
            separator: false,
            spine: ledger,
            gutter: if ledger { Gutter::Line } else { Gutter::None },
            entry: Rc::new(Entry {
                bytes: 0,
                lines: vec![line],
            }),
        });
        row += 1;
    }
    // The reasoning pane is part of the conversation: under the status row
    // while the turn runs, under the turn's closing line after, where `^r`
    // or a click on the row opened it.
    // Not on the workbench: that screen keeps the strip as its one
    // reasoning surface, and never publishes the row to click.
    if view.activity.mode == crate::activity::Mode::Expanded
        && view.activity.has_history
        && view.workbench.is_none()
    {
        let lines = reasoning_pane(view, width, pane_cap, theme);
        pane_at = Some(row);
        let n = lines.len();
        placed.push(Placed {
            start: row,
            separator: false,
            spine: ledger && view.busy,
            gutter: match (ledger, view.busy) {
                (false, _) => Gutter::None,
                (true, true) => Gutter::Line,
                (true, false) => Gutter::Blank,
            },
            entry: Rc::new(Entry { bytes: 0, lines }),
        });
        row += n;
    }
    Laid {
        placed,
        rows: row,
        status_at,
        pane_at,
    }
}

/// The reasoning pane: a header, `reasoning · 1:40   ^r close ▴`, then the
/// turn's reasoning, following its tail unless the user scrolled, dim and
/// italic; at most `cap` rows. While the model has sent nothing it says
/// so; after a turn with no reasoning it says that.
fn reasoning_pane(view: &View, width: usize, cap: usize, theme: Theme) -> Vec<Line<'static>> {
    let a = &view.activity;
    let dim = Style::default().fg(theme.dim).bg(theme.bg);
    let dim_italic = dim.add_modifier(Modifier::ITALIC);
    let secs = a.elapsed_ms / 1000;
    let clock = format!("{}:{:02}", secs / 60, secs % 60);
    let text = view
        .reasoning
        .get(&a.turn)
        .map(String::as_str)
        .unwrap_or("");
    let body_w = width.saturating_sub(2).max(8);
    let rows: Vec<String> = if text.trim().is_empty() {
        if a.busy() {
            vec![
                format!("waiting for the model · nothing streamed yet · {clock}"),
                "some models think on the server and send nothing until they answer".into(),
            ]
        } else {
            vec!["no reasoning stream from this model".into()]
        }
    } else {
        wrap::wrap_plain(text, body_w)
    };
    let body_h = rows.len().min(cap.saturating_sub(1).max(1));
    let max_start = rows.len().saturating_sub(body_h);
    let start = a.scroll.map_or(max_start, |s| s.min(max_start));
    let head = format!("reasoning · {clock}   ^r close ▴");
    let mut lines = vec![Line::from(Span::styled(wrap::truncate(&head, width), dim))];
    for r in rows.iter().skip(start).take(body_h) {
        lines.push(Line::from(vec![
            Span::styled(" ", theme.body()),
            Span::styled(r.clone(), dim_italic),
        ]));
    }
    lines
}

/// The live status row of a running turn: the spinner in the hat's color,
/// then `thinking · 1:40 · 12k tokens`, `running cargo test · 0:03`,
/// `waiting for the model · 0:42`, or `waiting for you`.
fn status_row(view: &View, width: usize, theme: Theme) -> Line<'static> {
    let a = &view.activity;
    let spinner = crate::activity::SPINNER[a.frame % crate::activity::SPINNER.len()];
    let accent = Style::default()
        .fg(theme.mode(view.mode))
        .bg(theme.bg)
        .add_modifier(Modifier::BOLD);
    let text = a.status(width.saturating_sub(3));
    Line::from(vec![
        Span::styled(spinner.to_string(), accent),
        Span::styled(" ", theme.body()),
        Span::styled(
            wrap::truncate(&text, width.saturating_sub(3)),
            theme.muted(),
        ),
    ])
}

/// Assemble the visible frame.
pub fn frame(view: &View, width: usize, height: usize, theme: Theme) -> ChatFrame {
    let width = width.max(12);
    // The pane takes at most a third of the chat's height.
    let pane_cap = (height / 3).clamp(3, 12);
    let Laid {
        placed,
        rows: doc_rows,
        status_at,
        pane_at,
    } = place(view, width, theme, pane_cap);
    // On the ledger a new turn is pinned a few rows down, so the turns
    // folded above it stay in sight: they are one line each, there to be
    // glanced at.
    let anchor_top = view.anchor_top().map(|t| {
        if view.ui.classic() {
            t
        } else {
            t.saturating_sub(4)
        }
    });
    let resolved = view.scroll.resolve(doc_rows, height, anchor_top, view.busy);
    let off = resolved.offset;
    let end = off + height;
    let blank = || Line::from(Span::styled(String::new(), theme.body()));
    let mut lines = rows_between(&placed, off, end, view, theme);
    while lines.len() < height {
        lines.push(blank());
    }
    lines.truncate(height);
    let mut sticky = Vec::new();
    if let Some(s) = resolved.sticky {
        if let Some(m) = view.anchor_message() {
            let bg = theme.sticky_bg;
            let name_style = Style::default()
                .fg(theme.user)
                .bg(bg)
                .add_modifier(Modifier::BOLD);
            let name = m.speaker(&view.username);
            let right = format!("↑ {}", s.rows_above);
            // On the ledger the pinned question sits on the timeline too.
            let lead = if view.ui.classic() {
                String::new()
            } else {
                format!("{:<5}  ●  ", stamp(view, m))
            };
            let name = format!("{lead}{name}");
            let used = wrap::width(&name) + 2;
            let room = width.saturating_sub(used + wrap::width(&right) + 3);
            let body = m.collapsed(room);
            let pad = width.saturating_sub(used + wrap::width(&body) + wrap::width(&right) + 1);
            sticky.push(Line::from(vec![
                Span::styled(name, name_style),
                Span::styled("  ", Style::default().bg(bg)),
                Span::styled(body, Style::default().fg(theme.fg).bg(bg)),
                Span::styled(" ".repeat(pad), Style::default().bg(bg)),
                Span::styled(right, Style::default().fg(theme.dim).bg(bg)),
                Span::styled(" ", Style::default().bg(bg)),
            ]));
            sticky.push(Line::from(Span::styled(
                "‥".repeat(width),
                Style::default().fg(theme.dim).bg(theme.bg),
            )));
        }
    }
    ChatFrame {
        lines,
        sticky,
        resolved,
        doc_rows,
        status_row: status_at,
        pane_header: pane_at,
    }
}

/// The document's rows from `off` up to `end`, as the pane draws them:
/// each message's rows behind its gutter, and the row between messages.
fn rows_between(
    placed: &[Placed],
    off: usize,
    end: usize,
    view: &View,
    theme: Theme,
) -> Vec<Line<'static>> {
    let blank = || Line::from(Span::styled(String::new(), theme.body()));
    let ledger = !view.ui.classic();
    let dim = Style::default().fg(theme.dim).bg(theme.bg);
    let spine_row = || Line::from(Span::styled(format!("{}│", " ".repeat(7)), dim));
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(end.saturating_sub(off));
    for p in placed {
        let sep_row = p.start.checked_sub(1);
        if p.separator {
            if let Some(r) = sep_row {
                if r >= off && r < end {
                    lines.push(if p.spine { spine_row() } else { blank() });
                }
            }
        }
        let p_end = p.start + p.entry.rows();
        if p_end <= off || p.start >= end {
            continue;
        }
        let from = off.saturating_sub(p.start);
        let to = (end - p.start).min(p.entry.rows());
        for (i, l) in p.entry.lines[from..to].iter().enumerate() {
            if !ledger || p.gutter == Gutter::None {
                lines.push(l.clone());
                continue;
            }
            let first = from + i == 0;
            let mut spans = gutter_spans(&p.gutter, first, theme);
            spans.extend(l.spans.iter().cloned());
            lines.push(Line::from(spans));
        }
    }
    lines
}

/// Rows `from` up to `to` of the document a pane `width` wide and `height`
/// tall is showing, in sight or not: what a selection that has scrolled
/// reads its text from. A row past the document's end is not there.
pub fn rows(
    view: &View,
    width: usize,
    height: usize,
    theme: Theme,
    from: usize,
    to: usize,
) -> Vec<Line<'static>> {
    let pane_cap = (height / 3).clamp(3, 12);
    let laid = place(view, width.max(12), theme, pane_cap);
    let to = to.min(laid.rows);
    let mut lines = rows_between(&laid.placed, from, to, view, theme);
    lines.resize(
        to.saturating_sub(from),
        Line::from(Span::styled(String::new(), theme.body())),
    );
    lines
}

/// A question's time in the gutter, unless `[ui] timestamps` is off.
fn stamp(view: &View, m: &super::Message) -> String {
    if view.ui.timestamps {
        m.at.hhmm()
    } else {
        String::new()
    }
}

/// The ledger's left columns for one row of a message.
fn gutter_spans(g: &Gutter, first: bool, theme: Theme) -> Vec<Span<'static>> {
    let dim = Style::default().fg(theme.dim).bg(theme.bg);
    let pad = |s: &str| Span::styled(s.to_string(), dim);
    let spine = || vec![pad("       │  ")];
    if !first {
        return match g {
            Gutter::None => Vec::new(),
            Gutter::Folded(..) | Gutter::Blank => vec![pad("          ")],
            _ => spine(),
        };
    }
    match g {
        Gutter::None => Vec::new(),
        Gutter::Blank => vec![pad("          ")],
        Gutter::User(t) => vec![
            Span::styled(format!("{t:<5}  "), dim),
            Span::styled("●", Style::default().fg(theme.user).bg(theme.bg)),
            pad("  "),
        ],
        Gutter::Folded(t, c) => vec![
            Span::styled(format!("{t:<5}  "), dim),
            Span::styled("●", Style::default().fg(*c).bg(theme.bg)),
            pad("  "),
        ],
        Gutter::Speaker(c) => vec![
            pad("       "),
            Span::styled("◆", Style::default().fg(*c).bg(theme.bg)),
            pad("  "),
        ],
        Gutter::Tool => vec![pad("       ├─ ")],
        Gutter::End => vec![pad("       └─ ")],
        Gutter::Note(mark, c) => vec![
            pad("       "),
            Span::styled(*mark, Style::default().fg(*c).bg(theme.bg)),
            pad("  "),
        ],
        Gutter::Line => spine(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn view_with_turns(n: usize) -> View {
        let mut v = View::new("x".into(), "m".into(), "p".into());
        for i in 0..n {
            v.submit_user(format!("question {i}"), format!("question {i}"));
            v.on_token(&format!("answer {i}\n").repeat(6));
            v.busy = false;
            v.activity
                .finish(crate::activity::Verb::Done, Some(0), Some(0));
        }
        v
    }

    #[test]
    fn cache_serves_earlier_messages_on_second_frame() {
        let v = view_with_turns(5);
        let t = Theme::truecolor_dark();
        frame(&v, 60, 20, t);
        let misses = v.cache.borrow().misses;
        frame(&v, 60, 20, t);
        assert_eq!(v.cache.borrow().misses, misses);
        assert!(v.cache.borrow().hits >= misses);
    }

    #[test]
    fn sticky_header_appears_when_turn_overflows() {
        let mut v = view_with_turns(2);
        v.submit_user("the in-flight question".into(), "q".into());
        v.on_token(&"streaming line\n".repeat(40));
        let f = frame(&v, 60, 15, Theme::truecolor_dark());
        assert_eq!(f.lines.len(), 15);
        assert!(f.resolved.sticky.is_some());
        let s = text(&f.sticky[0]);
        assert!(s.starts_with("you  the in-flight question"), "{s}");
        assert!(s.contains("↑ "), "{s}");
        assert!(text(&f.sticky[1]).starts_with('‥'));
        // While the turn's content still fits, the header row is the first row.
        let mut v = view_with_turns(2);
        v.submit_user("short".into(), "q".into());
        v.on_token("one line");
        let f = frame(&v, 60, 15, Theme::truecolor_dark());
        assert!(
            text(&f.lines[0]).starts_with("you"),
            "{}",
            text(&f.lines[0])
        );
        assert!(f.sticky.is_empty());
    }

    #[test]
    fn detached_view_does_not_move_on_new_content() {
        let mut v = view_with_turns(6);
        let t = Theme::truecolor_dark();
        frame(&v, 60, 12, t);
        v.scroll.page_up(false);
        let before = frame(&v, 60, 12, t).resolved.offset;
        v.on_token(&"more\n".repeat(30));
        let after = frame(&v, 60, 12, t);
        assert_eq!(after.resolved.offset, before);
        assert!(after.resolved.new_rows > 0);
        v.scroll.to_bottom();
        let f = frame(&v, 60, 12, t);
        assert_eq!(f.resolved.offset, f.doc_rows - 12);
    }
}
