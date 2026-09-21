//! In-tree markdown: block + inline parser → styled rows (`R-MD-*`).
//!
//! Coverage is a contract, not a suggestion: headings, paragraphs, fenced and
//! indented code, nested lists (4 levels), block quotes, thematic breaks, GFM
//! tables; bold, italic, code spans, strikethrough, links, autolinks, escapes.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::highlight;
use super::wrap::{self, Chunk};
use crate::theme::Theme;

/// Rendering knobs for one body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdOptions {
    /// Columns available.
    pub width: usize,
    /// Line-number gutter in code blocks (`R-MD-10`, `[ui] line_numbers`).
    pub line_numbers: bool,
    /// File extension guessed from a preceding tool row (`R-SYN-04`).
    pub lang_hint: Option<String>,
}

/// Column alignment from a GFM delimiter row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// `---` or `:---`.
    Left,
    /// `:---:`.
    Center,
    /// `---:`.
    Right,
}

/// One list item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListItem {
    /// Ordered-list number, if any.
    pub number: Option<u32>,
    /// Item content.
    pub blocks: Vec<Block>,
}

/// Block-level element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// ATX heading.
    Heading {
        /// 1–6.
        level: u8,
        /// Content.
        inline: Vec<Run>,
    },
    /// Paragraph.
    Paragraph(Vec<Run>),
    /// Fenced or indented code.
    Code {
        /// Info string token.
        lang: Option<String>,
        /// Raw lines.
        lines: Vec<String>,
        /// False for an unterminated fence mid-stream (`R-MD-11`).
        closed: bool,
    },
    /// List.
    List {
        /// Ordered.
        ordered: bool,
        /// Items.
        items: Vec<ListItem>,
    },
    /// Block quote.
    Quote(Vec<Block>),
    /// Thematic break.
    Rule,
    /// GFM table.
    Table {
        /// Per-column alignment.
        align: Vec<Align>,
        /// Header cells.
        header: Vec<Vec<Run>>,
        /// Body rows.
        rows: Vec<Vec<Vec<Run>>>,
    },
}

/// Inline run with style flags (flat, not a tree).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Run {
    /// Text.
    pub text: String,
    /// `**bold**`.
    pub bold: bool,
    /// `*italic*`.
    pub italic: bool,
    /// `` `code` ``.
    pub code: bool,
    /// `~~strike~~`.
    pub strike: bool,
    /// Link target when this run is link text.
    pub url: Option<String>,
}

// ---------------------------------------------------------------------------
// Block parsing
// ---------------------------------------------------------------------------

/// Parse markdown into blocks.
pub fn parse_blocks(text: &str) -> Vec<Block> {
    let lines: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    parse_lines(&lines, 0)
}

fn indent_of(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ').count()
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

fn fence_open(line: &str) -> Option<(char, usize, Option<String>)> {
    let t = line.trim_start();
    if indent_of(line) > 3 {
        return None;
    }
    let ch = t.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let n = t.chars().take_while(|c| *c == ch).count();
    if n < 3 {
        return None;
    }
    let info = t[n..].trim();
    if ch == '`' && info.contains('`') {
        return None;
    }
    let lang = info
        .split_whitespace()
        .next()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    Some((ch, n, lang))
}

fn fence_close(line: &str, ch: char, n: usize) -> bool {
    let t = line.trim();
    if indent_of(line) > 3 {
        return false;
    }
    !t.is_empty() && t.chars().all(|c| c == ch) && t.chars().count() >= n
}

fn heading(line: &str) -> Option<(u8, &str)> {
    if indent_of(line) > 3 {
        return None;
    }
    let t = line.trim_start();
    let n = t.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&n) {
        return None;
    }
    let rest = &t[n..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    let body = rest.trim().trim_end_matches('#').trim();
    Some((n as u8, body))
}

fn is_rule(line: &str) -> bool {
    if indent_of(line) > 3 {
        return false;
    }
    let t: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    t.chars().count() >= 3
        && (t.chars().all(|c| c == '-')
            || t.chars().all(|c| c == '*')
            || t.chars().all(|c| c == '_'))
}

/// `(marker_indent, content_indent, ordered, number)` for a list-item line.
fn list_marker(line: &str) -> Option<(usize, usize, bool, Option<u32>)> {
    let ind = indent_of(line);
    let t = &line[ind..];
    for m in ["- ", "* ", "+ ", "• "] {
        if let Some(rest) = t.strip_prefix(m) {
            let extra = rest.chars().take_while(|c| *c == ' ').count().min(3);
            return Some((ind, ind + m.chars().count() + extra, false, None));
        }
    }
    if t == "-" || t == "*" || t == "+" {
        return Some((ind, ind + 2, false, None));
    }
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if (1..=9).contains(&digits) {
        let after = &t[digits..];
        if let Some(rest) = after
            .strip_prefix(". ")
            .or_else(|| after.strip_prefix(") "))
        {
            let extra = rest.chars().take_while(|c| *c == ' ').count().min(3);
            let n = t[..digits].parse().ok();
            return Some((ind, ind + digits + 2 + extra, true, n));
        }
    }
    None
}

fn is_quote(line: &str) -> bool {
    indent_of(line) <= 3 && line.trim_start().starts_with('>')
}

fn strip_quote(line: &str) -> &str {
    let t = line.trim_start();
    let t = t.strip_prefix('>').unwrap_or(t);
    t.strip_prefix(' ').unwrap_or(t)
}

fn is_table_sep(line: &str) -> bool {
    let t = line.trim();
    if !t.contains('-') || !t.contains('|') && !t.contains(':') {
        return false;
    }
    let body = t.trim_matches('|');
    !body.is_empty()
        && body.split('|').all(|c| {
            let c = c.trim();
            !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':') && c.contains('-')
        })
}

