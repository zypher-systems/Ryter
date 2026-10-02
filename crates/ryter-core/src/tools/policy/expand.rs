//! What the shell does to a word before the command sees it.
//!
//! The gate judges the paths a command is given. The shell rewrites some
//! words first: `.en?` becomes `.env`, and `~/.ss{h,x}/id_rsa` becomes two
//! paths. Judged as written, `cat ~/.s?h/id_rsa` named no key, and ran.
//!
//! So the gate does what the shell would do and judges what comes out.
//! Where it can't (a variable in the pattern, more matches than it will
//! read), the word is marked as one only the shell can read, and is treated
//! as a variable is.

use std::path::{Path, PathBuf};

/// A glob character the shell would not act on, because it was quoted or
/// escaped, is kept as a character from a private range, so `'*.rs'` and
/// `*.rs` stay different words. [`plain`] puts the real ones back.
const QUOTED: [(char, char); 4] = [
    ('*', '\u{E000}'),
    ('?', '\u{E001}'),
    ('[', '\u{E002}'),
    ('{', '\u{E003}'),
];

/// Added to a word the gate could not expand: it holds a `$`, so every
/// check reads it as a word only the shell can read.
pub(super) const UNREAD: &str = "$*";

/// The most matches one word is expanded to, and the most folder entries
/// looked at for it. Past either, the word is one the gate can't read.
const MAX_MATCHES: usize = 2000;
const MAX_LOOKED_AT: usize = 50_000;
/// The most words a `{a,b}` list is expanded to.
const MAX_LISTED: usize = 256;

/// `c` as it is kept when the shell would not act on it.
pub(super) fn quoted(c: char) -> char {
    QUOTED
        .iter()
        .find(|(real, _)| *real == c)
        .map_or(c, |(_, kept)| *kept)
}

fn real(c: char) -> char {
    QUOTED
        .iter()
        .find(|(_, kept)| *kept == c)
        .map_or(c, |(real, _)| *real)
}

/// The word as the command would see it, with no expansion.
pub(super) fn plain(word: &str) -> String {
    word.chars().map(real).collect()
}

/// True when `text` holds one of the private characters already: a command
/// written with them can't be told from one the gate marked.
pub(super) fn has_private(text: &str) -> bool {
    text.chars().any(|c| QUOTED.iter().any(|(_, k)| *k == c))
}

/// Where a relative pattern starts, and where `~` is.
pub(super) struct Places<'a> {
    /// The folder the command runs in. `None` when the gate can't tell.
    pub cwd: Option<&'a Path>,
    pub home: Option<&'a Path>,
}

fn has_glob(word: &str) -> bool {
    word.contains(['*', '?', '['])
}

/// The words `word` stands for once the shell has expanded its lists
/// (`{a,b}`) and patterns (`*`, `?`, `[…]`): each as the command would be
/// given it. A word that expands to nothing the gate can list comes back
/// once, ending in [`UNREAD`].
pub(super) fn expand(word: &str, at: &Places) -> Vec<String> {
    if !word.contains(['*', '?', '[', '{']) {
        return vec![plain(word)];
    }
    let unread = || vec![format!("{}{UNREAD}", plain(word))];
    let mut listed = Vec::new();
    if !braces(word, &mut listed) {
        return unread();
    }
    let mut out = Vec::new();
    for alt in listed {
        if !has_glob(&alt) {
            out.push(plain(&alt));
            continue;
        }
        match glob(&alt, at) {
            // No match: the shell hands the pattern over as it is written.
            Some(found) if found.is_empty() => out.push(plain(&alt)),
            Some(found) => out.extend(found),
            None => return unread(),
        }
        if out.len() > MAX_MATCHES {
            return unread();
        }
    }
    out
}

