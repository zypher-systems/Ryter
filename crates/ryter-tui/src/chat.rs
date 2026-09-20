//! Chat transcript: wrap, markdown, user right / assistant left.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::Theme;
use crate::view::{LogLine, View};
use ryter_core::Phase;

/// Last path segment of a model id (`anthropic/claude-sonnet-4.6` → `claude-sonnet-4.6`).
pub fn short_model(id: &str) -> &str {
    id.rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(id)
}

/// Pre-wrapped rows for the chat pane (no Paragraph wrap).
///
/// The latest user prompt stays at the top of the viewport. Everything after
/// it (assistant, tools) fills the rest and scrolls from the bottom. Older
/// turns leave the pane when the next user message is submitted.
pub fn render(view: &View, width: u16, height: u16, theme: Theme) -> Vec<Line<'static>> {
    let w = width.max(8) as usize;
    let h = height as usize;
    if h == 0 {
        return Vec::new();
    }
    let last_user = view
        .lines
        .iter()
        .rposition(|l| matches!(l, LogLine::User(_)));
    let Some(idx) = last_user else {
        return take_tail(render_range(&view.lines, 0, view.lines.len(), w, theme), h);
    };
    let mut pin = render_one(&view.lines[idx], w, theme);
    let max_pin = h.saturating_sub(2).max(1).min((h / 3).max(1));
    if pin.len() > max_pin {
        pin.truncate(max_pin);
    }
    pin.push(blank(theme));
    let rest_h = h.saturating_sub(pin.len());
    let rest = take_tail(
        render_range(&view.lines, idx + 1, view.lines.len(), w, theme),
        rest_h,
    );
    pin.extend(rest);
    pin
}

fn render_range(
    lines: &[LogLine],
    start: usize,
    end: usize,
    width: usize,
    theme: Theme,
) -> Vec<Line<'static>> {
    let mut rows: Vec<Line<'static>> = Vec::new();
    for line in lines.iter().take(end).skip(start) {
        if !rows.is_empty() {
            rows.push(blank(theme));
        }
        rows.extend(render_one(line, width, theme));
    }
    rows
}

fn render_one(line: &LogLine, width: usize, theme: Theme) -> Vec<Line<'static>> {
    match line {
        LogLine::User(t) => render_user(t, width, theme),
        LogLine::Assistant(t) => render_assistant(t, width, theme),
        LogLine::Tool(t) => render_tool(t, width, theme),
        LogLine::System(t) | LogLine::Merge(t) => render_system(t, width, theme),
        LogLine::Specialist { role, text } => render_specialist(role, text, width, theme),
    }
}

fn take_tail(mut rows: Vec<Line<'static>>, h: usize) -> Vec<Line<'static>> {
    if h == 0 {
        return Vec::new();
    }
    if rows.len() > h {
        rows.drain(0..rows.len() - h);
    }
    rows
}

fn blank(theme: Theme) -> Line<'static> {
    Line::from(Span::styled(" ", theme.body()))
}

fn render_user(text: &str, width: usize, theme: Theme) -> Vec<Line<'static>> {
    let bubble = (width * 3 / 4).clamp(16, width.saturating_sub(1));
    let style = Style::default()
        .fg(theme.user)
        .bg(theme.bg)
        .add_modifier(Modifier::BOLD);
    wrap_words(text, bubble)
        .into_iter()
        .map(|s| {
            let pad = width.saturating_sub(s.chars().count());
            let mut spans = Vec::new();
            if pad > 0 {
                spans.push(Span::styled(" ".repeat(pad), theme.body()));
            }
            spans.push(Span::styled(s, style));
            Line::from(spans)
        })
        .collect()
}

fn render_specialist(role: &str, text: &str, width: usize, theme: Theme) -> Vec<Line<'static>> {
    let phase = match role {
        "planner" => Phase::Plan,
        "architect" => Phase::Architect,
        "auditor" => Phase::Audit,
        _ => Phase::Build,
    };
    let color = theme.phase(phase);
    let tag = Style::default()
        .fg(color)
        .bg(theme.bg)
        .add_modifier(Modifier::BOLD);
    let mut out = vec![Line::from(Span::styled(format!(" {role}"), tag))];
    out.extend(markdown_lines(text, width.saturating_sub(1).max(8), theme));
    out
}

