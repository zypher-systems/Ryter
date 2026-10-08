//! Selecting text with the mouse: hold the left button and drag to
//! highlight, release to copy what is highlighted.
//!
//! A selection stays inside the pane the press landed in: the conversation's
//! text (not its timeline gutter), the composer, a card, something drawn
//! over the conversation (the pinned question, the command palette), or the
//! whole screen while a panel is open. In the conversation its rows are the
//! document's rows, so the highlight stays on the text while the reply
//! streams or the pane scrolls, and dragging past the pane's edge scrolls
//! it and selects on. Everywhere else its rows are the screen's.
//!
//! What is highlighted is what is copied. A conversation's selection is
//! read from the document, so it is painted only on cells that show the
//! document: not on what is drawn over the pane, and not below its end.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::action::PanelId;
use crate::chat::{layout, markdown};
use crate::theme::Theme;
use crate::view::View;

/// The most rows of the conversation one selection copies.
const MOST_ROWS: i64 = 20_000;

/// What the press does if the pointer never leaves its cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    /// Open or close the reasoning pane.
    Pane,
    /// Open a card's panel.
    Open(PanelId),
}

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
    /// edge, or on something drawn over it there: the pane scrolls that
    /// way, and the selection ends at the row of the pane named by
    /// `pull_at`, whatever text is there by then.
    pub pull: i8,
    /// The row of the pane a pulled selection ends at, from its top.
    pub pull_at: u16,
    /// When the pane last scrolled for `pull`.
    pub pulled_at: u64,
    /// What the press does as a click.
    pub click: Option<Click>,
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
            pull_at: 0,
            pulled_at: 0,
            click: None,
        };
        s.anchor = s.point(column, row, top);
        s.head = s.anchor;
        s
    }

    /// The pointer moved to `(column, row)` of the screen.
    pub fn drag(&mut self, column: u16, row: u16, top: usize) {
        let bottom = self.area.height.saturating_sub(1);
        (self.pull, self.pull_at) = match () {
            () if self.chat.is_none() => (0, 0),
            () if row < self.area.y => (-1, 0),
            () if row >= self.area.y + self.area.height => (1, bottom),
            () => (0, 0),
        };
        self.reach(column, row, top);
    }

    /// The pointer moved onto `cover`, something drawn over the
    /// conversation: the pinned question at its top, the palette or the
    /// "new rows" mark at its bottom. That is past the text there, as the
    /// pane's edge is: the selection ends at the last row of text before
    /// the cover, and the pane scrolls toward it.
    pub fn drag_onto(&mut self, column: u16, cover: Rect, top: usize) {
        let middle = self.area.y + self.area.height / 2;
        let row = if cover.y < middle {
            self.pull = -1;
            cover.y + cover.height
        } else {
            self.pull = 1;
            cover.y.saturating_sub(1)
        };
        let row = row.clamp(
            self.area.y,
            self.area.y + self.area.height.saturating_sub(1),
        );
        self.pull_at = row - self.area.y;
        self.reach(column, row, top);
    }

    fn reach(&mut self, column: u16, row: u16, top: usize) {
        let head = self.point(column, row, top);
        if head != self.anchor || self.pull != 0 {
            self.moved = true;
        }
        self.head = head;
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

    /// Where it ends, with `top` the document row at the top of the pane.
    /// While it is being pulled that is a row of the pane, and so whatever
    /// document row is there now: counted from the scrolling instead, the
    /// two drifted apart when the pane had no further to go.
    fn head_at(&self, top: usize) -> (u16, i64) {
        if self.pull == 0 {
            return self.head;
        }
        let top = i64::try_from(top).unwrap_or(i64::MAX);
        (self.head.0, top + i64::from(self.pull_at))
    }

    /// Its rows, top to bottom, each with its first and last column.
    fn rows(&self, top: usize) -> Vec<RowSpan> {
        let (left, right) = (self.area.x, self.area.x + self.area.width.saturating_sub(1));
        let head = self.head_at(top);
        let (a, b) = if (self.anchor.1, self.anchor.0) <= (head.1, head.0) {
            (self.anchor, head)
        } else {
            (head, self.anchor)
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

    /// Paint the highlight over a drawn frame. `top` is the document row at
    /// the top of the conversation's pane and `rows` the document's height;
    /// `covers` is what was drawn over the conversation this frame.
    pub fn paint(&self, buf: &mut Buffer, top: usize, rows: usize, covers: &[Rect], bg: Color) {
        if !self.moved {
            return;
        }
        let first = i64::try_from(top).unwrap_or(i64::MAX);
        let end = i64::try_from(rows).unwrap_or(i64::MAX);
        let covered = |x: u16, y: u16| {
            covers
                .iter()
                .any(|c| x >= c.x && x < c.x + c.width && y >= c.y && y < c.y + c.height)
        };
        for (row, x0, x1) in self.rows(top) {
            // A row of the document is on the screen where the pane shows
            // it; a row past the document's end is not text.
            let y = if self.chat.is_some() {
                if row >= end {
                    continue;
                }
                i64::from(self.area.y) + (row - first)
            } else {
                row
            };
            if y < i64::from(self.area.y) || y >= i64::from(self.area.y + self.area.height) {
                continue;
            }
            let y = u16::try_from(y).unwrap_or(0);
            for x in x0..=x1 {
                if self.chat.is_some() && covered(x, y) {
                    continue;
                }
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
        let rows = self.rows(view.scroll.effective.get());
        let (Some(first), Some(last)) = (rows.first(), rows.last()) else {
            return String::new();
        };
        let page = Page {
            left: self.area.x,
            right: self.area.x + self.area.width.saturating_sub(1),
            block: self.block,
            caret: theme.prompt,
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
                    let (line, frame) = without_frame(l);
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
/// The frame is what the renderer marked as one ([`markdown::is_frame`]),
/// and nothing else. Told by its color instead, a line of a diff that was
/// only a number, drawn dim like the frame, was blanked with it.
fn without_frame(line: Line<'static>) -> (Line<'static>, u16) {
    let width = |s: &Span<'_>| crate::chat::wrap::width(&s.content);
    let mut spans = line.spans;
    let Some(at) = spans.iter().position(markdown::is_frame) else {
        return (Line::from(spans), 0);
    };
    let rule = spans[at].content.starts_with(['╭', '╰']);
    let mut left = 0;
    let mut leading = true;
    for s in &mut spans[at..] {
        if markdown::is_frame(s) {
            if leading {
                left += width(s);
            }
            *s = Span::styled(" ".repeat(width(s)), s.style);
        } else {
            leading = false;
        }
    }
    let left = if rule {
        RULE
    } else {
        u16::try_from(left).unwrap_or(0)
    };
    (Line::from(spans), left)
}

/// In place of a frame's width: the row is the rule above or below a code
/// block, and is left out of what is copied.
const RULE: u16 = u16::MAX;

/// The pane a selection's text is read in: its first and last column,
/// whether the selection is a rectangle, and the color the composer's caret
/// is drawn in on an empty cell.
struct Page {
    left: u16,
    right: u16,
    block: bool,
    caret: Color,
}

impl Page {
    /// The text in `rows` of `buf`, a line each, without the spaces a row
    /// is padded with on its right. `top` and `from` are the row and the
    /// column the buffer starts at; `frames` is how many columns of a code
    /// block's frame each row had on its left.
    ///
    /// A row of code is copied from where its frame ends, with every space
    /// of its own: the body of a function keeps its indentation. Other
    /// rows lose the margin they share, so an answer set in under its
    /// speaker starts at its first letter. A rectangle is the cells it
    /// covers, less a margin all of it shares.
    fn text(&self, buf: &Buffer, rows: &[RowSpan], top: i64, from: u16, frames: &[u16]) -> String {
        // A row's cells from `x0` to `x1`: a wide character takes two
        // cells, and the second is not another character. The composer's
        // caret on an empty cell is not text.
        let cells = |y: u16, x0: u16, x1: u16| -> String {
            let mut line = String::new();
            let mut x = x0;
            while x <= x1 {
                let Some(cell) = buf.cell((x.saturating_sub(from), y)) else {
                    break;
                };
                // Known by its color, not its background: the frame read here
                // has the highlight painted over it.
                let caret = cell.symbol() == "█" && cell.fg == self.caret;
                line.push_str(if caret { " " } else { cell.symbol() });
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
        // The margin the rows that are not code share.
        let margin = if self.block {
            0
        } else {
            (0..rows.len())
                .filter(|i| frame(*i) == 0 && !rule(*i))
                .filter_map(|i| {
                    let whole = cells(at(i)?, self.left, self.right);
                    (!whole.trim().is_empty()).then(|| lead(&whole))
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
            let x0 = match (self.block, frame(i)) {
                (true, _) => x0,
                (false, 0) => x0.max(self.left + margin),
                (false, frame) => x0.max(self.left + frame),
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