fn split_cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'|') {
            cur.push('|');
            chars.next();
        } else if c == '|' {
            cells.push(cur.trim().to_string());
            cur.clear();
        } else {
            cur.push(c);
        }
    }
    cells.push(cur.trim().to_string());
    cells
}

fn starts_block(line: &str) -> bool {
    fence_open(line).is_some()
        || heading(line).is_some()
        || is_rule(line)
        || is_quote(line)
        || list_marker(line).is_some()
}

fn parse_lines(lines: &[&str], depth: usize) -> Vec<Block> {
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut after_blank = true;
    while i < lines.len() {
        let line = lines[i];
        if is_blank(line) {
            i += 1;
            after_blank = true;
            continue;
        }
        if let Some((ch, n, lang)) = fence_open(line) {
            let mut body = Vec::new();
            let mut j = i + 1;
            let mut closed = false;
            while j < lines.len() {
                if fence_close(lines[j], ch, n) {
                    closed = true;
                    j += 1;
                    break;
                }
                body.push(lines[j].to_string());
                j += 1;
            }
            out.push(Block::Code {
                lang,
                lines: body,
                closed,
            });
            i = j;
            after_blank = false;
            continue;
        }
        if let Some((level, body)) = heading(line) {
            out.push(Block::Heading {
                level,
                inline: parse_inline(body),
            });
            i += 1;
            after_blank = false;
            continue;
        }
        if is_rule(line) && list_marker(line).is_none() {
            out.push(Block::Rule);
            i += 1;
            after_blank = false;
            continue;
        }
        if is_quote(line) {
            let mut inner = Vec::new();
            let mut j = i;
            while j < lines.len() && is_quote(lines[j]) {
                inner.push(strip_quote(lines[j]));
                j += 1;
            }
            out.push(Block::Quote(parse_lines(&inner, depth)));
            i = j;
            after_blank = false;
            continue;
        }
        if line.contains('|') && i + 1 < lines.len() && is_table_sep(lines[i + 1]) {
            let header: Vec<Vec<Run>> = split_cells(line).iter().map(|c| parse_inline(c)).collect();
            let align: Vec<Align> = split_cells(lines[i + 1])
                .iter()
                .map(|c| {
                    let l = c.starts_with(':');
                    let r = c.ends_with(':');
                    match (l, r) {
                        (true, true) => Align::Center,
                        (false, true) => Align::Right,
                        _ => Align::Left,
                    }
                })
                .collect();
            let mut rows = Vec::new();
            let mut j = i + 2;
            while j < lines.len() && !is_blank(lines[j]) && lines[j].contains('|') {
                rows.push(
                    split_cells(lines[j])
                        .iter()
                        .map(|c| parse_inline(c))
                        .collect(),
                );
                j += 1;
            }
            out.push(Block::Table {
                align,
                header,
                rows,
            });
            i = j;
            after_blank = false;
            continue;
        }
        if let Some((mind, _cind, ordered, _)) = list_marker(line) {
            if depth < 4 || mind == 0 {
                let (block, next) = parse_list(lines, i, ordered, depth);
                out.push(block);
                i = next;
                after_blank = false;
                continue;
            }
        }
        if after_blank && indent_of(line) >= 4 {
            let mut body = Vec::new();
            let mut j = i;
            while j < lines.len() && (is_blank(lines[j]) || indent_of(lines[j]) >= 4) {
                body.push(if is_blank(lines[j]) {
                    String::new()
                } else {
                    lines[j][4..].to_string()
                });
                j += 1;
            }
            while body.last().is_some_and(String::is_empty) {
                body.pop();
            }
            out.push(Block::Code {
                lang: None,
                lines: body,
                closed: true,
            });
            i = j;
            after_blank = false;
            continue;
        }
        // Paragraph: until blank or a block opener.
        let mut para = vec![line.trim()];
        let mut j = i + 1;
        while j < lines.len() && !is_blank(lines[j]) && !starts_block(lines[j]) {
            if lines[j].contains('|') && j + 1 < lines.len() && is_table_sep(lines[j + 1]) {
                break;
            }
            para.push(lines[j].trim());
            j += 1;
        }
        out.push(Block::Paragraph(parse_inline(&para.join(" "))));
        i = j;
        after_blank = false;
    }
    out
}