fn render_assistant(text: &str, width: usize, theme: Theme) -> Vec<Line<'static>> {
    let inner = width.saturating_sub(1).max(8);
    markdown_lines(text, inner, theme)
}

fn render_tool(text: &str, width: usize, theme: Theme) -> Vec<Line<'static>> {
    let style = Style::default().fg(theme.tool).bg(theme.bg);
    wrap_words(&format!("· {text}"), width.saturating_sub(1).max(8))
        .into_iter()
        .map(|s| Line::from(Span::styled(format!(" {s}"), style)))
        .collect()
}

fn render_system(text: &str, width: usize, theme: Theme) -> Vec<Line<'static>> {
    wrap_words(text, width.saturating_sub(1).max(8))
        .into_iter()
        .map(|s| Line::from(Span::styled(format!(" {s}"), theme.muted())))
        .collect()
}

fn markdown_lines(text: &str, width: usize, theme: Theme) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut para = String::new();
    let mut table: Vec<Vec<String>> = Vec::new();
    let mut code: Option<Vec<String>> = None;
    let flush_para = |para: &mut String, out: &mut Vec<Line<'static>>| {
        if para.is_empty() {
            return;
        }
        let t = std::mem::take(para);
        for w in wrap_words(t.trim(), width) {
            out.push(line_inline(&w, theme.body(), theme));
        }
    };
    let flush_table = |table: &mut Vec<Vec<String>>, out: &mut Vec<Line<'static>>| {
        if table.is_empty() {
            return;
        }
        out.extend(render_table(std::mem::take(table), width, theme));
    };

    for raw in text.split('\n') {
        let line = raw.trim_end_matches('\r');
        if let Some(buf) = &mut code {
            if line.trim_start().starts_with("```") {
                for c in buf.drain(..) {
                    let clipped: String = c.chars().take(width).collect();
                    out.push(Line::from(Span::styled(
                        format!(" {clipped}"),
                        Style::default().fg(theme.tool).bg(theme.bg),
                    )));
                }
                code = None;
            } else {
                buf.push(line.to_string());
            }
            continue;
        }
        if line.trim_start().starts_with("```") {
            flush_para(&mut para, &mut out);
            flush_table(&mut table, &mut out);
            code = Some(Vec::new());
            continue;
        }
        if is_table_row(line) {
            flush_para(&mut para, &mut out);
            if is_table_sep(line) {
                continue;
            }
            table.push(split_cells(line));
            continue;
        }
        if !table.is_empty() {
            flush_table(&mut table, &mut out);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            flush_para(&mut para, &mut out);
            out.push(blank(theme));
            continue;
        }
        if let Some(rest) = heading(trimmed) {
            flush_para(&mut para, &mut out);
            let style = theme.body().add_modifier(Modifier::BOLD);
            for w in wrap_words(rest, width) {
                out.push(Line::from(Span::styled(w, style)));
            }
            continue;
        }
        if let Some(rest) = bullet(trimmed) {
            flush_para(&mut para, &mut out);
            let hang = 2;
            let wrapped = wrap_words(rest, width.saturating_sub(hang).max(8));
            for (i, w) in wrapped.into_iter().enumerate() {
                let prefix = if i == 0 { "• " } else { "  " };
                out.push(line_inline(&format!("{prefix}{w}"), theme.body(), theme));
            }
            continue;
        }
        if let Some((n, rest)) = numbered(trimmed) {
            flush_para(&mut para, &mut out);
            let prefix = format!("{n}. ");
            let hang = prefix.chars().count();
            let wrapped = wrap_words(rest, width.saturating_sub(hang).max(8));
            for (i, w) in wrapped.into_iter().enumerate() {
                let p = if i == 0 {
                    prefix.clone()
                } else {
                    " ".repeat(hang)
                };
                out.push(line_inline(&format!("{p}{w}"), theme.body(), theme));
            }
            continue;
        }
        if trimmed == "---" || trimmed == "***" {
            flush_para(&mut para, &mut out);
            out.push(Line::from(Span::styled(
                "─".repeat(width.min(24)),
                theme.muted(),
            )));
            continue;
        }
        if !para.is_empty() {
            para.push(' ');
        }
        para.push_str(trimmed);
    }
    flush_para(&mut para, &mut out);
    flush_table(&mut table, &mut out);
    if let Some(buf) = code {
        for c in buf {
            let clipped: String = c.chars().take(width).collect();
            out.push(Line::from(Span::styled(
                format!(" {clipped}"),
                Style::default().fg(theme.tool).bg(theme.bg),
            )));
        }
    }
    if out.is_empty() {
        out.push(blank(theme));
    }
    out
}

