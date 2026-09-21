//! Multiline composer: buffer, cursor, paste, secret capture (`R-COMP-*`).

pub mod draw;

use ryter_core::Phase;
use unicode_segmentation::UnicodeSegmentation;

use crate::chat::wrap;

/// Pastes longer than this are summarized (`R-COMP-10`).
pub const PASTE_SUMMARY_AT: usize = 2000;
/// Character counter appears past this (`R-COMP-08`).
pub const COUNTER_AT: usize = 200;
/// Maximum visible rows before internal scrolling (`R-LAYOUT-03`).
pub const MAX_ROWS: usize = 8;

/// What the composer is capturing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// A prompt for the orchestrator.
    Normal,
    /// A pass note for a handoff.
    Handoff(Phase),
    /// An API key; text is masked and never rendered (`R-COMP-07`).
    Secret {
        /// Connection the key is for.
        connection: String,
    },
    /// A panel owns the composer as its input field (`R-COMP-17`).
    Field {
        /// Field name shown in the border title.
        label: String,
    },
}

/// Editing state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Composer {
    text: String,
    /// Byte offset of the cursor.
    cursor: usize,
    /// Current mode.
    pub mode: Mode,
    /// Secret buffer (never drawn).
    secret: String,
    /// Full text behind a `[pasted …]` placeholder.
    pub paste_buffer: Option<String>,
    /// Placeholder currently inserted for the paste.
    paste_marker: Option<String>,
    /// Last submit was rejected (border turns `error` until the next edit).
    pub rejected: bool,
    /// First visible wrapped row when content exceeds [`MAX_ROWS`].
    pub scroll_row: usize,
}

impl Default for Composer {
    fn default() -> Self {
        Self::new()
    }
}

