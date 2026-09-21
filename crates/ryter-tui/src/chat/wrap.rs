//! Unicode-aware width math and wrapping (`R-WRAP-*`).
//!
//! Every width here is a display column count from `unicode-width`; every
//! break lands on a grapheme-cluster boundary from `unicode-segmentation`.

use ratatui::style::Style;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Columns to expand a tab to in code (`R-WRAP-05`).
pub const TAB: usize = 4;

/// Display width of a string.
pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Expand tabs to the next multiple of [`TAB`] columns.
pub fn expand_tabs(s: &str) -> String {
    if !s.contains('\t') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 8);
    let mut col = 0usize;
    for g in s.graphemes(true) {
        if g == "\t" {
            let n = TAB - (col % TAB);
            out.extend(std::iter::repeat_n(' ', n));
            col += n;
        } else {
            out.push_str(g);
            col += width(g);
        }
    }
    out
}

/// Longest prefix of `s` whose width is at most `max`, plus that width.
pub fn take_width(s: &str, max: usize) -> (&str, usize) {
    let mut w = 0usize;
    let mut end = 0usize;
    for (i, g) in s.grapheme_indices(true) {
        let gw = width(g);
        if w + gw > max {
            break;
        }
        w += gw;
        end = i + g.len();
    }
    (&s[..end], w)
}

/// Truncate to `max` columns, ending with `…` when anything was cut.
pub fn truncate(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let (head, _) = take_width(s, max.saturating_sub(1));
    format!("{head}…")
}

/// Pad with spaces on the right to exactly `w` columns (truncating if longer).
pub fn pad_right(s: &str, w: usize) -> String {
    let cur = width(s);
    if cur > w {
        return take_width(s, w).0.to_string();
    }
    let mut out = String::with_capacity(s.len() + (w - cur));
    out.push_str(s);
    out.extend(std::iter::repeat_n(' ', w - cur));
    out
}

/// Pad with spaces on the left to exactly `w` columns.
pub fn pad_left(s: &str, w: usize) -> String {
    let cur = width(s);
    if cur >= w {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + (w - cur));
    out.extend(std::iter::repeat_n(' ', w - cur));
    out.push_str(s);
    out
}

/// One grapheme with its style, the unit of wrapping.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    /// The grapheme cluster.
    pub text: String,
    /// Display width.
    pub w: usize,
    /// Style it was authored with.
    pub style: Style,
    /// Byte offset in the source text (composer cursor math).
    pub at: usize,
}

impl Piece {
    fn is_space(&self) -> bool {
        self.text == " " || self.text == "\u{3000}"
    }
}

/// A styled run: adjacent pieces with the same style, joined.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    /// Text.
    pub text: String,
    /// Style.
    pub style: Style,
}