/// Parse a list starting at `start`; returns the block and the next index.
fn parse_list(lines: &[&str], start: usize, ordered: bool, depth: usize) -> (Block, usize) {
    let mut items = Vec::new();
    let mut i = start;
    let base_indent = list_marker(lines[start]).map(|m| m.0).unwrap_or(0);
    while i < lines.len() {
        let Some((mind, cind, ord, number)) = list_marker(lines[i]) else {
            break;
        };
        if ord != ordered || mind != base_indent {
            break;
        }
        let first = lines[i][cind.min(lines[i].len())..].to_string();
        let mut body: Vec<String> = vec![first];
        let mut j = i + 1;
        while j < lines.len() {
            let l = lines[j];
            if is_blank(l) {
                // Blank inside an item only if more indented content follows.
                let next = lines.get(j + 1).copied().unwrap_or("");
                if !is_blank(next) && indent_of(next) >= cind.clamp(1, 2) && indent_of(next) > mind
                {
                    body.push(String::new());
                    j += 1;
                    continue;
                }
                break;
            }
            let ind = indent_of(l);
            let nested_marker = list_marker(l).is_some() && ind > mind;
            if ind >= cind || nested_marker {
                let cut = ind.min(cind);
                body.push(l[cut..].to_string());
                j += 1;
                continue;
            }
            break;
        }
        let refs: Vec<&str> = body.iter().map(String::as_str).collect();
        items.push(ListItem {
            number,
            blocks: parse_lines(&refs, depth + 1),
        });
        i = j;
        // Skip blank lines between sibling items.
        while i < lines.len() && is_blank(lines[i]) {
            if lines.get(i + 1).is_some_and(|l| {
                list_marker(l).is_some_and(|m| m.0 == base_indent && m.2 == ordered)
            }) {
                i += 1;
            } else {
                break;
            }
        }
    }
    (Block::List { ordered, items }, i)
}

// ---------------------------------------------------------------------------
// Inline parsing
// ---------------------------------------------------------------------------

/// Parse inline markdown into flat styled runs.
pub fn parse_inline(text: &str) -> Vec<Run> {
    let chars: Vec<char> = text.chars().collect();
    let mut runs: Vec<Run> = Vec::new();
    let mut buf = String::new();
    let mut bold = false;
    let mut italic = false;
    let mut strike = false;
    let mut i = 0usize;
    let flush = |buf: &mut String, runs: &mut Vec<Run>, bold: bool, italic: bool, strike: bool| {
        if buf.is_empty() {
            return;
        }
        runs.push(Run {
            text: std::mem::take(buf),
            bold,
            italic,
            strike,
            ..Run::default()
        });
    };
    while i < chars.len() {
        let c = chars[i];
        // Escapes.
        if c == '\\' && i + 1 < chars.len() && chars[i + 1].is_ascii_punctuation() {
            buf.push(chars[i + 1]);
            i += 2;
            continue;
        }
        // Code span: match the same-length backtick run.
        if c == '`' {
            let n = chars[i..].iter().take_while(|c| **c == '`').count();
            if let Some(end) = find_run(&chars, i + n, '`', n) {
                flush(&mut buf, &mut runs, bold, italic, strike);
                let code: String = chars[i + n..end].iter().collect();
                let code = code.trim().to_string();
                runs.push(Run {
                    text: code,
                    code: true,
                    ..Run::default()
                });
                i = end + n;
                continue;
            }
        }
        // Links: [text](url)
        if c == '[' {
            if let Some((label, url, end)) = parse_link(&chars, i) {
                flush(&mut buf, &mut runs, bold, italic, strike);
                for mut r in parse_inline(&label) {
                    r.url = Some(url.clone());
                    r.bold |= bold;
                    r.italic |= italic;
                    runs.push(r);
                }
                i = end;
                continue;
            }
        }
        // Autolink <http://…>
        if c == '<' {
            if let Some(end) = chars[i + 1..].iter().position(|c| *c == '>') {
                let inner: String = chars[i + 1..i + 1 + end].iter().collect();
                if inner.starts_with("http://") || inner.starts_with("https://") {
                    flush(&mut buf, &mut runs, bold, italic, strike);
                    runs.push(Run {
                        text: inner.clone(),
                        url: Some(inner),
                        ..Run::default()
                    });
                    i += end + 2;
                    continue;
                }
            }
        }
        // Bare URL.
        if (c == 'h')
            && (starts_with_at(&chars, i, "http://") || starts_with_at(&chars, i, "https://"))
        {
            let mut end = i;
            while end < chars.len()
                && !chars[end].is_whitespace()
                && chars[end] != '<'
                && chars[end] != '>'
            {
                end += 1;
            }
            while end > i && matches!(chars[end - 1], '.' | ',' | ')' | ';' | ':' | '!' | '?') {
                end -= 1;
            }
            let url: String = chars[i..end].iter().collect();
            flush(&mut buf, &mut runs, bold, italic, strike);
            runs.push(Run {
                text: url.clone(),
                url: Some(url),
                ..Run::default()
            });
            i = end;
            continue;
        }
        // Strikethrough.
        if c == '~' && i + 1 < chars.len() && chars[i + 1] == '~' {
            let closes = strike || find_run(&chars, i + 2, '~', 2).is_some();
            if closes {
                flush(&mut buf, &mut runs, bold, italic, strike);
                strike = !strike;
                i += 2;
                continue;
            }
        }
        // Bold.
        if (c == '*' || c == '_') && i + 1 < chars.len() && chars[i + 1] == c {
            let closes = bold || find_run(&chars, i + 2, c, 2).is_some();
            if closes && (c == '*' || boundary(&chars, i, 2)) {
                flush(&mut buf, &mut runs, bold, italic, strike);
                bold = !bold;
                i += 2;
                continue;
            }
        }
        // Italic.
        if c == '*' || c == '_' {
            let closes = italic || find_run(&chars, i + 1, c, 1).is_some();
            let ok = if c == '_' {
                boundary(&chars, i, 1)
            } else {
                true
            };
            let not_space_after = italic || chars.get(i + 1).is_some_and(|n| !n.is_whitespace());
            if closes && ok && not_space_after {
                flush(&mut buf, &mut runs, bold, italic, strike);
                italic = !italic;
                i += 1;
                continue;
            }
        }
        buf.push(c);
        i += 1;
    }
    flush(&mut buf, &mut runs, bold, italic, strike);
    runs
}

