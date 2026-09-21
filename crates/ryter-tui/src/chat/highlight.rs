//! Syntax highlighting: syntect parses, the Ryter palette colors (`R-SYN-*`).
//!
//! Bundled TextMate themes are deliberately unused; scope stacks map onto
//! theme slots so highlighting always obeys the user's palette.

use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};

use super::wrap::Chunk;
use crate::theme::Theme;

/// Blocks at or over this many lines render unhighlighted (`R-SYN-06`).
pub const SKIP_LINES: usize = 2000;

/// Result of highlighting one code block.
#[derive(Debug, Clone, PartialEq)]
pub struct Highlighted {
    /// One row of styled runs per input line.
    pub rows: Vec<Vec<Chunk>>,
    /// True when the block took the skip path.
    pub skipped: bool,
    /// Language actually used (`None` = plain text).
    pub lang: Option<String>,
}

static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();

/// The extended syntax set, loaded on first use.
pub fn syntax_set() -> &'static SyntaxSet {
    SYNTAXES.get_or_init(two_face::syntax::extra_newlines)
}

/// Find a syntax by fence token or file extension (case-insensitive).
pub fn resolve_lang(token: &str) -> Option<&'static SyntaxReference> {
    let t = token.trim().trim_start_matches('.');
    if t.is_empty() {
        return None;
    }
    let ss = syntax_set();
    let lower = t.to_ascii_lowercase();
    let alias = match lower.as_str() {
        "rs" => "rust",
        "py" => "python",
        "sh" | "shell" | "zsh" => "bash",
        "js" => "javascript",
        "ts" => "typescript",
        "yml" => "yaml",
        "md" => "markdown",
        "txt" | "text" | "plain" => return None,
        other => other,
    };
    ss.find_syntax_by_token(alias)
        .or_else(|| ss.find_syntax_by_extension(alias))
        .or_else(|| ss.find_syntax_by_token(t))
}