impl Chunk {
    /// Convenience constructor.
    pub fn new(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

/// Split styled runs into graphemes.
pub fn pieces(chunks: &[Chunk]) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut at = 0usize;
    for c in chunks {
        for g in c.text.graphemes(true) {
            out.push(Piece {
                text: g.to_string(),
                w: width(g),
                style: c.style,
                at,
            });
            at += g.len();
        }
    }
    out
}

/// Merge consecutive same-style pieces back into chunks.
pub fn join(pieces: &[Piece]) -> Vec<Chunk> {
    let mut out: Vec<Chunk> = Vec::new();
    for p in pieces {
        match out.last_mut() {
            Some(c) if c.style == p.style => c.text.push_str(&p.text),
            _ => out.push(Chunk::new(p.text.clone(), p.style)),
        }
    }
    out
}

/// Word-preference wrap over graphemes. A word only breaks internally when it
/// alone exceeds `width` (`R-WRAP-02`). Returns rows of pieces; leading
/// whitespace at a break is dropped.
pub fn wrap_pieces(pieces: &[Piece], width: usize) -> Vec<Vec<Piece>> {
    let width = width.max(1);
    let mut rows: Vec<Vec<Piece>> = Vec::new();
    let mut cur: Vec<Piece> = Vec::new();
    let mut cur_w = 0usize;
    let mut i = 0usize;
    while i < pieces.len() {
        // Gather the next token: a run of spaces or a run of non-spaces.
        let start = i;
        let space = pieces[i].is_space();
        while i < pieces.len() && pieces[i].is_space() == space {
            i += 1;
        }
        let tok = &pieces[start..i];
        let tok_w: usize = tok.iter().map(|p| p.w).sum();
        if space {
            if cur_w == 0 && !rows.is_empty() {
                // Leading whitespace after a break is dropped.
                continue;
            }
            if cur_w + tok_w <= width {
                cur.extend_from_slice(tok);
                cur_w += tok_w;
            } else {
                // Spaces that would overflow end the row.
                rows.push(std::mem::take(&mut cur));
                cur_w = 0;
            }
            continue;
        }
        if cur_w + tok_w <= width {
            cur.extend_from_slice(tok);
            cur_w += tok_w;
            continue;
        }
        if cur_w > 0 {
            trim_trailing(&mut cur);
            rows.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        if tok_w <= width {
            cur.extend_from_slice(tok);
            cur_w = tok_w;
            continue;
        }
        // Unbreakable token wider than the row: fill rows grapheme by grapheme.
        for p in tok {
            if cur_w + p.w > width && cur_w > 0 {
                rows.push(std::mem::take(&mut cur));
                cur_w = 0;
            }
            cur_w += p.w;
            cur.push(p.clone());
        }
    }
    trim_trailing(&mut cur);
    if !cur.is_empty() || rows.is_empty() {
        rows.push(cur);
    }
    rows
}

fn trim_trailing(row: &mut Vec<Piece>) {
    while row.last().is_some_and(Piece::is_space) {
        row.pop();
    }
}

/// Wrap styled runs into rows of styled runs.
pub fn wrap_styled(chunks: &[Chunk], width: usize) -> Vec<Vec<Chunk>> {
    wrap_pieces(&pieces(chunks), width)
        .iter()
        .map(|row| join(row))
        .collect()
}

/// Byte ranges of each wrapped row of a single logical line (composer cursor math).
/// Ranges are `[start, end)` into `text`; dropped break whitespace is not
/// covered by any range.
pub fn wrap_ranges(text: &str, width: usize) -> Vec<(usize, usize)> {
    let ps = pieces(&[Chunk::new(text, Style::default())]);
    wrap_pieces(&ps, width)
        .iter()
        .map(|row| match (row.first(), row.last()) {
            (Some(a), Some(b)) => (a.at, b.at + b.text.len()),
            _ => (text.len(), text.len()),
        })
        .collect()
}

/// Word-wrap plain text; `\n` always breaks.
pub fn wrap_plain(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.split('\n') {
        let line = line.trim_end_matches('\r');
        for (a, b) in wrap_ranges(line, width) {
            out.push(line[a..b].to_string());
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Hard wrap at grapheme boundaries with no word preference (code, `R-WRAP-04`).
/// The first row is `width` wide; continuation rows are `width - cont` wide so
/// the caller can prefix a marker.
pub fn hard_wrap(text: &str, width: usize, cont: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let mut rest = text;
    let mut first = true;
    loop {
        let max = if first {
            width
        } else {
            width.saturating_sub(cont).max(1)
        };
        let (head, _) = take_width(rest, max);
        if head.is_empty() {
            if rest.is_empty() {
                if first {
                    rows.push(String::new());
                }
                break;
            }
            // A single grapheme wider than the row: emit it anyway.
            let g = rest.graphemes(true).next().unwrap_or(rest);
            rows.push(g.to_string());
            rest = &rest[g.len()..];
        } else {
            rows.push(head.to_string());
            rest = &rest[head.len()..];
        }
        first = false;
        if rest.is_empty() {
            break;
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_text(row: &[Piece]) -> String {
        row.iter().map(|p| p.text.as_str()).collect()
    }

    #[test]
    fn cjk_counts_two_columns() {
        assert_eq!(width("漢字"), 4);
        let rows = wrap_plain("漢字漢字漢字", 5);
        assert_eq!(rows, vec!["漢字", "漢字", "漢字"]);
        for r in rows {
            assert!(width(&r) <= 5);
        }
    }

    #[test]
    fn emoji_zwj_sequence_is_not_split() {
        let family = "👨\u{200d}👩\u{200d}👧";
        let text = format!("a {family} b");
        let rows = wrap_plain(&text, 3);
        assert!(rows.iter().any(|r| r.contains(family)), "{rows:?}");
        for r in &rows {
            assert!(width(r) <= 3, "{r:?}");
        }
    }

    #[test]
    fn combining_marks_stay_attached() {
        let s = "e\u{301}e\u{301}e\u{301}"; // é é é as base + combining
        let rows = wrap_plain(s, 2);
        assert_eq!(rows.len(), 2);
        for r in rows {
            assert!(r.chars().count() % 2 == 0, "{r:?}");
        }
    }

    #[test]
    fn unbreakable_token_fills_rows_exactly() {
        let long = "x".repeat(300);
        let rows = wrap_plain(&long, 40);
        assert_eq!(rows.len(), 8);
        assert!(rows.iter().all(|r| width(r) <= 40));
        assert_eq!(rows[7].len(), 20);
    }

    #[test]
    fn word_preference_and_dropped_break_space() {
        let rows = wrap_plain("the quick brown fox", 9);
        assert_eq!(rows, vec!["the quick", "brown fox"]);
        let rows = wrap_plain("a  b", 10);
        assert_eq!(rows, vec!["a  b"]);
    }

    #[test]
    fn tabs_expand_to_four() {
        assert_eq!(expand_tabs("\tx"), "    x");
        assert_eq!(expand_tabs("ab\tx"), "ab  x");
        assert_eq!(width(&expand_tabs("\t\t")), 8);
    }

    #[test]
    fn hard_wrap_leaves_room_for_marker() {
        let rows = hard_wrap("0123456789", 4, 2);
        assert_eq!(rows, vec!["0123", "45", "67", "89"]);
        assert_eq!(hard_wrap("", 4, 2), vec![""]);
    }

    #[test]
    fn truncate_and_pad_are_width_based() {
        assert_eq!(truncate("hello", 5), "hello");
        assert_eq!(truncate("hello world", 5), "hell…");
        assert_eq!(truncate("漢字漢字", 5), "漢字…");
        assert_eq!(pad_right("漢", 4), "漢  ");
        assert_eq!(pad_left("1", 3), "  1");
    }

    #[test]
    fn ranges_map_back_to_bytes() {
        let t = "héllo wörld";
        let r = wrap_ranges(t, 6);
        assert_eq!(r.len(), 2);
        assert_eq!(&t[r[0].0..r[0].1], "héllo");
        assert_eq!(&t[r[1].0..r[1].1], "wörld");
    }

    #[test]
    fn styled_wrap_preserves_styles_across_break() {
        let bold = Style::default().add_modifier(ratatui::style::Modifier::BOLD);
        let rows = wrap_styled(
            &[
                Chunk::new("plain ", Style::default()),
                Chunk::new("bold text", bold),
            ],
            11,
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(row_text(&pieces(&rows[0])), "plain bold");
        assert_eq!(rows[0][1].style, bold);
        assert_eq!(rows[1][0].style, bold);
        assert_eq!(row_text(&pieces(&rows[1])), "text");
    }
}
