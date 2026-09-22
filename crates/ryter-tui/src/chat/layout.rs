//! Document assembly and viewport slicing (`R-SCROLL-*`, `R-PERF-02/04`).

use std::rc::Rc;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::cache::{Entry, Key};
use super::{MessageKind, RenderOpts, lang_hint_from_tool, render_message, wrap};
use crate::theme::Theme;
use crate::view::View;
use crate::view::scroll::Resolved;

/// One rendered message with its document offset.
struct Placed {
    start: usize,
    separator: bool,
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

fn place(view: &View, width: usize, theme: Theme) -> (Vec<Placed>, usize) {
    let mut placed = Vec::with_capacity(view.messages.len());
    let mut row = 0usize;
    let mut cache = view.cache.borrow_mut();
    let mut tops = view.scroll.turn_tops.borrow_mut();
    tops.clear();
    let mut last_hint: Option<String> = None;
    for (i, msg) in view.messages.iter().enumerate() {
        // A model step that only called tools streams no text: nothing to
        // show, not an empty header.
        if matches!(msg.kind, MessageKind::Assistant { .. }) && msg.body.trim().is_empty() {
            continue;
        }
        let prev = i.checked_sub(1).map(|p| &view.messages[p]);
        let continuation = prev.is_some_and(|p| p.same_speaker(msg));
        let both_tools = prev.is_some_and(|p| {
            matches!(p.kind, MessageKind::Tool { .. })
                && matches!(msg.kind, MessageKind::Tool { .. })
        });
        let separator = i > 0 && !continuation && !both_tools;
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
            continuation,
        };
        let key = Key {
            id: msg.id,
            rev: msg.rev,
            width: u16::try_from(width).unwrap_or(u16::MAX),
            generation: theme.generation ^ view.theme_generation,
            flags: flags(&opts),
        };
        let entry = cache.get_or_insert(key, || render_message(msg, &opts, theme));
        placed.push(Placed {
            start: row,
            separator,
            entry: entry.clone(),
        });
        row += entry.rows();
        last_hint = lang_hint_from_tool(msg).or(last_hint);
        if matches!(msg.kind, MessageKind::Assistant { .. } | MessageKind::User) {
            last_hint = None;
        }
    }
    (placed, row)
}

/// Assemble the visible frame.
pub fn frame(view: &View, width: usize, height: usize, theme: Theme) -> ChatFrame {
    let width = width.max(12);
    let (placed, doc_rows) = place(view, width, theme);
    let anchor_top = view.anchor_top();
    let resolved = view.scroll.resolve(doc_rows, height, anchor_top, view.busy);
    let off = resolved.offset;
    let end = off + height;
    let blank = || Line::from(Span::styled(String::new(), theme.body()));
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(height);
    for p in &placed {
        let sep_row = p.start.checked_sub(1);
        if p.separator {
            if let Some(r) = sep_row {
                if r >= off && r < end {
                    lines.push(blank());
                }
            }
        }
        let p_end = p.start + p.entry.rows();
        if p_end <= off || p.start >= end {
            continue;
        }
        let from = off.saturating_sub(p.start);
        let to = (end - p.start).min(p.entry.rows());
        for l in &p.entry.lines[from..to] {
            lines.push(l.clone());
        }
    }
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ryter_core::Phase;

    fn text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn view_with_turns(n: usize) -> View {
        let mut v = View::new(Phase::Build, "x".into(), "m".into(), "p".into());
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
