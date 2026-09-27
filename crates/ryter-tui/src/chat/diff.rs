//! An edit in the chat: its hunks as numbered, full-width tinted rows, green
//! for what came and red for what went, highlighted like the file.
//!
//! The tool measured the diff against the file on disk, so the numbers are
//! the file's own. A long diff shows its first rows and says how many more
//! there are; `Ctrl+O` shows every edit (and audit) whole.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ryter_core::diff::{DiffLine, FileDiff, Hunk, LineKind};

use super::highlight;
use super::markdown::slice_chunks;
use super::wrap::{self, Chunk};
use crate::theme::{ColorMode, Theme};

/// Diff rows an edit shows before folding, unless edits are expanded.
pub const DEFAULT_ROWS: usize = 12;

/// Render `diff` in `width` columns, at most `max_rows` rows of it.
pub fn render(diff: &FileDiff, max_rows: usize, width: usize, theme: Theme) -> Vec<Line<'static>> {
    let hint = if max_rows == usize::MAX {
        "/changes has the whole diff"
    } else {
        "^O shows every edit whole · /changes"
    };
    render_folded(diff, max_rows, width, theme, hint)
}

/// [`render`], with `hint` saying where the rest can be seen.
pub fn render_folded(
    diff: &FileDiff,
    max_rows: usize,
    width: usize,
    theme: Theme,
    hint: &str,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    if diff.hunks.is_empty() {
        if !diff.is_empty() {
            out.push(note(
                "too large to show here · /changes has the diff",
                width,
                theme,
            ));
        }
        return out;
    }
    let num_w = diff
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .filter_map(|l| l.new.or(l.old))
        .max()
        .unwrap_or(1)
        .to_string()
        .len();
    let text_w = width.saturating_sub(num_w + 3).max(4);
    let ext = std::path::Path::new(&diff.path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_string);
    let tinted =
        theme.diff_add_bg != theme.bg && !matches!(theme.mode, ColorMode::Ansi16 | ColorMode::Mono);
    let total = diff.len();
    let mut shown = 0usize;
    'hunks: for (hi, hunk) in diff.hunks.iter().enumerate() {
        if hi > 0 {
            if out.len() >= max_rows {
                break;
            }
            out.push(Line::from(Span::styled(
                wrap::pad_right(&format!("{:>num_w$}", "⋯"), width),
                theme.muted(),
            )));
        }
        let rows = highlighted(hunk, ext.as_deref(), theme);
        for (line, chunks) in hunk.lines.iter().zip(rows) {
            let pieces = row(line, chunks, num_w, text_w, width, tinted, theme);
            if out.len() + pieces.len() > max_rows && !out.is_empty() {
                break 'hunks;
            }
            out.extend(pieces);
            shown += 1;
        }
    }
    let rest = total.saturating_sub(shown);
    if rest > 0 {
        let more = format!(
            "… {rest} more line{} · {hint}",
            if rest == 1 { "" } else { "s" }
        );
        out.push(note(&more, width, theme));
    }
    out
}

fn note(text: &str, width: usize, theme: Theme) -> Line<'static> {
    Line::from(Span::styled(
        wrap::truncate(text, width),
        theme.muted().add_modifier(Modifier::ITALIC),
    ))
}

/// Highlight a hunk the way each side of it reads: context and added lines
/// as the new file, context and removed lines as the old one. Highlighting
/// the rows in display order would parse a removed line and its
/// replacement as one program.
fn highlighted(hunk: &Hunk, ext: Option<&str>, theme: Theme) -> Vec<Vec<Chunk>> {
    let side = |keep: LineKind| -> (Vec<usize>, Vec<String>) {
        hunk.lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.kind == LineKind::Context || l.kind == keep)
            .map(|(i, l)| (i, wrap::expand_tabs(&l.text)))
            .unzip()
    };
    let mut rows: Vec<Vec<Chunk>> = vec![Vec::new(); hunk.lines.len()];
    for keep in [LineKind::Removed, LineKind::Added] {
        let (at, text) = side(keep);
        let hl = highlight::highlight(None, ext, &text, theme);
        for (i, r) in at.into_iter().zip(hl.rows) {
            // Context lines appear on both sides; the new side wins.
            rows[i] = r;
        }
    }
    rows
}