fn starts_with_at(chars: &[char], i: usize, pat: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    chars.len() >= i + p.len() && chars[i..i + p.len()] == p[..]
}

/// Index of the next run of exactly `n` `ch` chars at or after `from`.
fn find_run(chars: &[char], from: usize, ch: char, n: usize) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == ch {
            let k = chars[i..].iter().take_while(|c| **c == ch).count();
            if k == n {
                return Some(i);
            }
            i += k;
        } else {
            i += 1;
        }
    }
    None
}

/// Underscore emphasis only at word boundaries (intraword `snake_case` stays literal).
fn boundary(chars: &[char], i: usize, n: usize) -> bool {
    let before = i.checked_sub(1).map(|k| chars[k]);
    let after = chars.get(i + n).copied();
    let before_word = before.is_some_and(|c| c.is_alphanumeric());
    let after_word = after.is_some_and(|c| c.is_alphanumeric());
    !(before_word && after_word)
}

fn parse_link(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    let mut depth = 0i32;
    let mut i = start;
    let mut close = None;
    while i < chars.len() {
        match chars[i] {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let close = close?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = chars[close + 2..].iter().position(|c| *c == ')')? + close + 2;
    let label: String = chars[start + 1..close].iter().collect();
    let url: String = chars[close + 2..end].iter().collect();
    let url = url.split_whitespace().next().unwrap_or("").to_string();
    Some((label, url, end + 1))
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render a full markdown body.
pub fn render(text: &str, opts: &MdOptions, theme: Theme) -> Vec<Line<'static>> {
    let blocks = parse_blocks(text);
    let mut out = Vec::new();
    render_blocks(&blocks, opts, theme, 0, &mut out);
    if out.is_empty() {
        out.push(Line::from(Span::styled(String::new(), theme.body())));
    }
    out
}

/// Inline-only rendering for user bodies (`R-MD-12`): block syntax is literal.
pub fn render_inline_only(text: &str, width: usize, theme: Theme) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        if para.trim().is_empty() {
            out.push(Line::from(Span::styled(String::new(), theme.body())));
            continue;
        }
        let runs = parse_inline(para);
        let chunks = runs_to_chunks(&runs, theme, theme.body(), width);
        for row in wrap::wrap_styled(&chunks, width.max(1)) {
            out.push(chunks_to_line(row));
        }
    }
    if out.is_empty() {
        out.push(Line::from(Span::styled(String::new(), theme.body())));
    }
    out
}

fn chunks_to_line(chunks: Vec<Chunk>) -> Line<'static> {
    Line::from(
        chunks
            .into_iter()
            .map(|c| Span::styled(c.text, c.style))
            .collect::<Vec<_>>(),
    )
}

