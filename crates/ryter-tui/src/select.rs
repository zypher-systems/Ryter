//! Selecting text with the mouse: hold the left button and drag to
//! highlight, release to copy what is highlighted.
//!
//! A selection stays inside the pane the press landed in: the conversation's
//! text (not its timeline gutter), the composer, a card, or the whole screen
//! while a panel is open. In the conversation its rows are the document's
//! rows, so the highlight stays on the text while the reply streams or the
//! pane scrolls, and dragging past the pane's edge scrolls it and selects
//! on. Everywhere else its rows are the screen's.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::chat::layout;
use crate::theme::Theme;
use crate::view::View;

/// The most rows of the conversation one selection copies.
const MOST_ROWS: i64 = 20_000;

/// A drag in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// The cells it may reach.
    pub area: Rect,
    /// The conversation's pane, when the selection is in it: its rows are
    /// then rows of the document, not of the screen.
    pub chat: Option<Rect>,
    /// A rectangle (Alt held at the press), not lines of text.
    pub block: bool,
    /// Where the button went down: column, row.
    pub anchor: (u16, i64),
    /// Where the pointer is.
    pub head: (u16, i64),
    /// The pointer has left the cell it was pressed in. Until then it is
    /// a click, and nothing is highlighted or copied.
    pub moved: bool,
    /// The pointer is held past the conversation's top (-1) or bottom (1)
    /// edge: the pane scrolls that way and the selection grows with it.
    pub pull: i8,
    /// When the pane last scrolled for `pull`.
    pub pulled_at: u64,
}

/// One row of a selection: its row, and the first and last column.
type RowSpan = (i64, u16, u16);

impl Selection {
    /// A press at `(column, row)` of the screen. `top` is the document row
    /// at the top of the conversation's pane.
    pub fn begin(
        area: Rect,
        chat: Option<Rect>,
        block: bool,
        column: u16,
        row: u16,
        top: usize,
    ) -> Self {
        let mut s = Self {
            area,
            chat,
            block,
            anchor: (0, 0),
            head: (0, 0),
            moved: false,
            pull: 0,
            pulled_at: 0,
        };
        s.anchor = s.point(column, row, top);
        s.head = s.anchor;
        s
    }

    /// The pointer moved to `(column, row)` of the screen.
    pub fn drag(&mut self, column: u16, row: u16, top: usize) {
        self.pull = match () {
            () if self.chat.is_none() => 0,
            () if row < self.area.y => -1,
            () if row >= self.area.y + self.area.height => 1,
            () => 0,
        };
        let head = self.point(column, row, top);
        if head != self.anchor {
            self.moved = true;
        }
        self.head = head;
    }

    /// The pane scrolled a row for [`Self::pull`]: the selection reaches
    /// the row that came into view. `rows` is the document's height.
    pub fn pulled(&mut self, rows: usize) {
        let last = i64::try_from(rows).unwrap_or(i64::MAX).saturating_sub(1);
        self.head.1 = (self.head.1 + i64::from(self.pull)).clamp(0, last.max(0));
        self.moved = true;
    }

    /// A screen position as a point of the selection, held inside its area.
    fn point(&self, column: u16, row: u16, top: usize) -> (u16, i64) {
        let right = self.area.x + self.area.width.saturating_sub(1);
        let bottom = self.area.y + self.area.height.saturating_sub(1);
        let column = column.clamp(self.area.x, right);
        let row = row.clamp(self.area.y, bottom);
        let row = if self.chat.is_some() {
            i64::try_from(top).unwrap_or(i64::MAX) + i64::from(row - self.area.y)
        } else {
            i64::from(row)
        };
        (column, row)
    }

    /// Its rows, top to bottom, each with its first and last column.
    fn rows(&self) -> Vec<RowSpan> {
        let (left, right) = (self.area.x, self.area.x + self.area.width.saturating_sub(1));
        let (a, b) = if (self.anchor.1, self.anchor.0) <= (self.head.1, self.head.0) {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        };
        let last = b.1.min(a.1 + MOST_ROWS);
        if self.block {
            let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
            return (a.1..=last).map(|row| (row, x0, x1)).collect();
        }
        (a.1..=last)
            .map(|row| {
                let x0 = if row == a.1 { a.0 } else { left };
                let x1 = if row == b.1 { b.0 } else { right };
                (row, x0, x1)
            })
            .collect()
    }