/// Language token from a path (`src/main.rs` → `rs`), for `R-SYN-04`.
pub fn lang_from_path(path: &str) -> Option<String> {
    let name = path.rsplit('/').next()?;
    let ext = name.rsplit_once('.').map(|(_, e)| e)?;
    if ext.is_empty() || ext.len() > 12 {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

/// Style for the innermost mapped scope on the stack (`R-SYN-03`).
pub fn scope_style(stack: &[Scope], theme: Theme) -> Style {
    let base = theme.code();
    for scope in stack.iter().rev() {
        let s = scope.build_string();
        if let Some(style) = map_scope(&s, theme, base) {
            return style;
        }
    }
    base
}

fn map_scope(scope: &str, theme: Theme, base: Style) -> Option<Style> {
    // (prefix, slot). Longest matching prefix wins within one scope.
    let table: &[(&str, Color, Modifier)] = &[
        ("comment", theme.syn_comment, Modifier::ITALIC),
        ("string", theme.syn_string, Modifier::empty()),
        ("constant.character", theme.syn_string, Modifier::empty()),
        ("constant.numeric", theme.syn_number, Modifier::empty()),
        ("constant.language", theme.syn_number, Modifier::empty()),
        ("keyword", theme.syn_keyword, Modifier::BOLD),
        ("storage", theme.syn_keyword, Modifier::BOLD),
        ("storage.type", theme.syn_type, Modifier::empty()),
        (
            "entity.name.function",
            theme.syn_function,
            Modifier::empty(),
        ),
        ("support.function", theme.syn_function, Modifier::empty()),
        ("entity.name.type", theme.syn_type, Modifier::empty()),
        ("entity.name.class", theme.syn_type, Modifier::empty()),
        ("support.type", theme.syn_type, Modifier::empty()),
        ("variable.parameter", theme.syn_variable, Modifier::empty()),
        (
            "variable.other.member",
            theme.syn_variable,
            Modifier::empty(),
        ),
        ("punctuation", theme.syn_punct, Modifier::empty()),
        ("meta.brace", theme.syn_punct, Modifier::empty()),
        ("entity.name.tag", theme.syn_attr, Modifier::empty()),
        ("meta.attribute", theme.syn_attr, Modifier::empty()),
        ("meta.annotation", theme.syn_attr, Modifier::empty()),
        ("invalid", theme.error, Modifier::empty()),
    ];
    let mut best: Option<(usize, Color, Modifier)> = None;
    for (prefix, color, m) in table {
        let hit = scope == *prefix || scope.starts_with(&format!("{prefix}."));
        if hit && best.is_none_or(|(len, _, _)| prefix.len() > len) {
            best = Some((prefix.len(), *color, *m));
        }
    }
    best.map(|(_, color, m)| base.fg(color).add_modifier(m))
}

/// Highlight a block. `lang` is the fence token; `hint` is a file extension
/// guessed from a preceding tool row (`R-SYN-04`).
pub fn highlight(
    lang: Option<&str>,
    hint: Option<&str>,
    lines: &[String],
    theme: Theme,
) -> Highlighted {
    let base = theme.code();
    if lines.len() >= SKIP_LINES {
        return Highlighted {
            rows: lines
                .iter()
                .map(|l| vec![Chunk::new(l.clone(), base)])
                .collect(),
            skipped: true,
            lang: lang.map(str::to_string),
        };
    }
    let token = lang.map(str::trim).filter(|s| !s.is_empty());
    if matches!(token, Some("diff" | "patch" | "udiff")) {
        return Highlighted {
            rows: lines.iter().map(|l| vec![diff_line(l, theme)]).collect(),
            skipped: false,
            lang: Some("diff".into()),
        };
    }
    let syntax = token
        .and_then(resolve_lang)
        .or_else(|| hint.and_then(resolve_lang));
    let Some(syntax) = syntax else {
        return Highlighted {
            rows: lines
                .iter()
                .map(|l| vec![Chunk::new(l.clone(), base)])
                .collect(),
            skipped: false,
            lang: None,
        };
    };
    let ss = syntax_set();
    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut rows = Vec::with_capacity(lines.len());
    for line in lines {
        let mut with_nl = line.clone();
        with_nl.push('\n');
        let ops = match state.parse_line(&with_nl, ss) {
            Ok(ops) => ops,
            Err(_) => {
                rows.push(vec![Chunk::new(line.clone(), base)]);
                continue;
            }
        };
        let mut row: Vec<Chunk> = Vec::new();
        let mut last = 0usize;
        for (idx, op) in &ops {
            let idx = (*idx).min(line.len());
            if idx > last {
                push_run(
                    &mut row,
                    &line[last..idx],
                    scope_style(stack.as_slice(), theme),
                );
                last = idx;
            }
            let _ = stack.apply(op);
        }
        if last < line.len() {
            push_run(
                &mut row,
                &line[last..],
                scope_style(stack.as_slice(), theme),
            );
        }
        if row.is_empty() {
            row.push(Chunk::new(String::new(), base));
        }
        rows.push(row);
    }
    Highlighted {
        rows,
        skipped: false,
        lang: Some(syntax.name.to_ascii_lowercase()),
    }
}

fn push_run(row: &mut Vec<Chunk>, text: &str, style: Style) {
    if text.is_empty() {
        return;
    }
    match row.last_mut() {
        Some(c) if c.style == style => c.text.push_str(text),
        _ => row.push(Chunk::new(text, style)),
    }
}

/// `R-SYN-07`: diff coloring independent of syntect.
fn diff_line(line: &str, theme: Theme) -> Chunk {
    let base = theme.code();
    let style = if line.starts_with("+++") || line.starts_with("---") {
        base.fg(theme.dim).add_modifier(Modifier::BOLD)
    } else if line.starts_with("@@") {
        base.fg(theme.accent)
    } else if line.starts_with('+') {
        base.fg(theme.success)
    } else if line.starts_with('-') {
        base.fg(theme.error)
    } else if line.starts_with("diff ") || line.starts_with("index ") {
        base.fg(theme.dim).add_modifier(Modifier::BOLD)
    } else {
        base.fg(theme.dim)
    };
    Chunk::new(line, style)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(str::to_string).collect()
    }

    #[test]
    fn rust_block_has_three_or_more_colors() {
        let t = Theme::truecolor_dark();
        let src = lines(
            "// a comment\nfn main() {\n    let x: u32 = 42;\n    println!(\"hi {}\", x);\n}\n",
        );
        let h = highlight(Some("rust"), None, &src, t);
        assert!(!h.skipped);
        let colors: HashSet<Option<Color>> = h.rows.iter().flatten().map(|c| c.style.fg).collect();
        assert!(colors.len() >= 3, "{colors:?}");
        // The comment row is italic in the comment slot.
        assert!(h.rows[0].iter().any(|c| c.style.fg == Some(t.syn_comment)));
    }

    #[test]
    fn diff_lines_use_success_and_error() {
        let t = Theme::truecolor_dark();
        let h = highlight(
            Some("diff"),
            None,
            &lines("--- a\n+++ b\n@@ -1 +1 @@\n-old\n+new\n ctx"),
            t,
        );
        assert_eq!(h.rows[3][0].style.fg, Some(t.error));
        assert_eq!(h.rows[4][0].style.fg, Some(t.success));
        assert_eq!(h.rows[2][0].style.fg, Some(t.accent));
        assert_eq!(h.rows[5][0].style.fg, Some(t.dim));
    }

    #[test]
    fn unknown_language_does_not_panic() {
        let t = Theme::truecolor_dark();
        let h = highlight(Some("no-such-lang-xyz"), None, &lines("hello\nworld"), t);
        assert_eq!(h.rows.len(), 2);
        assert!(h.lang.is_none());
        let h = highlight(None, Some("rs"), &lines("fn x() {}"), t);
        assert_eq!(h.lang.as_deref(), Some("rust"));
    }

    #[test]
    fn huge_block_takes_skip_path() {
        let t = Theme::truecolor_dark();
        let src: Vec<String> = (0..SKIP_LINES)
            .map(|i| format!("let x{i} = {i};"))
            .collect();
        let h = highlight(Some("rust"), None, &src, t);
        assert!(h.skipped);
        assert!(h.rows.iter().all(|r| r.len() == 1));
    }

    #[test]
    fn scope_mapping_prefers_longest_prefix() {
        let t = Theme::truecolor_dark();
        let s = Scope::new("storage.type.rust").unwrap();
        assert_eq!(scope_style(&[s], t).fg, Some(t.syn_type));
        let s = Scope::new("storage.modifier.rust").unwrap();
        assert_eq!(scope_style(&[s], t).fg, Some(t.syn_keyword));
        assert_eq!(
            lang_from_path("crates/x/src/main.rs").as_deref(),
            Some("rs")
        );
        assert_eq!(lang_from_path("Makefile"), None);
    }
}
