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

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Literal(char),
    /// `?`
    One,
    /// `*`
    Run,
    /// `**/` as a whole segment (the separator after it included); at the
    /// end of the pattern, `**` or `/**`.
    Segments { tail: bool },
    Class { negated: bool, ranges: Vec<(char, char)> },
}

/// A compiled pattern.
#[derive(Debug, Clone)]
pub struct Pattern {
    text: String,
    options: Options,
    tokens: Vec<Token>,
}

impl Pattern {
    pub fn new(text: &str) -> Result<Pattern, Error> {
        Pattern::with_options(text, Options::default())
    }

    pub fn with_options(text: &str, options: Options) -> Result<Pattern, Error> {
        Ok(Pattern {
            text: text.to_string(),
            options,
            tokens: compile(text)?,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn matches(&self, path: &str) -> bool {
        let s: Vec<char> = path.chars().collect();
        Matcher {
            options: self.options,
            path: &s,
        }
        .run(&self.tokens, 0)
    }
}

fn compile(text: &str) -> Result<Vec<Token>, Error> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                let Some(&next) = chars.get(i + 1) else {
                    return Err(Error::TrailingEscape { position: i });
                };
                tokens.push(Token::Literal(next));
                i += 2;
            }
            '*' => {
                let at_segment_start = i == 0 || chars[i - 1] == '/';
                let double = chars.get(i + 1) == Some(&'*');
                let after = if double { chars.get(i + 2) } else { chars.get(i + 1) };
                if double && at_segment_start && (after.is_none() || after == Some(&'/')) {
                    if after.is_none() {
                        // A trailing `/**` also matches the path that ends
                        // before the `/`; a bare `**` is every path.
                        if matches!(tokens.last(), Some(Token::Literal('/'))) {
                            tokens.pop();
                            tokens.push(Token::Segments { tail: true });
                        } else {
                            tokens.push(Token::Segments { tail: false });
                        }
                        i += 2;
                    } else {
                        tokens.push(Token::Segments { tail: false });
                        i += 3;
                    }
                } else {
                    tokens.push(Token::Run);
                    i += 1;
                    while chars.get(i) == Some(&'*') {
                        i += 1;
                    }
                }
            }
            '?' => {
                tokens.push(Token::One);
                i += 1;
            }
            '[' => {
                let start = i;
                i += 1;
                let negated = matches!(chars.get(i), Some('!') | Some('^'));
                if negated {
                    i += 1;
                }
                let mut ranges = Vec::new();
                let mut first = true;
                loop {
                    let Some(&ch) = chars.get(i) else {
                        return Err(Error::UnterminatedClass { position: start });
                    };
                    if ch == ']' && !first {
                        i += 1;
                        break;
                    }
                    first = false;
                    if chars.get(i + 1) == Some(&'-') && chars.get(i + 2).is_some_and(|c| *c != ']') {
                        ranges.push((ch, chars[i + 2]));
                        i += 3;
                    } else {
                        ranges.push((ch, ch));
                        i += 1;
                    }
                }
                tokens.push(Token::Class { negated, ranges });
            }
            c => {
                tokens.push(Token::Literal(c));
                i += 1;
            }
        }
    }
    Ok(tokens)
}

struct Matcher<'a> {
    options: Options,
    path: &'a [char],
}

impl Matcher<'_> {
    fn same(&self, a: char, b: char) -> bool {
        a == b || (self.options.case_insensitive && a.to_ascii_lowercase() == b.to_ascii_lowercase())
    }

    fn in_class(&self, c: char, negated: bool, ranges: &[(char, char)]) -> bool {
        if c == '/' {
            return false;
        }
        let hit = |c: char| ranges.iter().any(|(lo, hi)| *lo <= c && c <= *hi);
        let found = hit(c)
            || (self.options.case_insensitive
                && (hit(c.to_ascii_lowercase()) || hit(c.to_ascii_uppercase())));
        found != negated
    }

    /// Whether a wildcard may stand for the character at `at`.
    fn wildcard_may_take(&self, at: usize) -> bool {
        let c = self.path[at];
        if c == '/' {
            return false;
        }
        let segment_start = at == 0 || self.path[at - 1] == '/';
        !(self.options.require_literal_leading_dot && segment_start && c == '.')
    }

    fn run(&self, tokens: &[Token], at: usize) -> bool {
        let Some((head, rest)) = tokens.split_first() else {
            return at == self.path.len();
        };
        // A segment that starts with `.` is matched only by a pattern
        // segment that starts with a literal `.`, when that is required.
        if self.options.require_literal_leading_dot
            && at < self.path.len()
            && self.path[at] == '.'
            && (at == 0 || self.path[at - 1] == '/')
            && !matches!(head, Token::Literal('.') | Token::Segments { .. })
        {
            return false;
        }
        match head {
            Token::Literal(c) => {
                at < self.path.len() && self.same(self.path[at], *c) && self.run(rest, at + 1)
            }
            Token::One => at < self.path.len() && self.wildcard_may_take(at) && self.run(rest, at + 1),
            Token::Run => {
                let mut end = at;
                loop {
                    if self.run(rest, end) {
                        return true;
                    }
                    if end < self.path.len() && self.wildcard_may_take(end) {
                        end += 1;
                    } else {
                        return false;
                    }
                }
            }
            Token::Class { negated, ranges } => {
                at < self.path.len()
                    && self.wildcard_may_take(at)
                    && self.in_class(self.path[at], *negated, ranges)
                    && self.run(rest, at + 1)
            }
            Token::Segments { tail } => {
                if *tail {
                    // The rest of the path, if any, is one or more segments
                    // under here: nothing, or `/` and then anything whose
                    // segments may be skipped.
                    if at == self.path.len() {
                        return true;
                    }
                    if self.path[at] != '/' {
                        return false;
                    }
                    return self.skips_segments(at + 1, &[]);
                }
                self.skips_segments(at, rest)
            }
        }
    }

    /// Zero or more whole segments from `at` (a segment start), then `rest`.
    fn skips_segments(&self, at: usize, rest: &[Token]) -> bool {
        if rest.is_empty() && at == self.path.len() {
            return true;
        }
        if self.run(rest, at) {
            return true;
        }
        if at >= self.path.len() {
            return false;
        }
        if self.options.require_literal_leading_dot && self.path[at] == '.' {
            return false;
        }
        // Skip this segment.
        let mut end = at;
        while end < self.path.len() && self.path[end] != '/' {
            end += 1;
        }
        if end == self.path.len() {
            // The last segment: skipping it leaves nothing, which `rest`
            // must accept.
            return rest.is_empty();
        }
        self.skips_segments(end + 1, rest)
    }
}