    /// Its rows as rows of the screen, leaving out what is scrolled out of
    /// sight. `top` is the document row at the top of the pane.
    fn on_screen(&self, top: usize) -> Vec<(u16, u16, u16)> {
        let top = i64::try_from(top).unwrap_or(i64::MAX);
        self.rows()
            .into_iter()
            .filter_map(|(row, x0, x1)| {
                let y = if self.chat.is_some() {
                    i64::from(self.area.y) + (row - top)
                } else {
                    row
                };
                let inside = y >= i64::from(self.area.y)
                    && y < i64::from(self.area.y) + i64::from(self.area.height);
                inside.then(|| (u16::try_from(y).unwrap_or(0), x0, x1))
            })
            .collect()
    }

    /// Paint the highlight over a drawn frame.
    pub fn paint(&self, buf: &mut Buffer, top: usize, bg: Color) {
        if !self.moved {
            return;
        }
        for (y, x0, x1) in self.on_screen(top) {
            for x in x0..=x1 {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_bg(bg);
                }
            }
        }
    }

    /// The text it covers. `frame` is the screen as last drawn; a selection
    /// in the conversation is read from the document instead, so rows that
    /// have scrolled out of sight are in it.
    pub fn text(&self, view: &View, frame: &Buffer, theme: Theme) -> String {
        let rows = self.rows();
        let (Some(first), Some(last)) = (rows.first(), rows.last()) else {
            return String::new();
        };
        let page = Page {
            left: self.area.x,
            right: self.area.x + self.area.width.saturating_sub(1),
            block: self.block,
        };
        let Some(pane) = self.chat else {
            return page.text(frame, &rows, 0, 0, &[]);
        };
        let (from, to) = (
            usize::try_from(first.0).unwrap_or(0),
            usize::try_from(last.0).unwrap_or(0) + 1,
        );
        // The rows as the pane draws them, on a page of their own.
        let width = usize::from(pane.width).saturating_sub(2).max(12);
        let mut frames: Vec<u16> = Vec::new();
        let lines: Vec<Line<'static>> =
            layout::rows(view, width, usize::from(pane.height), theme, from, to)
                .into_iter()
                .map(|l| {
                    let (line, frame) = without_frame(l, theme);
                    frames.push(frame);
                    let mut spans = vec![Span::raw(" ")];
                    spans.extend(line.spans);
                    Line::from(spans)
                })
                .collect();
        let height = u16::try_from(lines.len()).unwrap_or(u16::MAX);
        let sheet = Rect::new(0, 0, pane.width, height);
        let mut buf = Buffer::empty(sheet);
        Paragraph::new(lines).render(sheet, &mut buf);
        page.text(&buf, &rows, first.0, pane.x, &frames)
    }
}

/// A row of a code block without its frame: the box's sides, its line
/// numbers and the mark of a wrapped line become spaces, and the rule above
/// and below it an empty row. They are drawn around the code, and are not
/// the code: copied with it, three lines of a program came out as
/// `│ fn main() {    │`. Returns the row, and how many columns of frame
/// were on its left; [`RULE`] for the rule itself, which is no row of text
/// at all.
///
/// The frame is known by where it is and how it is drawn: the first span
/// that is `│ ` on the code's background, the dim spans after it, and the
/// ` │` that ends the row.
fn without_frame(line: Line<'static>, theme: Theme) -> (Line<'static>, u16) {
    let frame = |s: &Span<'_>| s.style.bg == Some(theme.code_bg) && s.style.fg == Some(theme.dim);
    let width = |s: &Span<'_>| crate::chat::wrap::width(&s.content);
    let blank = |s: &Span<'static>| Span::styled(" ".repeat(width(s)), s.style);
    let mut spans = line.spans;
    let Some(at) = spans.iter().position(frame) else {
        return (Line::from(spans), 0);
    };
    if spans[at].content.starts_with(['╭', '╰']) {
        for s in &mut spans[at..] {
            *s = blank(s);
        }
        return (Line::from(spans), RULE);
    }
    if spans[at].content != "│ " {
        return (Line::from(spans), 0);
    }
    // The side, then the line number or the wrap mark.
    let mut left = 0;
    let mut i = at;
    while i < spans.len()
        && frame(&spans[i])
        && (i == at
            || spans[i].content == "↳ "
            || spans[i]
                .content
                .chars()
                .all(|c| c == ' ' || c.is_ascii_digit()))
    {
        left += width(&spans[i]);
        spans[i] = blank(&spans[i]);
        i += 1;
    }
    if let Some(last) = spans.last_mut() {
        if last.content == " │" && frame(last) {
            *last = blank(last);
        }
    }
    (Line::from(spans), u16::try_from(left).unwrap_or(0))
}