/// Expand the first `{a,b}` list in `word`, and the lists in what that
/// leaves. `false` when there are too many to list.
fn braces(word: &str, out: &mut Vec<String>) -> bool {
    let mut chars: Vec<char> = word.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // `${NAME}` is a variable, not a list.
        if chars[i] == '{' && (i == 0 || chars[i - 1] != '$') {
            if let Some((end, parts, run)) = group(&chars, i) {
                let pre: String = chars[..i].iter().collect();
                let post: String = chars[end + 1..].iter().collect();
                if parts.len() > 1 {
                    for part in parts {
                        if !braces(&format!("{pre}{part}{post}"), out) {
                            return false;
                        }
                    }
                    return true;
                }
                // `{a..e}`, `{01..10}`: a run.
                if let Some(list) = parts.first().filter(|_| run).and_then(|p| run_of(p)) {
                    for part in list {
                        if !braces(&format!("{pre}{part}{post}"), out) {
                            return false;
                        }
                    }
                    return true;
                }
            }
            // `{x}`, or no closing brace: the shell leaves it as written.
            chars[i] = quoted('{');
        }
        i += 1;
    }
    if out.len() >= MAX_LISTED {
        return false;
    }
    out.push(chars.into_iter().collect());
    true
}

/// The words a run stands for: `a..e`, `1..10`, `01..10..3`. `None` when
/// it isn't one, and the shell leaves it as written.
fn run_of(inner: &str) -> Option<Vec<String>> {
    let parts: Vec<&str> = inner.split("..").collect();
    let (from, to) = match parts.as_slice() {
        [from, to] | [from, to, _] => (*from, *to),
        _ => return None,
    };
    let step = match parts.get(2) {
        Some(s) => s.parse::<i64>().ok()?.unsigned_abs().max(1) as i64,
        None => 1,
    };
    let list = |a: i64, b: i64| -> Vec<i64> {
        let mut out = Vec::new();
        let mut n = a;
        // One past the limit, so the caller sees there were too many.
        while (a <= b && n <= b || a > b && n >= b) && out.len() <= MAX_LISTED {
            out.push(n);
            n += if a <= b { step } else { -step };
        }
        out
    };
    if let (Ok(a), Ok(b)) = (from.parse::<i64>(), to.parse::<i64>()) {
        let padded = |s: &str| s.trim_start_matches('-').starts_with('0') && s.len() > 1;
        let width = if padded(from) || padded(to) {
            from.len().max(to.len())
        } else {
            0
        };
        return Some(
            list(a, b)
                .into_iter()
                .map(|n| format!("{n:0width$}"))
                .collect(),
        );
    }
    let letter = |s: &str| {
        let mut c = s.chars();
        match (c.next(), c.next()) {
            (Some(l), None) if l.is_ascii_alphabetic() => Some(l as i64),
            _ => None,
        }
    };
    let (a, b) = (letter(from)?, letter(to)?);
    Some(
        list(a, b)
            .into_iter()
            .filter_map(|n| u8::try_from(n).ok())
            .map(|n| (n as char).to_string())
            .collect(),
    )
}