/// Runs → styled chunks. Link URLs are appended in dim parentheses when the
/// whole paragraph still fits the width (`R-MD-07`).
fn runs_to_chunks(runs: &[Run], theme: Theme, base: Style, width: usize) -> Vec<Chunk> {
    let bg = base.bg.unwrap_or(theme.bg);
    let build = |with_urls: bool| -> Vec<Chunk> {
        let mut out = Vec::new();
        let mut last_url: Option<String> = None;
        for r in runs {
            if r.code {
                out.push(Chunk::new(
                    format!(" {} ", r.text),
                    Style::default().fg(theme.code_fg).bg(theme.code_bg),
                ));
                continue;
            }
            let mut st = base;
            if r.bold {
                st = st.add_modifier(Modifier::BOLD);
            }
            if r.italic {
                st = st.add_modifier(Modifier::ITALIC);
            }
            if r.strike {
                st = st.add_modifier(Modifier::CROSSED_OUT);
            }
            if let Some(url) = &r.url {
                st = st.fg(theme.link).add_modifier(Modifier::UNDERLINED);
                out.push(Chunk::new(r.text.clone(), st));
                let differs = r.text.trim() != url.trim();
                let same_link_continues = last_url.as_deref() == Some(url.as_str());
                if with_urls && differs && !same_link_continues {
                    out.push(Chunk::new(
                        format!(" ({url})"),
                        Style::default().fg(theme.dim).bg(bg),
                    ));
                }
                last_url = Some(url.clone());
                continue;
            }
            last_url = None;
            out.push(Chunk::new(r.text.clone(), st));
        }
        out
    };
    let with = build(true);
    let total: usize = with.iter().map(|c| wrap::width(&c.text)).sum();
    if total <= width { with } else { build(false) }
}

fn render_blocks(
    blocks: &[Block],
    opts: &MdOptions,
    theme: Theme,
    depth: usize,
    out: &mut Vec<Line<'static>>,
) {
    let width = opts.width.max(4);
    for (bi, b) in blocks.iter().enumerate() {
        if bi > 0 {
            out.push(Line::from(Span::styled(String::new(), theme.body())));
        }
        match b {
            Block::Paragraph(runs) => {
                let chunks = runs_to_chunks(runs, theme, theme.body(), width);
                for row in wrap::wrap_styled(&chunks, width) {
                    out.push(chunks_to_line(row));
                }
            }
            Block::Heading { level, inline } => render_heading(*level, inline, width, theme, out),
            Block::Code {
                lang,
                lines,
                closed,
            } => {
                render_code(lang.as_deref(), lines, *closed, opts, theme, out);
            }
            Block::List { ordered, items } => render_list(*ordered, items, opts, theme, depth, out),
            Block::Quote(inner) => {
                let mut rows = Vec::new();
                let sub = MdOptions {
                    width: width.saturating_sub(2).max(4),
                    ..opts.clone()
                };
                render_blocks(inner, &sub, theme, depth, &mut rows);
                for row in rows {
                    let mut spans = vec![Span::styled("▏ ", theme.muted())];
                    spans.extend(row.spans.into_iter().map(|s| {
                        let st = if s.style.fg == Some(theme.fg) {
                            s.style.fg(theme.dim)
                        } else {
                            s.style
                        };
                        Span::styled(s.content, st)
                    }));
                    out.push(Line::from(spans));
                }
            }
            Block::Rule => out.push(Line::from(Span::styled("─".repeat(width), theme.muted()))),
            Block::Table {
                align,
                header,
                rows,
            } => render_table(align, header, rows, width, theme, out),
        }
    }
}

fn render_heading(
    level: u8,
    inline: &[Run],
    width: usize,
    theme: Theme,
    out: &mut Vec<Line<'static>>,
) {
    let fg = if level <= 3 { theme.fg } else { theme.dim };
    let base = Style::default()
        .fg(fg)
        .bg(theme.bg)
        .add_modifier(Modifier::BOLD);
    let prefix = format!("{} ", "#".repeat(level as usize));
    let pw = wrap::width(&prefix);
    let chunks = runs_to_chunks(inline, theme, base, width.saturating_sub(pw));
    for (i, row) in wrap::wrap_styled(&chunks, width.saturating_sub(pw).max(1))
        .into_iter()
        .enumerate()
    {
        let mut spans = vec![Span::styled(
            if i == 0 {
                prefix.clone()
            } else {
                " ".repeat(pw)
            },
            theme.muted(),
        )];
        spans.extend(row.into_iter().map(|c| Span::styled(c.text, c.style)));
        out.push(Line::from(spans));
    }
    match level {
        1 => out.push(Line::from(Span::styled("─".repeat(width), theme.muted()))),
        2 => out.push(Line::from(Span::styled(
            "─".repeat((width / 2).max(4)),
            theme.muted(),
        ))),
        _ => {}
    }
}