/// In place of a frame's width: the row is the rule above or below a code
/// block, and is left out of what is copied.
const RULE: u16 = u16::MAX;

/// The pane a selection's text is read in: its first and last column, and
/// whether the selection is a rectangle.
struct Page {
    left: u16,
    right: u16,
    block: bool,
}

impl Page {
    /// The text in `rows` of `buf`, a line each, without the spaces a row
    /// is padded with on its right. `top` and `from` are the row and the
    /// column the buffer starts at; `frames` is how many columns of a code
    /// block's frame each row had on its left.
    ///
    /// Text set in from the pane's edge is copied from where it starts: an
    /// answer under its speaker, a code block inside its frame. The margin
    /// every row shares is left out, and a code row's frame with it, so
    /// code keeps its own indentation and nothing else's. A rectangle is
    /// the cells it covers, and loses only a margin all of it shares.
    fn text(&self, buf: &Buffer, rows: &[RowSpan], top: i64, from: u16, frames: &[u16]) -> String {
        // A row's cells from `x0` to `x1`: a wide character takes two
        // cells, and the second is not another character.
        let cells = |y: u16, x0: u16, x1: u16| -> String {
            let mut line = String::new();
            let mut x = x0;
            while x <= x1 {
                let Some(cell) = buf.cell((x.saturating_sub(from), y)) else {
                    break;
                };
                line.push_str(cell.symbol());
                x += u16::try_from(crate::chat::wrap::width(cell.symbol()))
                    .unwrap_or(1)
                    .max(1);
            }
            line
        };
        let lead = |l: &str| l.len() - l.trim_start_matches(' ').len();
        let at = |i: usize| -> Option<u16> { u16::try_from(rows[i].0 - top).ok() };
        let rule = |i: usize| !self.block && frames.get(i) == Some(&RULE);
        let frame = |i: usize| frames.get(i).copied().filter(|f| *f != RULE).unwrap_or(0);
        // The margin the rows share, past each one's frame.
        let margin = if self.block {
            0
        } else {
            (0..rows.len())
                .filter_map(|i| {
                    let whole = cells(at(i)?, self.left, self.right);
                    (!whole.trim().is_empty())
                        .then(|| lead(&whole).saturating_sub(usize::from(frame(i))))
                })
                .min()
                .unwrap_or(0)
        };
        let margin = u16::try_from(margin).unwrap_or(0);
        let mut out: Vec<String> = Vec::with_capacity(rows.len());
        for (i, &(_, x0, x1)) in rows.iter().enumerate() {
            let Some(y) = at(i) else {
                continue;
            };
            if rule(i) {
                continue;
            }
            let x0 = if self.block {
                x0
            } else {
                x0.max(self.left + frame(i) + margin)
            };
            out.push(cells(y, x0, x1).trim_end().to_string());
        }
        while out.last().is_some_and(String::is_empty) {
            out.pop();
        }
        while out.first().is_some_and(String::is_empty) {
            out.remove(0);
        }
        let shared = if self.block {
            out.iter()
                .filter(|l| !l.is_empty())
                .map(|l| lead(l))
                .min()
                .unwrap_or(0)
        } else {
            0
        };
        out.iter()
            .map(|l| l.get(shared..).unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// What a copy is called in the line that says it happened.
pub fn describe(text: &str) -> String {
    let lines = text.lines().count();
    if lines > 1 {
        format!("copied {lines} lines")
    } else {
        let n = text.chars().count();
        format!("copied {n} character{}", if n == 1 { "" } else { "s" })
    }
}