fn heading(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let n = t.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&n) {
        t.get(n..).map(str::trim).filter(|s| !s.is_empty())
    } else {
        None
    }
}

fn bullet(line: &str) -> Option<&str> {
    let t = line.trim_start();
    for p in ["- ", "* ", "• "] {
        if let Some(rest) = t.strip_prefix(p) {
            return Some(rest);
        }
    }
    None
}

fn numbered(line: &str) -> Option<(u32, &str)> {
    let t = line.trim_start();
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let (num, rest) = t.split_at(digits);
    let rest = rest.strip_prefix(". ")?;
    num.parse().ok().map(|n| (n, rest))
}

fn is_table_row(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.matches('|').count() >= 2
}

fn is_table_sep(line: &str) -> bool {
    let t = line.trim().trim_matches('|');
    !t.is_empty()
        && t.chars()
            .all(|c| c == '-' || c == ':' || c == '|' || c.is_whitespace())
}

fn split_cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|c| c.trim().to_string())
        .collect()
}

fn render_table(rows: Vec<Vec<String>>, width: usize, theme: Theme) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return Vec::new();
    }
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut widths = vec![0usize; cols];
    for r in &rows {
        for (i, c) in r.iter().enumerate() {
            widths[i] = widths[i].max(c.chars().count());
        }
    }
    let total: usize = widths.iter().sum::<usize>() + cols.saturating_sub(1) * 2;
    if total > width {
        let scale = width.max(1) as f64 / total as f64;
        for w in &mut widths {
            *w = ((*w as f64) * scale).floor() as usize;
            *w = (*w).max(2);
        }
    }
    let mut out = Vec::new();
    for (ri, r) in rows.iter().enumerate() {
        let mut s = String::new();
        for (i, w) in widths.iter().enumerate() {
            if i > 0 {
                s.push_str("  ");
            }
            let cell = r.get(i).map(String::as_str).unwrap_or("");
            let mut cell: String = cell.chars().take(*w).collect();
            while cell.chars().count() < *w {
                cell.push(' ');
            }
            s.push_str(&cell);
        }
        let style = if ri == 0 {
            theme.body().add_modifier(Modifier::BOLD)
        } else {
            theme.body()
        };
        out.push(Line::from(Span::styled(s, style)));
        if ri == 0 {
            let rule: String = widths
                .iter()
                .map(|w| "─".repeat(*w))
                .collect::<Vec<_>>()
                .join("  ");
            out.push(Line::from(Span::styled(rule, theme.muted())));
        }
    }
    out
}

fn line_inline(text: &str, base: Style, theme: Theme) -> Line<'static> {
    Line::from(inline_spans(text, base, theme))
}

fn inline_spans(text: &str, base: Style, theme: Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut buf = String::new();
    let mut chars = text.chars().peekable();
    let mut bold = false;
    let mut code = false;
    let flush = |buf: &mut String, bold: bool, code: bool, spans: &mut Vec<Span<'static>>| {
        if buf.is_empty() {
            return;
        }
        let mut st = if code {
            Style::default().fg(theme.tool).bg(theme.bg)
        } else {
            base
        };
        if bold && !code {
            st = st.add_modifier(Modifier::BOLD);
        }
        spans.push(Span::styled(std::mem::take(buf), st));
    };
    while let Some(c) = chars.next() {
        if !code && c == '*' && chars.peek() == Some(&'*') {
            chars.next();
            flush(&mut buf, bold, code, &mut spans);
            bold = !bold;
            continue;
        }
        if c == '`' {
            flush(&mut buf, bold, code, &mut spans);
            code = !code;
            continue;
        }
        buf.push(c);
    }
    flush(&mut buf, bold, code, &mut spans);
    if spans.is_empty() {
        spans.push(Span::styled(String::new(), base));
    }
    spans
}

fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split(' ') {
        if word.is_empty() {
            continue;
        }
        let wlen = word.chars().count();
        if cur.is_empty() {
            push_long(&mut lines, &mut cur, word, width);
        } else if cur.chars().count() + 1 + wlen <= width {
            cur.push(' ');
            cur.push_str(word);
        } else {
            lines.push(std::mem::take(&mut cur));
            push_long(&mut lines, &mut cur, word, width);
        }
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

fn push_long(lines: &mut Vec<String>, cur: &mut String, word: &str, width: usize) {
    let mut rest = word;
    while rest.chars().count() > width {
        let take: String = rest.chars().take(width).collect();
        let n = take.len();
        lines.push(take);
        rest = &rest[n..];
    }
    *cur = rest.to_string();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;
    use crate::view::{LogLine, View};
    use ryter_core::Phase;

    #[test]
    fn short_model_strips_provider_prefix() {
        assert_eq!(
            short_model("anthropic/claude-sonnet-4.6"),
            "claude-sonnet-4.6"
        );
        assert_eq!(short_model("grok-4.6"), "grok-4.6");
    }

    #[test]
    fn user_is_right_aligned_and_assistant_has_lists() {
        let mut v = View::new(Phase::Build, "x".into(), "y".into(), "p".into());
        v.lines.push(LogLine::User("hello world".into()));
        v.lines.push(LogLine::Assistant(
            "Intro sentence.\n\n- alpha\n- beta\n\n1. one\n2. two\n".into(),
        ));
        let theme = Theme::truecolor_dark();
        let rows = render(&v, 40, 20, theme);
        let joined: Vec<String> = rows.iter().map(line_text).collect();
        let user = joined.iter().find(|s| s.contains("hello world")).unwrap();
        assert!(
            user.trim_start().len() < user.len(),
            "user should be padded on the left: {user:?}"
        );
        assert!(
            joined
                .iter()
                .any(|s| s.contains('•') && s.contains("alpha")),
            "{joined:?}"
        );
        assert!(
            joined.iter().any(|s| s.contains("1.") && s.contains("one")),
            "{joined:?}"
        );
    }

    #[test]
    fn latest_user_prompt_stays_at_top() {
        let mut v = View::new(Phase::Build, "x".into(), "y".into(), "p".into());
        v.lines.push(LogLine::User("first question".into()));
        v.lines
            .push(LogLine::Assistant("old answer that can leave".into()));
        v.lines.push(LogLine::User("pin me please".into()));
        v.lines.push(LogLine::Assistant("x\n".repeat(40)));
        let rows = render(&v, 40, 10, Theme::truecolor_dark());
        let joined: Vec<String> = rows.iter().map(line_text).collect();
        let first_content = joined
            .iter()
            .find(|s| !s.trim().is_empty())
            .cloned()
            .unwrap_or_default();
        assert!(
            first_content.contains("pin me please"),
            "pinned user should be first visible: {joined:?}"
        );
        assert!(
            !joined.iter().any(|s| s.contains("first question")),
            "previous user should have left the pane: {joined:?}"
        );
    }

    #[test]
    fn specialist_is_tagged() {
        let mut v = View::new(Phase::Plan, "x".into(), "y".into(), "p".into());
        v.lines.push(LogLine::User("plan this".into()));
        v.lines.push(LogLine::Specialist {
            role: "planner".into(),
            text: "Ship a flag.".into(),
        });
        let rows = render(&v, 40, 12, Theme::truecolor_dark());
        let joined: Vec<String> = rows.iter().map(line_text).collect();
        assert!(joined.iter().any(|s| s.contains("planner")), "{joined:?}");
        assert!(
            joined.iter().any(|s| s.contains("Ship a flag")),
            "{joined:?}"
        );
    }

    #[test]
    fn table_renders_cells() {
        let mut v = View::new(Phase::Build, "x".into(), "y".into(), "p".into());
        v.lines.push(LogLine::Assistant(
            "| A | B |\n| --- | --- |\n| 1 | 2 |\n".into(),
        ));
        let rows = render(&v, 40, 12, Theme::truecolor_dark());
        let joined: Vec<String> = rows.iter().map(line_text).collect();
        assert!(
            joined.iter().any(|s| s.contains('A') && s.contains('B')),
            "{joined:?}"
        );
        assert!(
            joined.iter().any(|s| s.contains('1') && s.contains('2')),
            "{joined:?}"
        );
    }

    fn line_text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }
}