fn render_code(
    lang: Option<&str>,
    lines: &[String],
    closed: bool,
    opts: &MdOptions,
    theme: Theme,
    out: &mut Vec<Line<'static>>,
) {
    let width = opts.width.max(8);
    let border = Style::default().fg(theme.dim).bg(theme.code_bg);
    let code = theme.code();
    let expanded: Vec<String> = lines.iter().map(|l| wrap::expand_tabs(l)).collect();
    let hl = highlight::highlight(lang, opts.lang_hint.as_deref(), &expanded, theme);
    let label = hl.lang.clone().or_else(|| lang.map(str::to_string));
    let mut title = label.map(|l| format!(" {l} ")).unwrap_or_default();
    if hl.skipped {
        title.push_str(&format!(
            "(highlighting skipped — {}+ lines) ",
            highlight::SKIP_LINES
        ));
    }
    let title = wrap::truncate(&title, width.saturating_sub(4));
    let fill = width.saturating_sub(2 + wrap::width(&title) + 1);
    out.push(Line::from(vec![
        Span::styled("╭─", border),
        Span::styled(title, border.add_modifier(Modifier::ITALIC)),
        Span::styled("─".repeat(fill), border),
        Span::styled("╮", border),
    ]));
    let gutter = if opts.line_numbers && lines.len() >= 4 && width >= 60 {
        lines.len().to_string().len() + 1
    } else {
        0
    };
    let inner = width.saturating_sub(4 + gutter).max(1);
    for (i, row) in hl.rows.iter().enumerate() {
        let text: String = row.iter().map(|c| c.text.as_str()).collect();
        let pieces = wrap::hard_wrap(&text, inner, 2);
        let mut consumed = 0usize;
        for (pi, piece) in pieces.iter().enumerate() {
            let mut spans = vec![Span::styled("│ ", border)];
            if gutter > 0 {
                let num = if pi == 0 {
                    wrap::pad_left(&(i + 1).to_string(), gutter - 1)
                } else {
                    " ".repeat(gutter - 1)
                };
                spans.push(Span::styled(format!("{num} "), border));
            }
            let mut used = 0usize;
            if pi > 0 {
                spans.push(Span::styled("↳ ", border));
                used += 2;
            }
            // Re-slice the styled row for this piece.
            let start = consumed;
            let end = consumed + piece.len();
            consumed = end;
            for c in slice_chunks(row, start, end) {
                used += wrap::width(&c.text);
                spans.push(Span::styled(c.text, c.style));
            }
            let pad = inner.saturating_sub(used);
            spans.push(Span::styled(" ".repeat(pad), code));
            spans.push(Span::styled(" │", border));
            out.push(Line::from(spans));
        }
        if pieces.is_empty() {
            let mut spans = vec![Span::styled("│ ", border)];
            spans.push(Span::styled(" ".repeat(inner + gutter), code));
            spans.push(Span::styled(" │", border));
            out.push(Line::from(spans));
        }
    }
    let tail = if closed { "─" } else { "┄" };
    out.push(Line::from(vec![
        Span::styled("╰", border),
        Span::styled(tail.repeat(width.saturating_sub(2)), border),
        Span::styled("╯", border),
    ]));
}

/// Byte-slice a row of chunks to `[start, end)` of its concatenated text.
fn slice_chunks(row: &[Chunk], start: usize, end: usize) -> Vec<Chunk> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    for c in row {
        let c_start = pos;
        let c_end = pos + c.text.len();
        pos = c_end;
        if c_end <= start || c_start >= end {
            continue;
        }
        let a = start.max(c_start) - c_start;
        let b = end.min(c_end) - c_start;
        if let Some(s) = c.text.get(a..b) {
            if !s.is_empty() {
                out.push(Chunk::new(s, c.style));
            }
        }
    }
    out
}

const BULLETS: [&str; 4] = ["•", "◦", "▪", "·"];

fn render_list(
    ordered: bool,
    items: &[ListItem],
    opts: &MdOptions,
    theme: Theme,
    depth: usize,
    out: &mut Vec<Line<'static>>,
) {
    let width = opts.width.max(4);
    let digits = if ordered {
        items
            .iter()
            .enumerate()
            .map(|(i, it)| it.number.unwrap_or(i as u32 + 1).to_string().len())
            .max()
            .unwrap_or(1)
    } else {
        0
    };
    let marker_w = if ordered { digits + 1 } else { 1 };
    let text_col = marker_w + 1;
    let sub = MdOptions {
        width: width.saturating_sub(text_col).max(4),
        ..opts.clone()
    };
    let mut counter = 0u32;
    for it in items {
        counter = it.number.unwrap_or(counter + 1);
        let marker = if ordered {
            wrap::pad_left(&format!("{counter}."), marker_w)
        } else {
            BULLETS[depth % BULLETS.len()].to_string()
        };
        let mut rows = Vec::new();
        render_blocks(&it.blocks, &sub, theme, depth + 1, &mut rows);
        if rows.is_empty() {
            rows.push(Line::from(Span::styled(String::new(), theme.body())));
        }
        for (i, row) in rows.into_iter().enumerate() {
            let lead = if i == 0 {
                format!("{marker} ")
            } else {
                " ".repeat(text_col)
            };
            let mut spans = vec![Span::styled(lead, theme.muted())];
            spans.extend(row.spans);
            out.push(Line::from(spans));
        }
    }
}