/// One diff line as screen rows: the number, the sign, the text, and the
/// row's tint out to the edge. A long line wraps under its own text.
fn row(
    line: &DiffLine,
    chunks: Vec<Chunk>,
    num_w: usize,
    text_w: usize,
    width: usize,
    tinted: bool,
    theme: Theme,
) -> Vec<Line<'static>> {
    let (bg, sign, sign_fg) = match line.kind {
        LineKind::Added => (theme.diff_add_bg, "+", theme.success),
        LineKind::Removed => (theme.diff_del_bg, "-", theme.error),
        LineKind::Context => (theme.bg, " ", theme.dim),
    };
    let bg = if tinted { bg } else { theme.bg };
    let base = Style::default().bg(bg);
    // Without tints the text itself carries the change.
    let plain = match line.kind {
        LineKind::Added => base.fg(theme.success),
        LineKind::Removed => base.fg(theme.error),
        LineKind::Context => base.fg(theme.dim),
    };
    let num = match line.kind {
        LineKind::Removed => line.old,
        _ => line.new,
    }
    .map(|n| n.to_string())
    .unwrap_or_default();
    let text: String = chunks.iter().map(|c| c.text.as_str()).collect();
    let pieces = wrap::hard_wrap(&text, text_w, 0);
    let pieces = if pieces.is_empty() {
        vec![String::new()]
    } else {
        pieces
    };
    let mut out = Vec::with_capacity(pieces.len());
    let mut consumed = 0usize;
    for (pi, piece) in pieces.iter().enumerate() {
        let mut spans = Vec::new();
        let (n, s) = if pi == 0 {
            (num.as_str(), sign)
        } else {
            ("", " ")
        };
        spans.push(Span::styled(format!("{n:>num_w$} "), base.fg(theme.dim)));
        spans.push(Span::styled(
            format!("{s} "),
            base.fg(sign_fg).add_modifier(Modifier::BOLD),
        ));
        let mut used = num_w + 3;
        let start = consumed;
        consumed += piece.len();
        for c in slice_chunks(&chunks, start, consumed) {
            used += wrap::width(&c.text);
            let style = if tinted { c.style.bg(bg) } else { plain };
            spans.push(Span::styled(c.text, style));
        }
        spans.push(Span::styled(" ".repeat(width.saturating_sub(used)), base));
        out.push(Line::from(spans));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn an_edit_reads_as_numbered_rows_with_its_context() {
        let d = FileDiff::new(
            "a.rs",
            Some("fn a() {}\nlet x = 1;\nfn b() {}\n"),
            "fn a() {}\nlet x = 2;\nfn b() {}\n",
        );
        let rows = render(&d, 50, 40, Theme::truecolor_dark());
        assert_eq!(
            text(&rows),
            vec![
                "1   fn a() {}",
                "2 - let x = 1;",
                "2 + let x = 2;",
                "3   fn b() {}"
            ]
        );
    }

    /// The whole row is tinted, out to the pane's edge, on added and
    /// removed lines, and not on context.
    #[test]
    fn changed_rows_are_tinted_edge_to_edge() {
        let t = Theme::truecolor_dark();
        let d = FileDiff::new("a.txt", Some("same\nold\n"), "same\nnew\n");
        let rows = render(&d, 50, 30, t);
        let width = |l: &Line| {
            l.spans
                .iter()
                .map(|s| wrap::width(&s.content))
                .sum::<usize>()
        };
        for (row, bg) in rows.iter().zip([t.bg, t.diff_del_bg, t.diff_add_bg]) {
            assert_eq!(width(row), 30);
            assert!(row.spans.iter().all(|s| s.style.bg == Some(bg)), "{row:?}");
        }
        assert_ne!(t.diff_add_bg, t.diff_del_bg);
    }

    #[test]
    fn a_long_diff_folds_and_says_how_much_is_left() {
        let new: String = (1..=40).map(|i| format!("line {i}\n")).collect();
        let d = FileDiff::new("n.txt", None, &new);
        let rows = text(&render(&d, 12, 60, Theme::truecolor_dark()));
        assert_eq!(rows.len(), 13);
        assert_eq!(rows[11], "12 + line 12");
        assert!(rows[12].starts_with("… 28 more lines"), "{}", rows[12]);
        let all = render(&d, usize::MAX, 60, Theme::truecolor_dark());
        assert_eq!(all.len(), 40);
    }

    #[test]
    fn without_tints_the_text_carries_the_color() {
        let t = Theme::default_16();
        let d = FileDiff::new("a.txt", Some("old\n"), "new\n");
        let rows = render(&d, 50, 30, t);
        let fg = |l: &Line| {
            l.spans
                .iter()
                .rev()
                .find(|s| !s.content.trim().is_empty())
                .and_then(|s| s.style.fg)
        };
        assert_eq!(fg(&rows[0]), Some(t.error));
        assert_eq!(fg(&rows[1]), Some(t.success));
    }
}