/// The `{…}` that opens at `open`: where it closes, its comma-separated
/// parts, and whether it is a run (`a..z`).
fn group(chars: &[char], open: usize) -> Option<(usize, Vec<String>, bool)> {
    let mut depth = 0;
    let mut parts = Vec::new();
    let mut cur = String::new();
    for (j, &c) in chars.iter().enumerate().skip(open) {
        match c {
            '{' => {
                depth += 1;
                if depth > 1 {
                    cur.push(c);
                }
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let run = parts.is_empty() && cur.contains("..");
                    parts.push(cur);
                    return Some((j, parts, run));
                }
                cur.push(c);
            }
            ',' if depth == 1 => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    None
}

/// The paths that match `pattern`, written as the shell would write them.
/// Empty when nothing matches; `None` when the gate can't tell (a variable
/// in the pattern, a folder it doesn't know, too many matches).
fn glob(pattern: &str, at: &Places) -> Option<Vec<String>> {
    // Where the pattern starts, and how that start is written.
    let (start, written, rest) = if let Some(rest) = pattern.strip_prefix("~/") {
        let home = at.home?;
        (home.to_path_buf(), format!("{}/", home.display()), rest)
    } else if let Some(rest) = pattern
        .strip_prefix("$HOME/")
        .or_else(|| pattern.strip_prefix("${HOME}/"))
        .or_else(|| pattern.strip_prefix("$\u{E003}HOME}/"))
    {
        let home = at.home?;
        (home.to_path_buf(), format!("{}/", home.display()), rest)
    } else if pattern.contains('$') || pattern.starts_with('~') {
        return None;
    } else if pattern.starts_with('/') {
        (
            PathBuf::from("/"),
            "/".to_string(),
            pattern.trim_start_matches('/'),
        )
    } else {
        (at.cwd?.to_path_buf(), String::new(), pattern)
    };
    if rest.contains('$') {
        return None;
    }
    let mut found = vec![(start, written)];
    let mut matched = false;
    let mut looked_at = 0;
    let parts: Vec<&str> = rest.split('/').collect();
    let join = |written: &str, name: &str| {
        if written.is_empty() || written.ends_with('/') {
            format!("{written}{name}")
        } else {
            format!("{written}/{name}")
        }
    };
    for (k, part) in parts.iter().enumerate() {
        if part.is_empty() {
            // `src/*/`: folders only, written with the slash.
            if k + 1 == parts.len() {
                found.retain(|(path, _)| path.is_dir());
                for (_, written) in &mut found {
                    if !written.ends_with('/') {
                        written.push('/');
                    }
                }
            }
            continue;
        }
        if !has_glob(part) {
            let name = plain(part);
            for (path, written) in &mut found {
                path.push(&name);
                *written = join(written, &name);
            }
            // After a pattern, a name has to be there to be a match.
            if matched {
                found.retain(|(path, _)| path.symlink_metadata().is_ok());
            }
            continue;
        }
        matched = true;
        let want: Vec<char> = squeeze(part);
        // A name that starts with a dot is matched only by a pattern that
        // starts with one: `*` is not `.env`. `.` and `..` are names too,
        // to a pattern that starts with a dot.
        let dotted = want.first() == Some(&'.');
        let mut next = Vec::new();
        for (path, written) in &found {
            let Ok(entries) = std::fs::read_dir(path) else {
                continue;
            };
            let mut names: Vec<String> = entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            if dotted {
                names.push(".".into());
                names.push("..".into());
            }
            names.sort();
            for name in names {
                looked_at += 1;
                if looked_at > MAX_LOOKED_AT {
                    return None;
                }
                if name.starts_with('.') && !dotted {
                    continue;
                }
                let has: Vec<char> = name.chars().collect();
                if matches(&want, &has) {
                    next.push((path.join(&name), join(written, &name)));
                    if next.len() > MAX_MATCHES {
                        return None;
                    }
                }
            }
        }
        found = next;
    }
    Some(found.into_iter().map(|(_, written)| written).collect())
}

/// `part` as characters, with a run of `*` as one.
fn squeeze(part: &str) -> Vec<char> {
    let mut out: Vec<char> = Vec::new();
    for c in part.chars() {
        if c == '*' && out.last() == Some(&'*') {
            continue;
        }
        out.push(c);
    }
    out
}

/// Whether `name` matches `pattern`. A set (`[a-z]`) matches any one
/// character: which ones is not worked out, and matching too much only
/// means more is judged.
pub(super) fn matches(pattern: &[char], name: &[char]) -> bool {
    match pattern.first() {
        None => name.is_empty(),
        Some('*') => (0..=name.len()).any(|k| matches(&pattern[1..], &name[k..])),
        Some('?') => !name.is_empty() && matches(&pattern[1..], &name[1..]),
        Some('[') => match set_end(pattern) {
            Some(end) => !name.is_empty() && matches(&pattern[end + 1..], &name[1..]),
            None => name.first() == Some(&'[') && matches(&pattern[1..], &name[1..]),
        },
        Some(&c) => name.first() == Some(&real(c)) && matches(&pattern[1..], &name[1..]),
    }
}

/// Where the set that opens `pattern` closes. A `]` straight after the
/// opening (or after `!`/`^`) is a member, not the end.
fn set_end(pattern: &[char]) -> Option<usize> {
    let mut i = 1;
    if matches!(pattern.get(i), Some('!' | '^')) {
        i += 1;
    }
    if pattern.get(i) == Some(&']') {
        i += 1;
    }
    (i..pattern.len()).find(|&j| pattern[j] == ']')
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn at<'a>(cwd: &'a Path, home: &'a Path) -> Places<'a> {
        Places {
            cwd: Some(cwd),
            home: Some(home),
        }
    }

    /// The shell's own answer: what `bash` hands a command for `word`.
    fn bash(word: &str, cwd: &Path, home: &Path) -> Vec<String> {
        let out = std::process::Command::new("bash")
            .arg("-c")
            .arg(format!("printf '%s\\n' {word}"))
            .current_dir(cwd)
            .env("HOME", home)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// Every path the shell would hand the command is one the gate lists.
    /// The gate may list more (a set matches any character here), never
    /// fewer: a path it didn't list is a path it didn't judge.
    #[test]
    fn what_the_shell_expands_to_the_gate_lists() {
        let dir = TempDir::new().unwrap();
        let cwd = dir.path().join("proj");
        let home = dir.path().join("home");
        for f in [
            "proj/.env",
            "proj/.envrc",
            "proj/a.txt",
            "proj/b.txt",
            "proj/src/main.rs",
            "proj/src/lib.rs",
            "proj/src/.hidden",
            "proj/sub/deep/x.md",
            "home/.ssh/id_rsa",
            "home/.ssh/config",
            "home/.aws/credentials",
            "home/notes.txt",
        ] {
            let p = dir.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "x").unwrap();
        }
        for word in [
            "*",
            ".*",
            ".en?",
            ".e*",
            ".en[v]",
            ".e{nv,x}",
            "{.env,a.txt}",
            "*.txt",
            "src/*.rs",
            "src/*",
            "src/.*",
            "*/*.rs",
            "*/",
            "s*/d*/*.md",
            "[ab].txt",
            "[!a].txt",
            "{a,b}.{txt,md}",
            "a.{t..u}xt",
            "{a..c}.txt",
            "f{01..12..5}",
            "f{3..1}",
            "~/.s?h/id_rsa",
            "~/.ss*/*",
            "~/.ss{h,x}/id_rsa",
            "~/{.ssh,.aws}/*",
            "{~/.ssh,~/.aws}/c*",
            "$HOME/.s?h/id_rsa",
            "${HOME}/.ssh/id_*",
            "~/*",
            "nothing*here",
            "src/{main,lib}.rs",
            "./.en*",
            "../home/.ssh/id_*",
            "s??/*.rs",
            "**/*.rs",
        ] {
            let lexed = super::super::lex(&format!("x {word}")).remove(1);
            // A word with no pattern keeps its `~`: the gate reads that as
            // the user's folder wherever it judges a path.
            let ours: Vec<String> = expand(&lexed, &at(&cwd, &home))
                .into_iter()
                .map(|w| {
                    match ["~/", "$HOME/", "${HOME}/"]
                        .iter()
                        .find_map(|p| w.strip_prefix(p))
                    {
                        Some(rest) => format!("{}/{rest}", home.display()),
                        None => w,
                    }
                })
                .collect();
            for theirs in bash(word, &cwd, &home) {
                assert!(
                    ours.contains(&theirs),
                    "{word}: the shell gives {theirs}, the gate listed {ours:?}"
                );
            }
        }
        // Quoted, a pattern is a name.
        for (written, means) in [
            ("'*.txt'", "*.txt"),
            ("\"*.txt\"", "*.txt"),
            ("\\*.txt", "*.txt"),
            ("'.en?'", ".en?"),
            ("'{a,b}.txt'", "{a,b}.txt"),
        ] {
            let lexed = super::super::lex(&format!("x {written}")).remove(1);
            assert_eq!(expand(&lexed, &at(&cwd, &home)), [means], "{written}");
        }
        // What can't be listed is marked, not passed as a name.
        let lexed = super::super::lex("x $DIR/*.txt").remove(1);
        assert_eq!(
            expand(&lexed, &at(&cwd, &home)),
            [format!("$DIR/*.txt{UNREAD}")]
        );
        let nowhere = Places {
            cwd: None,
            home: Some(&home),
        };
        assert_eq!(
            expand("*.txt", &nowhere),
            [format!("*.txt{UNREAD}")],
            "a pattern in a folder the gate doesn't know"
        );
        assert_eq!(expand("~/.s?h/id_rsa", &nowhere).len(), 1);
        assert!(expand("~/.s?h/id_rsa", &nowhere)[0].ends_with("/.ssh/id_rsa"));
    }
}