fn render_table(
    align: &[Align],
    header: &[Vec<Run>],
    rows: &[Vec<Vec<Run>>],
    width: usize,
    theme: Theme,
    out: &mut Vec<Line<'static>>,
) {
    let cols = header
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(0))
        .max(1);
    let cell_text = |runs: &[Run]| -> String { runs.iter().map(|r| r.text.as_str()).collect() };
    let mut widths = vec![1usize; cols];
    for (i, c) in header.iter().enumerate() {
        widths[i] = widths[i].max(wrap::width(&cell_text(c)));
    }
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            if i < cols {
                widths[i] = widths[i].max(wrap::width(&cell_text(c)));
            }
        }
    }
    // Clamp: shrink the widest columns until the frame fits.
    let frame = |w: &[usize]| w.iter().sum::<usize>() + 3 * w.len() + 1;
    while frame(&widths) > width && widths.iter().any(|w| *w > 3) {
        if let Some((i, _)) = widths.iter().enumerate().max_by_key(|(_, w)| **w) {
            widths[i] -= 1;
        }
    }
    // Still too wide: drop trailing columns and mark the last visible one.
    let mut visible = widths.len();
    let mut truncated_cols = false;
    while visible > 1 && frame(&widths[..visible]) > width {
        visible -= 1;
        truncated_cols = true;
    }
    let widths = &widths[..visible];
    let border = theme.muted();
    let rule = |l: &str, m: &str, r: &str| -> Line<'static> {
        let mut s = String::from(l);
        for (i, w) in widths.iter().enumerate() {
            s.push_str(&"─".repeat(w + 2));
            s.push_str(if i + 1 == widths.len() { r } else { m });
        }
        Line::from(Span::styled(s, border))
    };
    let cell_line = |cells: &[Vec<Run>], header_row: bool| -> Line<'static> {
        let mut spans = vec![Span::styled("│", border)];
        for (i, w) in widths.iter().enumerate() {
            let runs = cells.get(i).cloned().unwrap_or_default();
            let mut text = cell_text(&runs);
            let last_and_truncated = truncated_cols && i + 1 == widths.len();
            let clipped = wrap::width(&text) > *w;
            if last_and_truncated || clipped {
                // Cell content that does not fit is cut and marked `▸` (never wrapped).
                let (head, _) = wrap::take_width(&text, w.saturating_sub(1));
                text = format!("{head}▸");
            }
            let tw = wrap::width(&text);
            let pad = w.saturating_sub(tw);
            let (l, r) = match align.get(i).copied().unwrap_or(Align::Left) {
                Align::Left => (0, pad),
                Align::Right => (pad, 0),
                Align::Center => (pad / 2, pad - pad / 2),
            };
            let bg = if header_row { theme.panel_bg } else { theme.bg };
            let mut st = Style::default().fg(theme.fg).bg(bg);
            if header_row {
                st = st.add_modifier(Modifier::BOLD);
            }
            let bold = runs.iter().all(|r| r.bold) && !runs.is_empty();
            if bold {
                st = st.add_modifier(Modifier::BOLD);
            }
            spans.push(Span::styled(format!(" {}", " ".repeat(l)), st));
            spans.push(Span::styled(text, st));
            spans.push(Span::styled(format!("{} ", " ".repeat(r)), st));
            spans.push(Span::styled("│", border));
        }
        Line::from(spans)
    };
    out.push(rule("┌", "┬", "┐"));
    out.push(cell_line(header, true));
    out.push(rule("├", "┼", "┤"));
    for r in rows {
        out.push(cell_line(r, false));
    }
    out.push(rule("└", "┴", "┘"));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn rows(md: &str, width: usize) -> Vec<String> {
        let opts = MdOptions {
            width,
            line_numbers: true,
            lang_hint: None,
        };
        render(md, &opts, Theme::truecolor_dark())
            .iter()
            .map(text)
            .collect()
    }

    #[test]
    fn block_types_table_driven() {
        let cases: Vec<(&str, Vec<&str>)> = vec![
            ("# Title", vec!["# Title", "────"]),
            ("## Sub", vec!["## Sub"]),
            ("### Deep", vec!["### Deep"]),
            ("plain paragraph", vec!["plain paragraph"]),
            (
                "```rust\nfn x() {}\n```",
                vec!["╭─ rust", "│ fn x() {}", "╰──"],
            ),
            ("~~~\nraw\n~~~", vec!["│ raw"]),
            ("    indented code", vec!["│ indented code"]),
            ("- a\n- b", vec!["• a", "• b"]),
            ("* star", vec!["• star"]),
            ("+ plus", vec!["• plus"]),
            ("1. one\n2. two", vec!["1. one", "2. two"]),
            ("> quoted", vec!["▏ quoted"]),
            ("> outer\n> > inner", vec!["▏ outer", "▏ ▏ inner"]),
            ("---", vec!["────"]),
            ("***", vec!["────"]),
            (
                "| a | b |\n| --- | --- |\n| 1 | 2 |",
                vec!["┌", "│ a", "│ 1", "└"],
            ),
        ];
        for (md, expect) in cases {
            let got = rows(md, 40);
            for e in expect {
                assert!(
                    got.iter().any(|r| r.contains(e)),
                    "{md:?}: expected {e:?} in {got:?}"
                );
            }
        }
    }

    #[test]
    fn inline_types_table_driven() {
        let t = Theme::truecolor_dark();
        type Check = Box<dyn Fn(&Run) -> bool>;
        let cases: Vec<(&str, Check)> = vec![
            ("**bold**", Box::new(|r| r.bold && r.text == "bold")),
            ("*it*", Box::new(|r| r.italic && r.text == "it")),
            ("_it_", Box::new(|r| r.italic && r.text == "it")),
            ("`code`", Box::new(|r| r.code && r.text == "code")),
            ("~~gone~~", Box::new(|r| r.strike && r.text == "gone")),
            (
                "<https://x.y>",
                Box::new(|r| r.url.as_deref() == Some("https://x.y")),
            ),
            (
                "see https://x.y/z.",
                Box::new(|r| r.url.as_deref() == Some("https://x.y/z")),
            ),
            (
                "[t](https://u)",
                Box::new(|r| r.text == "t" && r.url.as_deref() == Some("https://u")),
            ),
            (
                "\\*literal\\*",
                Box::new(|r| r.text == "*literal*" && !r.italic),
            ),
            ("snake_case_name", Box::new(|r| r.text == "snake_case_name")),
        ];
        for (md, check) in cases {
            let runs = parse_inline(md);
            assert!(runs.iter().any(check), "{md:?} → {runs:?}");
        }
        let lines = render_inline_only("# not a heading", 40, t);
        assert_eq!(text(&lines[0]), "# not a heading");
    }

    #[test]
    fn nested_lists_to_four_levels() {
        let md = "- one\n  - two\n    - three\n      - four";
        let got = rows(md, 60);
        assert!(
            got.iter().any(|r| r.trim_start().starts_with("• one")),
            "{got:?}"
        );
        assert!(got.iter().any(|r| r.contains("◦ two")), "{got:?}");
        assert!(got.iter().any(|r| r.contains("▪ three")), "{got:?}");
        assert!(got.iter().any(|r| r.contains("· four")), "{got:?}");
        // Continuation lines align to the text column, not the marker.
        let got = rows(
            "- a very long list item that must wrap onto a second row",
            30,
        );
        assert!(got[0].starts_with("• "));
        assert!(got[1].starts_with("  "), "{got:?}");
        assert!(got.iter().all(|r| wrap::width(r) <= 30), "{got:?}");
        // Ordered numbers right-aligned in the marker column.
        let got = rows("9. nine\n10. ten", 40);
        assert!(got.iter().any(|r| r.starts_with(" 9. nine")), "{got:?}");
        assert!(got.iter().any(|r| r.starts_with("10. ten")), "{got:?}");
    }

    #[test]
    fn gfm_table_honors_alignment_and_clamps() {
        let md = "| l | c | r |\n| :--- | :---: | ---: |\n| a | bb | ccc |";
        let got = rows(md, 40);
        let body = got.iter().find(|r| r.contains("ccc")).unwrap();
        assert!(body.contains("│ a "), "{body}");
        assert!(body.contains(" ccc │"), "{body}");
        // Wider than the pane: truncated with ▸, never wrapped.
        let md = "| aaaaaaaaaaaaaaaaaaaa | bbbbbbbbbbbbbbbbbbbb | cccccccccccccccccccc |\n| --- | --- | --- |\n| 1 | 2 | 3 |";
        let got = rows(md, 24);
        assert!(got.iter().all(|r| wrap::width(r) <= 24), "{got:?}");
        assert!(got.iter().any(|r| r.contains('▸')), "{got:?}");
    }

    #[test]
    fn unterminated_fence_still_renders_as_code() {
        let got = rows("intro\n\n```rust\nfn a() {}\nlet b", 40);
        assert!(got.iter().any(|r| r.contains("│ fn a() {}")), "{got:?}");
        assert!(got.iter().any(|r| r.contains("│ let b")), "{got:?}");
        assert!(
            got.last().unwrap().contains('┄'),
            "open fence has a dashed foot: {got:?}"
        );
        assert!(got[0].contains("intro"));
    }

    #[test]
    fn code_gutter_and_hard_wrap() {
        let long = "x".repeat(100);
        let md = format!("```\n{long}\nl2\nl3\nl4\n```");
        let got = rows(&md, 70);
        assert!(got.iter().any(|r| r.contains("↳")), "{got:?}");
        assert!(got.iter().any(|r| r.contains("│ 1 ")), "{got:?}");
        assert!(got.iter().all(|r| wrap::width(r) <= 70), "{got:?}");
    }

    #[test]
    fn links_append_url_only_when_room() {
        let wide = rows("[docs](https://example.com/d)", 60);
        assert!(wide[0].contains("(https://example.com/d)"), "{wide:?}");
        let narrow = rows("[docs](https://example.com/d)", 12);
        assert_eq!(narrow[0].trim(), "docs");
    }
}
