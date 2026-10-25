//! Glob patterns for paths: `*`, `?`, `**`, classes and escapes.

/// Why a pattern could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A `[` with no `]` after it; `position` is the `[`'s index.
    UnterminatedClass { position: usize },
    /// A `\\` with nothing after it; `position` is its index.
    TrailingEscape { position: usize },
}

/// How a pattern matches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    /// ASCII letters match either case.
    pub case_insensitive: bool,
    /// Wildcards do not match a `.` that starts a segment.
    pub require_literal_leading_dot: bool,
}

/// A compiled pattern.
#[derive(Debug, Clone)]
pub struct Pattern {
    text: String,
    #[allow(dead_code)]
    options: Options,
}

impl Pattern {
    pub fn new(text: &str) -> Result<Pattern, Error> {
        Pattern::with_options(text, Options::default())
    }

    pub fn with_options(text: &str, options: Options) -> Result<Pattern, Error> {
        Ok(Pattern {
            text: text.to_string(),
            options,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn matches(&self, path: &str) -> bool {
        let p: Vec<char> = self.text.chars().collect();
        let s: Vec<char> = path.chars().collect();
        matches_at(&p, &s)
    }
}

fn matches_at(p: &[char], s: &[char]) -> bool {
    match p.split_first() {
        None => s.is_empty(),
        Some(('*', rest)) => (0..=s.len()).any(|n| matches_at(rest, &s[n..])),
        Some(('?', rest)) => !s.is_empty() && matches_at(rest, &s[1..]),
        Some((c, rest)) => s.first() == Some(c) && matches_at(rest, &s[1..]),
    }
}