impl Composer {
    /// Empty, normal mode.
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            mode: Mode::Normal,
            secret: String::new(),
            paste_buffer: None,
            paste_marker: None,
            rejected: false,
            scroll_row: 0,
        }
    }

    /// Visible text (never the secret).
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Cursor byte offset.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// True when nothing is typed (secret included).
    pub fn is_empty(&self) -> bool {
        if matches!(self.mode, Mode::Secret { .. }) {
            self.secret.is_empty()
        } else {
            self.text.is_empty()
        }
    }

    /// Grapheme count of the secret (for the `•` mask).
    pub fn secret_len(&self) -> usize {
        self.secret.graphemes(true).count()
    }

    /// Take the secret, leaving the composer in normal mode.
    pub fn take_secret(&mut self) -> Option<(String, String)> {
        let Mode::Secret { connection } = std::mem::replace(&mut self.mode, Mode::Normal) else {
            return None;
        };
        let key = std::mem::take(&mut self.secret);
        Some((connection, key))
    }

    /// Enter secret capture for `connection`.
    pub fn begin_secret(&mut self, connection: String) {
        self.secret.clear();
        self.text.clear();
        self.cursor = 0;
        self.mode = Mode::Secret { connection };
    }

    /// Enter panel-field mode with a label.
    pub fn begin_field(&mut self, label: impl Into<String>, initial: &str) {
        self.clear();
        self.text = initial.to_string();
        self.cursor = self.text.len();
        self.mode = Mode::Field {
            label: label.into(),
        };
    }

    /// Leave field / handoff / secret mode, clearing text.
    pub fn end_special(&mut self) {
        self.mode = Mode::Normal;
        self.secret.clear();
        self.clear();
    }

    /// Replace text wholesale (history recall, completion).
    pub fn set_text(&mut self, s: &str) {
        self.text = s.to_string();
        self.cursor = self.text.len();
        self.paste_buffer = None;
        self.paste_marker = None;
        self.rejected = false;
        self.scroll_row = 0;
    }

    /// Clear text and paste state (keeps mode).
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.paste_buffer = None;
        self.paste_marker = None;
        self.rejected = false;
        self.scroll_row = 0;
    }

    /// Take the submitted text, expanding a paste placeholder (`R-COMP-10`).
    pub fn take(&mut self) -> String {
        let mut out = std::mem::take(&mut self.text);
        if let (Some(marker), Some(full)) = (self.paste_marker.take(), self.paste_buffer.take()) {
            if let Some(at) = out.find(&marker) {
                out.replace_range(at..at + marker.len(), &full);
            }
        }
        self.cursor = 0;
        self.rejected = false;
        self.scroll_row = 0;
        out
    }

    /// Character count for the counter (visible text, or expanded paste length).
    pub fn char_count(&self) -> usize {
        let mut n = self.text.chars().count();
        if let (Some(m), Some(full)) = (&self.paste_marker, &self.paste_buffer) {
            n = n.saturating_sub(m.chars().count()) + full.chars().count();
        }
        n
    }

    // -- editing ----------------------------------------------------------

    /// Insert one character at the cursor.
    pub fn insert_char(&mut self, c: char) {
        self.rejected = false;
        if matches!(self.mode, Mode::Secret { .. }) {
            if !c.is_control() {
                self.secret.push(c);
            }
            return;
        }
        if c.is_control() && c != '\n' && c != '\t' {
            return;
        }
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Insert a string at the cursor (newlines literal, never submits).
    pub fn insert_str(&mut self, s: &str) {
        self.rejected = false;
        if matches!(self.mode, Mode::Secret { .. }) {
            self.secret.push_str(s.trim());
            return;
        }
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    /// Bracketed paste. Large pastes are summarized (`R-COMP-10`).
    pub fn paste(&mut self, s: &str) {
        let s = s.replace("\r\n", "\n").replace('\r', "\n");
        if matches!(self.mode, Mode::Secret { .. }) {
            self.secret.push_str(s.trim());
            return;
        }
        if s.chars().count() <= PASTE_SUMMARY_AT || self.paste_buffer.is_some() {
            self.insert_str(&s);
            return;
        }
        let chars = s.chars().count();
        let lines = s.lines().count();
        let marker = format!("[pasted {} chars, {} lines]", group_thousands(chars), lines);
        self.insert_str(&marker);
        self.paste_marker = Some(marker);
        self.paste_buffer = Some(s);
    }

    /// Insert a newline (`Shift+Enter`, `Alt+Enter`).
    pub fn newline(&mut self) {
        self.insert_char('\n');
    }

    /// Delete the grapheme before the cursor.
    pub fn backspace(&mut self) {
        self.rejected = false;
        if matches!(self.mode, Mode::Secret { .. }) {
            if let Some((i, _)) = self.secret.grapheme_indices(true).next_back() {
                self.secret.truncate(i);
            }
            return;
        }
        if self.cursor == 0 {
            return;
        }
        let start = prev_boundary(&self.text, self.cursor);
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    /// Delete the grapheme under the cursor.
    pub fn delete(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        let end = next_boundary(&self.text, self.cursor);
        self.text.replace_range(self.cursor..end, "");
    }

    /// `Ctrl+W`.
    pub fn kill_word_back(&mut self) {
        let start = self.word_left_pos();
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    /// `Ctrl+U`.
    pub fn kill_to_line_start(&mut self) {
        let start = self.line_start();
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    /// `Ctrl+K`.
    pub fn kill_to_line_end(&mut self) {
        let end = self.line_end();
        self.text.replace_range(self.cursor..end, "");
    }

    // -- movement ---------------------------------------------------------

    /// One grapheme left.
    pub fn left(&mut self) {
        self.cursor = prev_boundary(&self.text, self.cursor);
    }

    /// One grapheme right.
    pub fn right(&mut self) {
        self.cursor = next_boundary(&self.text, self.cursor);
    }

    /// Start of the logical line.
    pub fn home(&mut self) {
        self.cursor = self.line_start();
    }

    /// End of the logical line.
    pub fn end(&mut self) {
        self.cursor = self.line_end();
    }

    /// `Ctrl+←`.
    pub fn word_left(&mut self) {
        self.cursor = self.word_left_pos();
    }

    /// `Ctrl+→`.
    pub fn word_right(&mut self) {
        let bytes = self.text.as_bytes();
        let mut i = self.cursor;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        self.cursor = i;
    }

    /// True when the text spans more than one logical line.
    pub fn is_multiline(&self) -> bool {
        self.text.contains('\n')
    }

    /// Move to the previous logical line; false when already on the first.
    pub fn up(&mut self) -> bool {
        let start = self.line_start();
        if start == 0 {
            return false;
        }
        let col = wrap::width(&self.text[start..self.cursor]);
        let prev_start = line_start_at(&self.text, start - 1);
        self.cursor = col_to_offset(&self.text, prev_start, start - 1, col);
        true
    }

    /// Move to the next logical line; false when already on the last.
    pub fn down(&mut self) -> bool {
        let end = self.line_end();
        if end >= self.text.len() {
            return false;
        }
        let start = self.line_start();
        let col = wrap::width(&self.text[start..self.cursor]);
        let next_start = end + 1;
        let next_end = line_end_at(&self.text, next_start);
        self.cursor = col_to_offset(&self.text, next_start, next_end, col);
        true
    }

    fn line_start(&self) -> usize {
        line_start_at(&self.text, self.cursor)
    }

    fn line_end(&self) -> usize {
        line_end_at(&self.text, self.cursor)
    }

    fn word_left_pos(&self) -> usize {
        let bytes = self.text.as_bytes();
        let mut i = self.cursor;
        while i > 0 && bytes[i - 1].is_ascii_whitespace() {
            i -= 1;
        }
        while i > 0 && !bytes[i - 1].is_ascii_whitespace() {
            i -= 1;
        }
        i
    }

    // -- layout -----------------------------------------------------------

    /// Wrapped rows as byte ranges over `text`, plus the cursor's (row, col).
    pub fn layout(&self, width: usize) -> (Vec<(usize, usize)>, (usize, usize)) {
        let width = width.max(1);
        let mut rows: Vec<(usize, usize)> = Vec::new();
        let mut cursor_rc = (0usize, 0usize);
        let mut base = 0usize;
        let mut found = false;
        for line in self.text.split('\n') {
            let ranges = wrap::wrap_ranges(line, width);
            let n = ranges.len();
            for (i, (a, b)) in ranges.into_iter().enumerate() {
                let (ga, gb) = (base + a, base + b);
                let last_row_of_line = i + 1 == n;
                if !found {
                    let inside = self.cursor >= ga
                        && (self.cursor < gb
                            || (last_row_of_line && self.cursor <= base + line.len()));
                    if inside {
                        let col = wrap::width(&self.text[ga..self.cursor.min(gb).max(ga)]);
                        cursor_rc = (rows.len(), col.min(width));
                        found = true;
                    }
                }
                rows.push((ga, gb));
            }
            base += line.len() + 1;
        }
        if rows.is_empty() {
            rows.push((0, 0));
        }
        if !found {
            cursor_rc = (rows.len() - 1, 0);
        }
        (rows, cursor_rc)
    }

    /// Wrapped row count at `width`.
    pub fn rows(&self, width: usize) -> usize {
        if matches!(self.mode, Mode::Secret { .. }) {
            return 1;
        }
        self.layout(width).0.len()
    }
}

fn prev_boundary(s: &str, at: usize) -> usize {
    s[..at]
        .grapheme_indices(true)
        .next_back()
        .map(|(i, _)| i)
        .unwrap_or(0)
}

fn next_boundary(s: &str, at: usize) -> usize {
    s[at..]
        .graphemes(true)
        .next()
        .map(|g| at + g.len())
        .unwrap_or(s.len())
}

fn line_start_at(s: &str, at: usize) -> usize {
    s[..at].rfind('\n').map(|i| i + 1).unwrap_or(0)
}

fn line_end_at(s: &str, at: usize) -> usize {
    s[at..].find('\n').map(|i| at + i).unwrap_or(s.len())
}

fn col_to_offset(s: &str, start: usize, end: usize, col: usize) -> usize {
    let mut w = 0usize;
    for (i, g) in s[start..end].grapheme_indices(true) {
        if w >= col {
            return start + i;
        }
        w += wrap::width(g);
    }
    end
}

/// `4210` → `4,210`.
pub fn group_thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_move_delete_with_wide_chars() {
        let mut c = Composer::new();
        for ch in "héllo 漢字".chars() {
            c.insert_char(ch);
        }
        assert_eq!(c.text(), "héllo 漢字");
        c.left();
        c.left();
        assert_eq!(&c.text()[c.cursor()..], "漢字");
        c.backspace();
        assert_eq!(c.text(), "héllo漢字");
        c.home();
        assert_eq!(c.cursor(), 0);
        c.end();
        assert_eq!(c.cursor(), c.text().len());
        c.kill_word_back();
        assert_eq!(c.text(), "");
    }

    #[test]
    fn multiline_up_down_and_kills() {
        let mut c = Composer::new();
        c.set_text("first line\nsecond");
        assert!(c.is_multiline());
        assert!(!c.down());
        assert!(c.up());
        assert_eq!(&c.text()[..c.cursor()], "first ");
        c.kill_to_line_end();
        assert_eq!(c.text(), "first \nsecond");
        c.kill_to_line_start();
        assert_eq!(c.text(), "\nsecond");
        c.down();
        c.end();
        c.word_left();
        assert_eq!(&c.text()[c.cursor()..], "second");
    }

    #[test]
    fn layout_maps_cursor_after_wrap() {
        let mut c = Composer::new();
        c.set_text("the quick brown fox");
        let (rows, (r, col)) = c.layout(9);
        assert_eq!(rows.len(), 2);
        assert_eq!((r, col), (1, 9));
        c.home();
        let (_, (r, col)) = c.layout(9);
        assert_eq!((r, col), (0, 0));
        c.set_text("a\n\nb");
        assert_eq!(c.rows(20), 3);
    }

    #[test]
    fn large_paste_is_summarized_and_expanded_on_take() {
        let mut c = Composer::new();
        let big: String = (0..3000)
            .map(|i| if i % 50 == 49 { '\n' } else { 'x' })
            .collect();
        c.paste(&big);
        assert!(
            c.text().starts_with("[pasted 3,000 chars, 60 lines]"),
            "{}",
            c.text()
        );
        assert_eq!(c.char_count(), 3000);
        let out = c.take();
        assert_eq!(out, big);
        assert!(c.text().is_empty());
        // Small pastes with newlines insert literally.
        c.paste("a\r\nb");
        assert_eq!(c.text(), "a\nb");
    }

    #[test]
    fn secret_mode_hides_text() {
        let mut c = Composer::new();
        c.begin_secret("openrouter".into());
        for ch in "sk-abc".chars() {
            c.insert_char(ch);
        }
        assert_eq!(c.text(), "");
        assert_eq!(c.secret_len(), 6);
        c.backspace();
        assert_eq!(c.secret_len(), 5);
        let (conn, key) = c.take_secret().unwrap();
        assert_eq!(conn, "openrouter");
        assert_eq!(key, "sk-ab");
        assert_eq!(c.mode, Mode::Normal);
    }

    #[test]
    fn thousands() {
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(4210), "4,210");
        assert_eq!(group_thousands(1_234_567), "1,234,567");
    }
}
