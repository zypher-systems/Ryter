//! The project's verbs: what the model wants done, mapped to the project's
//! own commands. `test` runs the run file's test commands, and narrowed to a
//! name or to files it runs the runner's own filter, so the model never
//! writes `-k`, `-t`, `-run` or `-p` itself, and never runs the whole suite
//! to see one test.
//!
//! The commands come from the run file the user approved. Narrowing adds
//! the runner's filter to that command; it never swaps the runner for
//! another program. A command Ryter can't narrow is said so, not run whole.

use std::collections::{BTreeMap, BTreeSet};
use std::iter::Peekable;
use std::path::{Component, Path, PathBuf};
use std::str::CharIndices;

/// The most files the walk for test files visits.
const MAX_WALK: usize = 20_000;

/// A test runner whose filter Ryter knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runner {
    /// `cargo test`, `cargo nextest run`: a positional name, `-p` a package.
    Cargo,
    /// `pytest`: `-k`, test files as paths.
    Pytest,
    /// `go test`: `-run`, packages as `./dir`.
    Go,
    /// `jest`: `-t`, `--findRelatedTests` for files.
    Jest,
    /// `vitest`: `-t`, test files as paths.
    Vitest,
    /// `mocha`: `--grep`, test files as paths.
    Mocha,
    /// `bun test`: `-t`, test files as paths.
    Bun,
}

impl Runner {
    fn name(self) -> &'static str {
        match self {
            Self::Cargo => "cargo test",
            Self::Pytest => "pytest",
            Self::Go => "go test",
            Self::Jest => "jest",
            Self::Vitest => "vitest",
            Self::Mocha => "mocha",
            Self::Bun => "bun test",
        }
    }
}

/// A test command narrowed to what was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Narrowed {
    /// The command to run.
    pub cmd: String,
    /// Files the run does not cover: each with why.
    pub unmatched: Vec<String>,
}

/// What narrowing one test command came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// Narrowed to the filter, the files, or both.
    Narrowed(Narrowed),
    /// The command can't be narrowed: why.
    Cannot(String),
}

/// A recognised command: the runner, and how arguments reach it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Recognised {
    runner: Runner,
    /// What goes between the command and arguments added to it: ` -- `
    /// through `npm` and `pnpm`, a space otherwise.
    joint: &'static str,
    /// Whether a script behind the command (`npm test` → `vitest run test/`)
    /// already names test paths, which a file list could not take out.
    positional: bool,
}

/// Narrow `cmd`, one of the run file's test commands, to `filter` (a test
/// name or pattern, in the runner's own sense) and `files` (paths under
/// `root`, the ones the model changed). Called with neither, it is the
/// command as it stands.
pub fn narrow(cmd: &str, filter: Option<&str>, files: &[String], root: &Path) -> Scope {
    let filter = filter.map(str::trim).filter(|f| !f.is_empty());
    if filter.is_none() && files.is_empty() {
        return Scope::Narrowed(Narrowed {
            cmd: cmd.to_string(),
            unmatched: Vec::new(),
        });
    }
    if is_compound(cmd) {
        return Scope::Cannot(
            "it is several commands in one, which Ryter can't narrow; the run file can \
             list them one by one"
                .into(),
        );
    }
    if shell_words(cmd).is_none() {
        return Scope::Cannot("the command has an unclosed quote or substitution".into());
    }
    let Some(found) = recognise(cmd, root) else {
        return Scope::Cannot(
            "it is not a runner Ryter can narrow (cargo test, pytest, go test, jest, vitest, \
             mocha, bun test)"
                .into(),
        );
    };
    let mut unmatched = Vec::new();
    let mut matched = Vec::new();
    for f in files {
        match locate(f, root) {
            Ok(rel) => matched.push(rel),
            Err(why) => unmatched.push(why),
        }
    }
    let scoped = match found.runner {
        Runner::Cargo => Ok(cargo(cmd, filter, &matched, root, &mut unmatched)),
        Runner::Pytest => pytest(cmd, &found, filter, &matched, root, &mut unmatched),
        Runner::Go => go(cmd, filter, &matched, root, &mut unmatched),
        Runner::Jest => jest(cmd, &found, filter, &matched, root, &mut unmatched),
        Runner::Vitest | Runner::Bun => {
            by_test_file(cmd, &found, "-t", filter, &matched, root, &mut unmatched)
        }
        Runner::Mocha => by_test_file(
            cmd,
            &found,
            "--grep",
            filter,
            &matched,
            root,
            &mut unmatched,
        ),
    };
    let scoped = match scoped {
        Ok(scoped) => scoped,
        Err(why) => return Scope::Cannot(why),
    };
    match scoped {
        // Files were given and none of them is covered: a run of the whole
        // suite under the name of a narrow one is not what was asked.
        None if filter.is_none() => {
            Scope::Cannot(format!("{}: {}", found.runner.name(), unmatched.join("; ")))
        }
        None => Scope::Narrowed(Narrowed {
            cmd: with_filter(cmd, &found, filter.unwrap_or_default()),
            unmatched,
        }),
        Some(cmd) => Scope::Narrowed(Narrowed { cmd, unmatched }),
    }
}

/// Whether a run that passed ran no test at all: a filter that matched
/// nothing, read in each runner's own words.
pub fn ran_nothing(output: &str) -> bool {
    // The `[exit N]` line the shell tool appends is not the runner's.
    let lines: Vec<&str> = output
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("[exit "))
        .collect();
    // cargo: `test result: ok. 0 passed; 0 failed; …` for every binary.
    let totals: Vec<&&str> = lines
        .iter()
        .filter(|l| l.starts_with("test result:"))
        .collect();
    if !totals.is_empty() {
        return totals
            .iter()
            .all(|l| l.contains(" 0 passed;") && l.contains(" 0 failed;"));
    }
    // pytest: `no tests ran in 0.01s`, or only deselected ones.
    if let Some(last) = lines.iter().rev().find(|l| !l.is_empty()) {
        if last.contains("no tests ran") {
            return true;
        }
        // `2 deselected` alone; `1 error, 2 deselected` is a run that
        // collected something and broke.
        if last.contains("deselected")
            && !last.contains("passed")
            && !last.contains("failed")
            && !last.contains("error")
        {
            return true;
        }
    }
    // go: `ok  pkg 0.002s [no tests to run]` for every package.
    let go: Vec<&&str> = lines
        .iter()
        .filter(|l| l.starts_with("ok") || l.starts_with("---") || l.starts_with("FAIL"))
        .collect();
    if !go.is_empty() && go.iter().all(|l| l.contains("[no tests to run]")) {
        return true;
    }
    // mocha: `0 passing` with nothing failing; `0 passing` over `1 failing`
    // is a failure, not an empty run.
    let failing = lines.iter().any(|l| {
        l.split_once(' ').is_some_and(|(n, rest)| {
            rest.starts_with("failing") && n.parse::<u64>().is_ok_and(|n| n > 0)
        })
    });
    // jest, vitest, mocha.
    lines.iter().any(|l| {
        l.starts_with("No tests found")
            || l.starts_with("No test files found")
            || (!failing && (*l == "0 passing" || l.starts_with("0 passing (")))
    })
}

/// `a && b`, `a | b`, `a; b`: more than one command.
fn is_compound(cmd: &str) -> bool {
    let mut quote: Option<char> = None;
    let mut prev = ' ';
    for c in cmd.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None => match c {
                '\'' | '"' => quote = Some(c),
                '&' if prev == '&' => return true,
                '|' | ';' => return true,
                _ => {}
            },
        }
        prev = c;
    }
    false
}

/// The runner `cmd` runs, through whatever wrapper: `uv run pytest`,
/// `python -m pytest`, `npx jest`, `npm test` (read from `package.json`).
fn recognise(cmd: &str, root: &Path) -> Option<Recognised> {
    recognise_line(cmd, root, 0)
}

/// `recognise`, `depth` wrappers deep: the line is split as a shell splits
/// it, so `LDFLAGS="-s -w" go test` is an assignment and then `go`.
fn recognise_line(cmd: &str, root: &Path, depth: u8) -> Option<Recognised> {
    let words = shell_words(cmd)?;
    let words: Vec<&str> = words
        .iter()
        .map(|w| w.text.as_str())
        .skip_while(|w| is_assignment(w))
        .collect();
    recognise_words(&words, root, depth)
}

fn recognise_words(words: &[&str], root: &Path, depth: u8) -> Option<Recognised> {
    let (first, rest) = words.split_first()?;
    let first = first.rsplit('/').next().unwrap_or(first);
    let direct = |runner| {
        Some(Recognised {
            runner,
            joint: " ",
            positional: false,
        })
    };
    match first {
        "cargo" => match rest {
            ["test", ..] => direct(Runner::Cargo),
            ["nextest", "run", ..] => direct(Runner::Cargo),
            _ => None,
        },
        "pytest" | "py.test" => direct(Runner::Pytest),
        "go" if rest.first() == Some(&"test") => direct(Runner::Go),
        "jest" => direct(Runner::Jest),
        "vitest" => direct(Runner::Vitest),
        "mocha" => direct(Runner::Mocha),
        "bun" => match rest {
            ["test", ..] => direct(Runner::Bun),
            ["run", script, ..] => script_runner(root, script, depth, " "),
            [script, ..] if !script.starts_with('-') => script_runner(root, script, depth, " "),
            _ => None,
        },
        // `python -m pytest`.
        p if p.starts_with("python") => match rest {
            ["-m", module, ..] => recognise_words(&[module], root, depth),
            _ => None,
        },
        // Wrappers that run the program after them.
        "uv" | "poetry" | "pipenv" | "hatch" | "pdm" => match rest {
            ["run", ..] => recognise_words(&rest[1..], root, depth),
            _ => None,
        },
        "npx" | "bunx" => recognise_words(skip_flags(rest), root, depth),
        "npm" => match rest {
            ["exec", "--", ..] => recognise_words(&rest[2..], root, depth),
            ["test" | "t" | "tst", ..] => script_runner(root, "test", depth, " -- "),
            ["run" | "run-script", script, ..] => script_runner(root, script, depth, " -- "),
            _ => None,
        },
        "pnpm" => match rest {
            ["exec", ..] => recognise_words(&rest[1..], root, depth),
            ["test" | "t" | "tst", ..] => script_runner(root, "test", depth, " -- "),
            ["run", script, ..] => script_runner(root, script, depth, " -- "),
            _ => None,
        },
        "yarn" => match rest {
            ["exec", ..] => recognise_words(&rest[1..], root, depth),
            ["test", ..] => script_runner(root, "test", depth, " "),
            ["run", script, ..] => script_runner(root, script, depth, " "),
            _ => None,
        },
        _ => None,
    }
}

/// `KEY=value` before a command.
fn is_assignment(word: &str) -> bool {
    word.split_once('=')
        .is_some_and(|(k, _)| !k.is_empty() && k.chars().all(|c| c.is_alphanumeric() || c == '_'))
}

/// The words from the first that is not an option.
fn skip_flags<'a>(words: &'a [&'a str]) -> &'a [&'a str] {
    let n = words.iter().take_while(|w| w.starts_with('-')).count();
    &words[n..]
}

/// The runner a `package.json` script runs, with how `npm`/`yarn` pass
/// arguments through to it.
fn script_runner(root: &Path, script: &str, depth: u8, joint: &'static str) -> Option<Recognised> {
    if depth > 2 {
        return None;
    }
    let text = std::fs::read_to_string(root.join("package.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let line = json.get("scripts")?.get(script)?.as_str()?;
    if is_compound(line) {
        return None;
    }
    let inner = recognise_line(line, root, depth + 1)?;
    // A path the script names itself can't be taken out through `npm test
    // -- …`: a file list would widen to it.
    let positional = inner.positional
        || match strip_positional_paths(line, inner.runner, root) {
            Ok((_, dropped)) => dropped,
            Err(_) => true,
        };
    Some(Recognised {
        runner: inner.runner,
        joint,
        positional,
    })
}

/// The runner's own word in `cmd` (through wrappers), and the index after
/// it and its subcommand (`go test`, `bun test`, `cargo test`), from which
/// the command's own arguments run.
fn arguments_start(words: &[&str], runner: Runner) -> Option<usize> {
    let names: &[&str] = match runner {
        Runner::Cargo => &["cargo"],
        Runner::Pytest => &["pytest", "py.test"],
        Runner::Go => &["go"],
        Runner::Jest => &["jest"],
        Runner::Vitest => &["vitest"],
        Runner::Mocha => &["mocha"],
        Runner::Bun => &["bun"],
    };
    let at = words.iter().position(|w| {
        let base = w.rsplit('/').next().unwrap_or(w);
        names.contains(&base)
    })?;
    Some(match runner {
        Runner::Cargo | Runner::Go | Runner::Bun => at + 2,
        _ => at + 1,
    })
}

/// Whether `flag` (without an `=`) takes the next word as its value, for
/// the runners whose positional words are paths.
fn takes_value(runner: Runner, flag: &str) -> bool {
    let list: &[&str] = match runner {
        Runner::Pytest => &[
            "-k",
            "-m",
            "-p",
            "-c",
            "-o",
            "-W",
            "-n",
            "-r",
            "--rootdir",
            "--confcutdir",
            "--tb",
            "--durations",
            "--maxfail",
            "--junitxml",
            "--basetemp",
            "--deselect",
            "--ignore",
            "--ignore-glob",
            "--import-mode",
            "--log-level",
            "--log-file",
            "--override-ini",
            "--timeout",
            "--dist",
            "--cov",
            "--cov-report",
            "--cov-config",
            "--doctest-glob",
        ],
        Runner::Vitest => &[
            "-t",
            "--testNamePattern",
            "-c",
            "--config",
            "-r",
            "--root",
            "--dir",
            "--reporter",
            "--outputFile",
            "--pool",
            "--project",
            "--environment",
            "--exclude",
            "--testTimeout",
            "--hookTimeout",
            "--maxConcurrency",
            "--shard",
            "--mode",
        ],
        Runner::Mocha => &[
            "-g",
            "--grep",
            "-f",
            "--fgrep",
            "-r",
            "--require",
            "-R",
            "--reporter",
            "-t",
            "--timeout",
            "-s",
            "--slow",
            "--file",
            "--config",
            "--package",
            "--extension",
            "-u",
            "--ui",
            "--spec",
            "-O",
            "--reporter-option",
            "--retries",
            "-j",
            "--jobs",
            "-n",
            "--node-option",
            "--global",
            "--ignore",
            "--exclude",
        ],
        Runner::Bun => &[
            "-t",
            "--test-name-pattern",
            "--timeout",
            "--preload",
            "-r",
            "--rerun-each",
            "--coverage-dir",
            "--coverage-reporter",
            "--reporter",
            "--reporter-outfile",
        ],
        Runner::Jest => &[
            "-t",
            "--testNamePattern",
            "-c",
            "--config",
            "--rootDir",
            "--roots",
            "--reporters",
            "--maxWorkers",
            "-w",
            "--testPathPattern",
            "--testPathIgnorePatterns",
            "--coverageDirectory",
            "--outputFile",
            "--selectProjects",
            "--shard",
            "--testTimeout",
        ],
        Runner::Cargo | Runner::Go => &[],
    };
    list.contains(&flag)
}

/// Words the runner takes as its own subcommands, not as paths.
fn is_subcommand(runner: Runner, word: &str) -> bool {
    match runner {
        Runner::Vitest => matches!(word, "run" | "watch" | "related" | "bench" | "dev" | "init"),
        _ => false,
    }
}

/// Whether a positional word reads as a path or a path pattern.
fn path_like(word: &str) -> bool {
    word.contains('/')
        || word.contains(['*', '?', '['])
        || word.ends_with(".py")
        || Path::new(word)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| {
                matches!(
                    e,
                    "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "mts" | "cts"
                )
            })
}

/// `cmd` without the test paths it names itself, so the files' own tests
/// can take their place: a positional word that is a folder under `root`
/// is dropped. A positional word that still reads as a path or a pattern
/// (`test/**/*.js`, `tests/unit.py`) can't be told from one the runner
/// needs, and is refused.
fn strip_positional_paths(
    cmd: &str,
    runner: Runner,
    root: &Path,
) -> std::result::Result<(String, bool), String> {
    let words = shell_words(cmd)
        .ok_or_else(|| "the command has an unclosed quote or substitution".to_string())?;
    let refs: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
    let Some(start) = arguments_start(&refs, runner) else {
        return Ok((cmd.to_string(), false));
    };
    let start = start.min(words.len());
    let mut out: Vec<Word> = words[..start].to_vec();
    let mut dropped = false;
    let mut value_next = false;
    let mut previous: Option<&str> = None;
    for word in &words[start..] {
        let w = word.text.as_str();
        if value_next {
            out.push(word.clone());
            value_next = false;
            previous = None;
            continue;
        }
        if w.starts_with('-') {
            out.push(word.clone());
            value_next = !w.contains('=') && takes_value(runner, w);
            previous = Some(w);
            continue;
        }
        if is_subcommand(runner, w) {
            out.push(word.clone());
            previous = None;
            continue;
        }
        // A word the shell expands (`${DIR:-tests}`, `"$DIR"`, `$(…)`,
        // `{a,b}`, `~`) is a test path Ryter can't see: kept, it widens the
        // run beyond the files.
        if expands(&word.raw) || w.starts_with('~') {
            return Err(format!(
                "`{}` in the command is expanded by the shell into tests of its own, which a \
                 file list can't take out; write it out in the run file, or run the runner \
                 with bash",
                word.raw
            ));
        }
        if root.join(w).is_dir() {
            // After a flag Ryter does not know, a folder may be the flag's
            // value (`--html reports`) or a test path: dropping it either
            // way turns the narrow run into the suite or breaks the flag.
            if let Some(flag) = previous.filter(|f| !f.contains('=') && !boolean_flag(runner, f)) {
                return Err(format!(
                    "`{flag} {w}` in the command: Ryter can't tell the flag's value from a test \
                     folder; write it as `{flag}={w}`, or run the runner with bash"
                ));
            }
            dropped = true;
            previous = None;
            continue;
        }
        if path_like(w) {
            return Err(format!(
                "`{w}` in the command picks tests of its own, which a file list can't take out; \
                 take it out of the run file, or run the runner with bash"
            ));
        }
        out.push(word.clone());
        previous = None;
    }
    Ok((join_words(&out), dropped))
}

/// Flags the runners take without a value, so a folder after one is a
/// test path and not the flag's value.
fn boolean_flag(runner: Runner, flag: &str) -> bool {
    let list: &[&str] = match runner {
        Runner::Pytest => &[
            "-q",
            "-qq",
            "-v",
            "-vv",
            "-s",
            "-x",
            "-l",
            "--lf",
            "--ff",
            "--nf",
            "--sw",
            "--co",
            "--collect-only",
            "--quiet",
            "--verbose",
            "--no-header",
            "--no-summary",
            "--strict",
            "--strict-markers",
            "--exitfirst",
            "--pdb",
            "--trace",
            "--runxfail",
            "--disable-warnings",
            "--cache-clear",
            "--last-failed",
            "--failed-first",
            "--new-first",
            "--stepwise",
            "--showlocals",
            "--full-trace",
            "--color",
        ],
        Runner::Vitest => &[
            "--run",
            "--watch",
            "-w",
            "--coverage",
            "--ui",
            "--silent",
            "--passWithNoTests",
            "--update",
            "-u",
            "--changed",
            "--no-color",
            "--globals",
            "--isolate",
            "--dom",
            "--browser",
            "--standalone",
            "--no-file-parallelism",
            "--typecheck",
        ],
        Runner::Mocha => &[
            "--exit",
            "--bail",
            "-b",
            "--recursive",
            "--watch",
            "-w",
            "--parallel",
            "--forbid-only",
            "--forbid-pending",
            "--check-leaks",
            "--color",
            "--no-colors",
            "--dry-run",
            "--fail-zero",
            "--allow-uncaught",
            "--async-only",
            "-A",
            "--delay",
            "--diff",
            "--full-trace",
            "--inline-diffs",
            "--list-interfaces",
            "--list-reporters",
            "--sort",
            "-S",
            "--invert",
            "-i",
        ],
        Runner::Bun => &[
            "--watch",
            "--coverage",
            "--only",
            "--todo",
            "--update-snapshots",
            "-u",
            "--bail",
            "--hot",
            "--no-clear-screen",
        ],
        Runner::Jest => &[
            "--ci",
            "--coverage",
            "--watch",
            "--watchAll",
            "--bail",
            "-b",
            "--verbose",
            "--silent",
            "--runInBand",
            "-i",
            "--passWithNoTests",
            "--detectOpenHandles",
            "--forceExit",
            "--onlyChanged",
            "-o",
            "--lastCommit",
            "--listTests",
            "--json",
            "--useStderr",
            "--noStackTrace",
            "--updateSnapshot",
            "-u",
            "--expand",
            "-e",
            "--clearCache",
            "--showConfig",
            "--debug",
            "--no-cache",
            "--colors",
            "--no-colors",
            "--findRelatedTests",
            "--onlyFailures",
            "-f",
        ],
        Runner::Cargo | Runner::Go => &[],
    };
    list.contains(&flag)
}

/// One word of a command: what the shell makes of it, and how it was
/// written. It goes back on the line as written, so a `"$VAR"`, a
/// `$(...)` or a `NAME="value"` prefix the user approved is still read by
/// the shell the way it was.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Word {
    text: String,
    raw: String,
}

impl Word {
    /// A word Ryter adds, written as one shell word.
    fn new(text: &str) -> Word {
        Word {
            text: text.to_string(),
            raw: quote(text),
        }
    }
}

/// `cmd` split as a shell splits it: quotes group, a backslash escapes, a
/// `$(…)`, `${…}` or `` `…` `` runs to its closing mark. `None` when one
/// of them is unclosed.
fn shell_words(cmd: &str) -> Option<Vec<Word>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut start = 0;
    let mut in_word = false;
    let mut chars = cmd.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if !in_word && !c.is_whitespace() {
            start = i;
        }
        match c {
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some((_, '\'')) => break,
                        Some((_, c)) => word.push(c),
                        None => return None,
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some((_, '"')) => break,
                        Some((_, '\\')) => match chars.next() {
                            Some((_, c @ ('"' | '\\' | '$' | '`'))) => word.push(c),
                            Some((_, c)) => {
                                word.push('\\');
                                word.push(c);
                            }
                            None => return None,
                        },
                        Some((_, c @ ('`' | '$'))) => substitution(&mut chars, c, &mut word)?,
                        Some((_, c)) => word.push(c),
                        None => return None,
                    }
                }
            }
            '\\' => {
                in_word = true;
                word.push(chars.next()?.1);
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(Word {
                        text: std::mem::take(&mut word),
                        raw: cmd[start..i].to_string(),
                    });
                    in_word = false;
                }
            }
            c @ ('`' | '$') => {
                in_word = true;
                substitution(&mut chars, c, &mut word)?;
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        words.push(Word {
            text: word,
            raw: cmd[start..].to_string(),
        });
    }
    Some(words)
}

/// A `$(…)`, `${…}` or `` `…` `` whose first mark `c` has been read,
/// pushed onto `word` through its closing mark, the shell's way: it is one
/// word however many spaces it holds. A bare `$` is just that. `None` when
/// unclosed.
fn substitution(chars: &mut Peekable<CharIndices>, c: char, word: &mut String) -> Option<()> {
    word.push(c);
    let (open, close) = match c {
        '`' => ('`', '`'),
        _ => match chars.peek() {
            Some(&(_, o @ ('(' | '{'))) => {
                chars.next();
                word.push(o);
                (o, if o == '(' { ')' } else { '}' })
            }
            _ => return Some(()),
        },
    };
    let mut depth = 1;
    while let Some((_, c)) = chars.next() {
        // A substitution inside this one ends on its own mark, so its
        // closer can't end this one: `${X:-$(echo a} b)}` is one word.
        if c == '$' || (c == '`' && open != '`') {
            substitution(chars, c, word)?;
            continue;
        }
        word.push(c);
        match c {
            '\\' => word.push(chars.next()?.1),
            '\'' if open != '`' => loop {
                let (_, c) = chars.next()?;
                word.push(c);
                if c == '\'' {
                    break;
                }
            },
            '"' if open != '`' => loop {
                let (_, c) = chars.next()?;
                if c == '$' || c == '`' {
                    substitution(chars, c, word)?;
                    continue;
                }
                word.push(c);
                match c {
                    '\\' => word.push(chars.next()?.1),
                    '"' => break,
                    _ => {}
                }
            },
            c if c == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(());
                }
            }
            c if c == open => depth += 1,
            _ => {}
        }
    }
    None
}

/// Whether a word as written is expanded by the shell before the program
/// sees it: a `$…`, a `` `…` `` or a `{a,b}`.
fn expands(raw: &str) -> bool {
    raw.contains(['$', '`', '{'])
}

/// Words back into one command line, each as it was written.
fn join_words(words: &[Word]) -> String {
    words
        .iter()
        .map(|w| w.raw.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `cmd` ready for a file list: its own test paths taken out, or why it
/// can't be. A script behind it that names paths is refused the same way.
fn without_own_paths(
    cmd: &str,
    found: &Recognised,
    root: &Path,
) -> std::result::Result<String, String> {
    if found.positional {
        return Err(
            "the script it runs already names test paths, which a file list can't take out; \
             run the runner itself with bash, or put it in the run file directly"
                .into(),
        );
    }
    strip_positional_paths(cmd, found.runner, root).map(|(cmd, _)| cmd)
}

/// The test files for a source file, by name: `test_{stem}.py` and
/// `{stem}_test.py` for Python, `{stem}.test.*` and `{stem}.spec.*` for a
/// script, the stem being the name before its last extension
/// (`button.component` for `button.component.ts`). Those beside the source
/// are preferred to those elsewhere.
fn tests_named_for(source: &Path, index: &[PathBuf], python: bool) -> Vec<PathBuf> {
    let stem = source.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if stem.is_empty() {
        return Vec::new();
    }
    let wanted = |name: &str| {
        if python {
            name == format!("test_{stem}.py") || name == format!("{stem}_test.py")
        } else {
            name.starts_with(&format!("{stem}.test.")) || name.starts_with(&format!("{stem}.spec."))
        }
    };
    let hits: Vec<&PathBuf> = index
        .iter()
        .filter(|t| t.file_name().and_then(|n| n.to_str()).is_some_and(wanted))
        .collect();
    let beside: Vec<&PathBuf> = hits
        .iter()
        .copied()
        .filter(|t| t.parent() == source.parent())
        .collect();
    (if beside.is_empty() { hits } else { beside })
        .into_iter()
        .cloned()
        .collect()
}

/// `file` as a path under `root`, relative, or why it isn't one.
fn locate(file: &str, root: &Path) -> std::result::Result<PathBuf, String> {
    let given = Path::new(file.trim());
    if given.as_os_str().is_empty() {
        return Err("an empty path".into());
    }
    let rel = if given.is_absolute() {
        match given.strip_prefix(root) {
            Ok(r) => r.to_path_buf(),
            Err(_) => return Err(format!("`{file}` is outside the project")),
        }
    } else {
        given.to_path_buf()
    };
    if rel.components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::Prefix(_) | Component::RootDir
        )
    }) {
        return Err(format!("`{file}` is outside the project"));
    }
    if !root.join(&rel).is_file() {
        return Err(format!("`{file}` is not a file in the project"));
    }
    Ok(rel)
}

/// `cmd` with `filter` added as the runner takes it.
fn with_filter(cmd: &str, found: &Recognised, filter: &str) -> String {
    let flag = match found.runner {
        Runner::Cargo => return cargo_insert(cmd, &[quote(filter)]),
        Runner::Pytest => "-k",
        Runner::Go => "-run",
        Runner::Jest | Runner::Vitest | Runner::Bun => "-t",
        Runner::Mocha => "--grep",
    };
    format!("{cmd}{}{flag} {}", found.joint, quote(filter))
}

/// `cmd` with `args` added, after the arguments the command already has.
fn append(cmd: &str, found: &Recognised, args: &[String]) -> String {
    if args.is_empty() {
        cmd.to_string()
    } else {
        format!("{cmd}{}{}", found.joint, args.join(" "))
    }
}

/// `arg` as one shell word.
fn quote(arg: &str) -> String {
    if !arg.is_empty()
        && arg.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '@' | '=' | ',')
        })
    {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', "'\\''"))
}

/// Cargo's own arguments go before a `--`; what follows it is the test
/// binary's.
fn cargo_insert(cmd: &str, args: &[String]) -> String {
    let (head, tail) = match cmd.find(" -- ") {
        Some(i) => (&cmd[..i], Some(&cmd[i..])),
        None => match cmd.strip_suffix(" --") {
            Some(h) => (h, Some(" --")),
            None => (cmd, None),
        },
    };
    let mut s = head.trim_end().to_string();
    for a in args {
        s.push(' ');
        s.push_str(a);
    }
    if let Some(t) = tail {
        s.push_str(t);
    }
    s
}

/// `cargo test`: the filter as its positional name, the files' packages
/// as `-p`, with `--workspace`/`--all` dropped so `-p` is heard.
fn cargo(
    cmd: &str,
    filter: Option<&str>,
    files: &[PathBuf],
    root: &Path,
    unmatched: &mut Vec<String>,
) -> Option<String> {
    let mut packages = BTreeSet::new();
    for f in files {
        match cargo_package(f, root) {
            Some(p) => {
                packages.insert(p);
            }
            None => unmatched.push(format!("`{}` is in no Cargo package", f.display())),
        }
    }
    if packages.is_empty() && !files.is_empty() {
        return None;
    }
    let mut args: Vec<String> = Vec::new();
    for p in &packages {
        args.push("-p".into());
        args.push(quote(p));
    }
    if let Some(f) = filter {
        args.push(quote(f));
    }
    let cmd = if packages.is_empty() {
        cmd.to_string()
    } else {
        // `--workspace -p x` runs the workspace: the flag has to go.
        let (head, tail) = match cmd.find(" -- ") {
            Some(i) => (&cmd[..i], &cmd[i..]),
            None => (cmd, ""),
        };
        let head: Vec<&str> = head
            .split_whitespace()
            .filter(|w| !matches!(*w, "--workspace" | "--all"))
            .collect();
        format!("{}{tail}", head.join(" "))
    };
    Some(cargo_insert(&cmd, &args))
}

/// The name of the Cargo package `rel` (under `root`) belongs to: the
/// nearest `Cargo.toml` above it with a `[package]`.
fn cargo_package(rel: &Path, root: &Path) -> Option<String> {
    let mut dir = root.join(rel).parent()?.to_path_buf();
    loop {
        let manifest = dir.join("Cargo.toml");
        if manifest.is_file() {
            if let Ok(text) = std::fs::read_to_string(&manifest) {
                if let Ok(value) = text.parse::<toml::Value>() {
                    if let Some(name) = value
                        .get("package")
                        .and_then(|p| p.get("name"))
                        .and_then(toml::Value::as_str)
                    {
                        return Some(name.to_string());
                    }
                }
            }
        }
        if dir == root || !dir.pop() || !dir.starts_with(root) {
            return None;
        }
    }
}

/// `pytest`: `-k` for the filter; test files as paths, a source file by the
/// test file named for it.
fn pytest(
    cmd: &str,
    found: &Recognised,
    filter: Option<&str>,
    files: &[PathBuf],
    root: &Path,
    unmatched: &mut Vec<String>,
) -> std::result::Result<Option<String>, String> {
    let index = if files.iter().any(|f| !is_python_test(f)) {
        test_files(root)
    } else {
        Vec::new()
    };
    let mut paths: Vec<String> = Vec::new();
    for f in files {
        if is_python_test(f) {
            paths.push(quote(&f.display().to_string()));
            continue;
        }
        let hits: Vec<String> = if f.extension().and_then(|e| e.to_str()) == Some("py") {
            tests_named_for(f, &index, true)
                .iter()
                .map(|t| quote(&t.display().to_string()))
                .collect()
        } else {
            Vec::new()
        };
        if hits.is_empty() {
            unmatched.push(format!("no test file for `{}`", f.display()));
        } else {
            paths.extend(hits);
        }
    }
    if paths.is_empty() && !files.is_empty() {
        return Ok(None);
    }
    paths.sort();
    paths.dedup();
    let cmd = if paths.is_empty() {
        cmd.to_string()
    } else {
        without_own_paths(cmd, found, root)?
    };
    let mut args = Vec::new();
    if let Some(f) = filter {
        args.push("-k".to_string());
        args.push(quote(f));
    }
    args.extend(paths);
    Ok(Some(append(&cmd, found, &args)))
}

fn is_python_test(rel: &Path) -> bool {
    let name = rel.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.ends_with(".py") && (name.starts_with("test_") || name.ends_with("_test.py"))
}

/// `go test`: `-run` for the filter; the files' directories as the
/// packages, in place of the ones the command names.
fn go(
    cmd: &str,
    filter: Option<&str>,
    files: &[PathBuf],
    root: &Path,
    unmatched: &mut Vec<String>,
) -> std::result::Result<Option<String>, String> {
    let mut dirs = BTreeSet::new();
    for f in files {
        if f.extension().and_then(|e| e.to_str()) != Some("go") {
            unmatched.push(format!("`{}` is not a Go file", f.display()));
            continue;
        }
        let dir = f
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        dirs.insert(if dir.is_empty() {
            ".".to_string()
        } else {
            format!("./{dir}")
        });
    }
    if dirs.is_empty() && !files.is_empty() {
        return Ok(None);
    }
    let words = shell_words(cmd)
        .ok_or_else(|| "the command has an unclosed quote or substitution".to_string())?;
    let refs: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
    let start = arguments_start(&refs, Runner::Go)
        .unwrap_or(words.len())
        .min(words.len());
    let mut out: Vec<Word> = words[..start].to_vec();
    let own = &words[start..];
    // Everything from `-args` on belongs to the test binary: the packages,
    // and a `-run`, go before it.
    let mut tail: &[Word] = &[];
    let mut value_next = false;
    for (i, word) in own.iter().enumerate() {
        let w = word.text.as_str();
        if value_next {
            out.push(word.clone());
            value_next = false;
            continue;
        }
        // The separator is a word of its own: a flag's value spelt `-args`
        // (`-ldflags -args`) was taken as the value above. `-test.args` is a
        // flag like any other.
        if matches!(w, "-args" | "--args") {
            tail = &own[i..];
            break;
        }
        if w.starts_with('-') {
            out.push(word.clone());
            if w.contains('=') {
                continue;
            }
            // Go's flags take their value as the next word unless they are
            // boolean. For a flag Ryter does not know, the next word is a
            // value unless it reads as a package pattern.
            let name = go_flag_name(w);
            value_next = if go_value_flag(&name) {
                true
            } else if go_boolean_flag(&name) {
                false
            } else {
                !own.get(i + 1)
                    .is_some_and(|next| go_package_word(&next.text, root))
            };
            continue;
        }
        // Every other word names packages (`./...`, `all`, `...`, a
        // directory, an import path); the files' own take their place. One
        // the shell expands (`${FLAGS:--race}`, `$(…)`, `{a,b}`) may be
        // flags as well as packages: it can't be dropped, nor kept.
        if !dirs.is_empty() {
            if expands(&word.raw) {
                return Err(format!(
                    "`{}` in the command is expanded by the shell, so Ryter can't tell its \
                     flags from its packages; write it out in the run file, or run the \
                     runner with bash",
                    word.raw
                ));
            }
            continue;
        }
        out.push(word.clone());
    }
    if let Some(f) = filter {
        out.push(Word::new("-run"));
        out.push(Word::new(f));
    }
    out.extend(dirs.iter().map(|d| Word::new(d)));
    out.extend(tail.iter().cloned());
    Ok(Some(join_words(&out)))
}

/// A Go flag's bare name: `--race` and `-test.race` are `race`.
fn go_flag_name(flag: &str) -> String {
    let name = flag.trim_start_matches('-');
    name.strip_prefix("test.").unwrap_or(name).to_string()
}

/// `go test`'s flags that take no value (`go help testflag`, `go help build`).
fn go_boolean_flag(name: &str) -> bool {
    matches!(
        name,
        "v" | "race"
            | "cover"
            | "short"
            | "json"
            | "failfast"
            | "benchmem"
            | "msan"
            | "asan"
            | "a"
            | "n"
            | "x"
            | "work"
            | "trimpath"
            | "linkshared"
            | "c"
            | "i"
            | "fullpath"
            | "buildvcs"
            | "artifacts"
            | "modcacherw"
            | "paniconexit0"
    )
}

/// `go test`'s flags whose value is a path or free text, kept whatever it
/// looks like.
fn go_value_flag(name: &str) -> bool {
    matches!(
        name,
        "coverprofile"
            | "exec"
            | "o"
            | "outputdir"
            | "C"
            | "pkgdir"
            | "toolexec"
            | "ldflags"
            | "gcflags"
            | "asmflags"
            | "gccgoflags"
            | "tags"
            | "mod"
            | "modfile"
            | "overlay"
            | "run"
            | "bench"
            | "skip"
            | "timeout"
            | "count"
            | "cpu"
            | "parallel"
            | "shuffle"
            | "coverpkg"
            | "covermode"
            | "benchtime"
            | "blockprofile"
            | "cpuprofile"
            | "memprofile"
            | "mutexprofile"
            | "trace"
            | "fuzz"
            | "fuzztime"
            | "list"
            | "p"
            | "vet"
            | "installsuffix"
            | "compiler"
            | "buildmode"
            | "pgo"
            | "cache"
    )
}

/// Whether a word in `go test` names packages rather than a flag's value.
fn go_package_word(word: &str, root: &Path) -> bool {
    matches!(word, "..." | "all" | "std" | "cmd" | "." | "./...")
        || word.ends_with("/...")
        || (word.starts_with("./") && root.join(word).is_dir())
        || (!word.contains('.') && root.join(word).is_dir())
}

/// `jest`: `-t` for the filter, `--findRelatedTests` for the files, which
/// jest maps to their tests itself (source or test).
fn jest(
    cmd: &str,
    found: &Recognised,
    filter: Option<&str>,
    files: &[PathBuf],
    root: &Path,
    unmatched: &mut Vec<String>,
) -> std::result::Result<Option<String>, String> {
    let mut related = Vec::new();
    for f in files {
        if is_script(f) {
            related.push(quote(&f.display().to_string()));
        } else {
            unmatched.push(format!("`{}` is not a script jest runs", f.display()));
        }
    }
    if related.is_empty() && !files.is_empty() {
        return Ok(None);
    }
    let cmd = if related.is_empty() {
        cmd.to_string()
    } else {
        without_own_paths(cmd, found, root)?
    };
    let mut args = Vec::new();
    if let Some(f) = filter {
        args.push("-t".to_string());
        args.push(quote(f));
    }
    if !related.is_empty() {
        args.push("--findRelatedTests".into());
        args.extend(related);
    }
    Ok(Some(append(&cmd, found, &args)))
}

/// vitest, mocha, bun test: `flag` for the filter; a test file as its path,
/// a source file by the test file named for it (`x.test.ts`, `x.spec.ts`,
/// `test/x.js`).
fn by_test_file(
    cmd: &str,
    found: &Recognised,
    flag: &str,
    filter: Option<&str>,
    files: &[PathBuf],
    root: &Path,
    unmatched: &mut Vec<String>,
) -> std::result::Result<Option<String>, String> {
    let index = if files.iter().any(|f| !is_script_test(f)) {
        test_files(root)
    } else {
        Vec::new()
    };
    let mut paths: Vec<String> = Vec::new();
    for f in files {
        if is_script_test(f) {
            paths.push(quote(&f.display().to_string()));
            continue;
        }
        let hits: Vec<String> = if is_script(f) {
            tests_named_for(f, &index, false)
                .iter()
                .map(|t| quote(&t.display().to_string()))
                .collect()
        } else {
            Vec::new()
        };
        if hits.is_empty() {
            unmatched.push(format!("no test file for `{}`", f.display()));
        } else {
            paths.extend(hits);
        }
    }
    if paths.is_empty() && !files.is_empty() {
        return Ok(None);
    }
    paths.sort();
    paths.dedup();
    let cmd = if paths.is_empty() {
        cmd.to_string()
    } else {
        without_own_paths(cmd, found, root)?
    };
    let mut args = Vec::new();
    if let Some(f) = filter {
        args.push(flag.to_string());
        args.push(quote(f));
    }
    args.extend(paths);
    Ok(Some(append(&cmd, found, &args)))
}

fn is_script(rel: &Path) -> bool {
    matches!(
        rel.extension().and_then(|e| e.to_str()),
        Some("js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "mts" | "cts")
    )
}

/// `x.test.ts`, `x.spec.js`, or anything under a `test`, `tests` or
/// `__tests__` folder.
fn is_script_test(rel: &Path) -> bool {
    if !is_script(rel) {
        return false;
    }
    let name = rel.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.contains(".test.")
        || name.contains(".spec.")
        || rel.components().any(|c| {
            matches!(
                c.as_os_str().to_str(),
                Some("test" | "tests" | "__tests__" | "spec")
            )
        })
}

/// What a project's manifest says its verbs are: a run file with the
/// commands it implies, and the file it was read from. `None` when nothing
/// at the root says how the project builds or tests. The product's own
/// `start`, `ready` and `stop` are never guessed: a server is the model's
/// to read and propose.
pub fn detect(root: &Path) -> Option<(crate::run::RunFile, &'static str)> {
    use crate::run::RunFile;
    let has = |name: &str| root.join(name).exists();
    let read = |name: &str| std::fs::read_to_string(root.join(name)).unwrap_or_default();
    if has("Cargo.toml") {
        let manifest = read("Cargo.toml");
        let workspace = manifest
            .parse::<toml::Value>()
            .ok()
            .is_some_and(|v| v.get("workspace").is_some());
        let scope = if workspace { " --workspace" } else { "" };
        let run = RunFile {
            build: Some(format!("cargo build{scope}")),
            test: vec![format!("cargo test{scope}")],
            check: Some(format!("cargo check{scope}")),
            lint: Some(format!("cargo clippy{scope} --all-targets")),
            format: Some(if workspace {
                "cargo fmt --all".to_string()
            } else {
                "cargo fmt".to_string()
            }),
            ..RunFile::default()
        };
        return Some((run, "Cargo.toml"));
    }
    if has("package.json") {
        let json: serde_json::Value = serde_json::from_str(&read("package.json")).ok()?;
        let scripts = json.get("scripts").and_then(|s| s.as_object());
        let script = |name: &str| scripts.is_some_and(|s| s.contains_key(name));
        let runner = if has("pnpm-lock.yaml") {
            "pnpm"
        } else if has("yarn.lock") {
            "yarn"
        } else if has("bun.lockb") || has("bun.lock") {
            "bun"
        } else {
            "npm"
        };
        let run_script = |name: &str| {
            if name == "test" && runner != "bun" {
                format!("{runner} test")
            } else {
                format!("{runner} run {name}")
            }
        };
        let first = |names: &[&str]| names.iter().find(|n| script(n)).map(|n| run_script(n));
        let run = RunFile {
            build: first(&["build"]),
            test: first(&["test"]).into_iter().collect(),
            check: first(&["typecheck", "check"]).or_else(|| {
                has("tsconfig.json").then(|| format!("{} tsc --noEmit", exec_of(runner)))
            }),
            lint: first(&["lint"]),
            format: first(&["format", "fmt"]),
            ..RunFile::default()
        };
        return (!run.is_empty()).then_some((run, "package.json"));
    }
    if has("go.mod") {
        let run = RunFile {
            build: Some("go build ./...".into()),
            test: vec!["go test ./...".into()],
            check: Some("go vet ./...".into()),
            format: Some("gofmt -l -w .".into()),
            ..RunFile::default()
        };
        return Some((run, "go.mod"));
    }
    if has("pyproject.toml") || has("setup.py") || has("pytest.ini") || has("setup.cfg") {
        let pyproject = read("pyproject.toml");
        let table = pyproject.parse::<toml::Value>().ok();
        let tool = |name: &str| {
            table
                .as_ref()
                .and_then(|t| t.get("tool"))
                .and_then(|t| t.get(name))
                .is_some()
        };
        let prefix = if has("uv.lock") {
            "uv run "
        } else if has("poetry.lock") {
            "poetry run "
        } else {
            ""
        };
        let pytest =
            tool("pytest") || has("pytest.ini") || has("conftest.py") || has("tests/conftest.py");
        let run = RunFile {
            test: vec![if pytest {
                format!("{prefix}pytest")
            } else {
                format!("{prefix}python3 -m unittest")
            }],
            check: if tool("pyright") || has("pyrightconfig.json") {
                Some(format!("{prefix}pyright"))
            } else if tool("mypy") || has("mypy.ini") {
                Some(format!("{prefix}mypy ."))
            } else {
                None
            },
            lint: (tool("ruff") || has("ruff.toml")).then(|| format!("{prefix}ruff check .")),
            format: (tool("ruff") || has("ruff.toml")).then(|| format!("{prefix}ruff format .")),
            ..RunFile::default()
        };
        let source = if has("pyproject.toml") {
            "pyproject.toml"
        } else if has("setup.py") {
            "setup.py"
        } else if has("pytest.ini") {
            "pytest.ini"
        } else {
            "setup.cfg"
        };
        return Some((run, source));
    }
    // In the order GNU make reads them, by the exact name: on a
    // case-insensitive filesystem `exists("makefile")` is true of `Makefile`.
    let entries: Vec<String> = std::fs::read_dir(root)
        .map(|d| {
            d.flatten()
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if let Some(name) = ["GNUmakefile", "makefile", "Makefile"]
        .into_iter()
        .find(|n| entries.iter().any(|e| e == n))
    {
        let makefile = read(name);
        let target = |name: &str| {
            makefile
                .lines()
                .any(|l| l.starts_with(&format!("{name}:")) && !l.starts_with(&format!("{name}:=")))
                .then(|| format!("make {name}"))
        };
        let run = RunFile {
            build: target("build"),
            test: target("test").into_iter().collect(),
            check: target("check"),
            lint: target("lint"),
            format: target("format").or_else(|| target("fmt")),
            ..RunFile::default()
        };
        return (!run.is_empty()).then_some((run, name));
    }
    None
}

/// How a package manager runs a program from the project's dependencies.
fn exec_of(runner: &str) -> &'static str {
    match runner {
        "pnpm" => "pnpm exec",
        "yarn" => "yarn",
        "bun" => "bunx",
        _ => "npx",
    }
}

/// The project's files as they stand, by path: a digest of each one's
/// content. Taken before and after `format` runs, to say which files it
/// changed; a formatter that rewrites a file as it was changed nothing.
pub type Snapshot = BTreeMap<PathBuf, [u8; 32]>;

/// The largest file read whole for its digest; a larger one is known by
/// its size and mtime.
const DIGEST_BYTES: u64 = 1 << 20;

/// The files under `root` the ignore files keep, each with its digest,
/// capped.
pub fn snapshot(root: &Path) -> Snapshot {
    use sha2::{Digest, Sha256};
    let mut out = BTreeMap::new();
    for entry in walk(root).take(MAX_WALK).flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let digest: [u8; 32] = if meta.len() <= DIGEST_BYTES {
            match std::fs::read(entry.path()) {
                Ok(bytes) => Sha256::digest(&bytes).into(),
                Err(_) => continue,
            }
        } else {
            let mut h = Sha256::new();
            h.update(meta.len().to_le_bytes());
            h.update(format!("{:?}", meta.modified().ok()).as_bytes());
            h.finalize().into()
        };
        out.insert(rel.to_path_buf(), digest);
    }
    out
}

/// The files `after` has that `before` did not, or not as they are now.
pub fn changed(before: &Snapshot, after: &Snapshot) -> Vec<String> {
    after
        .iter()
        .filter(|(path, stamp)| before.get(*path) != Some(stamp))
        .map(|(path, _)| path.display().to_string())
        .collect()
}

/// The project's files, the ignore files honoured and the build and
/// dependency folders skipped.
fn walk(root: &Path) -> ignore::Walk {
    let mut walker = ignore::WalkBuilder::new(root);
    walker
        .hidden(true)
        .follow_links(false)
        .git_ignore(true)
        .git_exclude(true)
        .require_git(false)
        .filter_entry(|e| {
            !(e.file_type().is_some_and(|t| t.is_dir())
                && e.file_name()
                    .to_str()
                    .is_some_and(|n| crate::map::SKIP_DIRS.contains(&n)))
        });
    walker.build()
}

/// Every test file in the project (Python and script), relative to `root`:
/// the ignore files honoured, the build and dependency folders skipped,
/// capped.
fn test_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in walk(root).take(MAX_WALK).flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        if is_python_test(rel) || is_script_test(rel) {
            out.push(rel.to_path_buf());
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn narrowed(scope: Scope) -> Narrowed {
        match scope {
            Scope::Narrowed(n) => n,
            Scope::Cannot(why) => panic!("cannot: {why}"),
        }
    }

    fn cannot(scope: Scope) -> String {
        match scope {
            Scope::Cannot(why) => why,
            Scope::Narrowed(n) => panic!("narrowed: {}", n.cmd),
        }
    }

    fn files(root: &Path, list: &[(&str, &str)]) {
        for (p, text) in list {
            let path = root.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn nothing_asked_is_the_command_as_it_stands() {
        let root = tempfile::tempdir().unwrap();
        let n = narrowed(narrow("make test", None, &[], root.path()));
        assert_eq!(n.cmd, "make test");
        let n = narrowed(narrow("make test", Some("  "), &[], root.path()));
        assert_eq!(n.cmd, "make test");
    }

    #[test]
    fn cargo_filter_goes_before_the_double_dash() {
        let root = tempfile::tempdir().unwrap();
        let n = narrowed(narrow(
            "cargo test --workspace -- --nocapture",
            Some("newest"),
            &[],
            root.path(),
        ));
        assert_eq!(n.cmd, "cargo test --workspace newest -- --nocapture");
        let n = narrowed(narrow("cargo test", Some("a::b"), &[], root.path()));
        assert_eq!(n.cmd, "cargo test a::b");
        let n = narrowed(narrow(
            "cargo nextest run",
            Some("decisions::"),
            &[],
            root.path(),
        ));
        assert_eq!(n.cmd, "cargo nextest run decisions::");
    }

    #[test]
    fn cargo_files_become_their_packages_and_workspace_gives_way() {
        let root = tempfile::tempdir().unwrap();
        files(
            root.path(),
            &[
                ("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n"),
                (
                    "crates/core/Cargo.toml",
                    "[package]\nname = \"ryter-core\"\n",
                ),
                ("crates/core/src/lib.rs", ""),
                ("crates/core/src/map.rs", ""),
                ("crates/tui/Cargo.toml", "[package]\nname = \"ryter-tui\"\n"),
                ("crates/tui/src/lib.rs", ""),
                ("README.md", ""),
            ],
        );
        let n = narrowed(narrow(
            "cargo test --workspace",
            None,
            &strings(&[
                "crates/core/src/map.rs",
                "crates/core/src/lib.rs",
                "README.md",
            ]),
            root.path(),
        ));
        assert_eq!(n.cmd, "cargo test -p ryter-core");
        assert_eq!(n.unmatched, ["`README.md` is in no Cargo package"]);
        let n = narrowed(narrow(
            "cargo test --workspace -- --test-threads=1",
            Some("map::"),
            &strings(&["crates/tui/src/lib.rs", "crates/core/src/map.rs"]),
            root.path(),
        ));
        assert_eq!(
            n.cmd,
            "cargo test -p ryter-core -p ryter-tui map:: -- --test-threads=1"
        );
        // Only a file in no package: not narrowed, said.
        let why = cannot(narrow(
            "cargo test",
            None,
            &strings(&["README.md"]),
            root.path(),
        ));
        assert_eq!(why, "cargo test: `README.md` is in no Cargo package");
    }

    #[test]
    fn pytest_takes_k_and_test_files_by_name() {
        let root = tempfile::tempdir().unwrap();
        files(
            root.path(),
            &[
                ("pkg/mod.py", ""),
                ("pkg/other.py", ""),
                ("tests/test_mod.py", ""),
                ("tests/test_api.py", ""),
            ],
        );
        let n = narrowed(narrow("pytest -q", Some("newest"), &[], root.path()));
        assert_eq!(n.cmd, "pytest -q -k newest");
        let n = narrowed(narrow(
            "pytest",
            Some("a and not b"),
            &strings(&["pkg/mod.py", "tests/test_api.py", "pkg/other.py"]),
            root.path(),
        ));
        assert_eq!(
            n.cmd,
            "pytest -k 'a and not b' tests/test_api.py tests/test_mod.py"
        );
        assert_eq!(n.unmatched, ["no test file for `pkg/other.py`"]);
        let why = cannot(narrow(
            "pytest",
            None,
            &strings(&["pkg/other.py"]),
            root.path(),
        ));
        assert_eq!(why, "pytest: no test file for `pkg/other.py`");
    }

    #[test]
    fn a_source_matches_the_test_named_for_its_whole_stem_and_beside_it_first() {
        let root = tempfile::tempdir().unwrap();
        files(
            root.path(),
            &[
                ("src/button.ts", ""),
                ("src/button.test.ts", ""),
                ("src/button.component.ts", ""),
                ("src/button.component.spec.ts", ""),
                ("src/api/index.ts", ""),
                ("src/api/index.test.ts", ""),
                ("src/web/index.test.ts", ""),
                ("src/lib/util.ts", ""),
                ("test/util.test.ts", ""),
                ("test/other.test.ts", ""),
            ],
        );
        let run =
            |files: &[&str]| narrowed(narrow("npx vitest", None, &strings(files), root.path())).cmd;
        // The whole stem, not the text before the first dot.
        assert_eq!(run(&["src/button.ts"]), "npx vitest src/button.test.ts");
        assert_eq!(
            run(&["src/button.component.ts"]),
            "npx vitest src/button.component.spec.ts"
        );
        // Beside the source first; elsewhere only when nothing is beside.
        assert_eq!(
            run(&["src/api/index.ts"]),
            "npx vitest src/api/index.test.ts"
        );
        assert_eq!(run(&["src/lib/util.ts"]), "npx vitest test/util.test.ts");
        // Python the same way.
        let py = tempfile::tempdir().unwrap();
        files(
            py.path(),
            &[
                ("pkg/mod.py", ""),
                ("pkg/test_mod.py", ""),
                ("tests/test_mod.py", ""),
                ("pkg/other.py", ""),
                ("tests/test_other.py", ""),
            ],
        );
        let n = narrowed(narrow(
            "pytest",
            None,
            &strings(&["pkg/mod.py", "pkg/other.py"]),
            py.path(),
        ));
        assert_eq!(n.cmd, "pytest pkg/test_mod.py tests/test_other.py");
    }

    #[test]
    fn a_commands_own_test_paths_give_way_to_the_files() {
        let root = tempfile::tempdir().unwrap();
        files(
            root.path(),
            &[
                ("pkg/mod.py", ""),
                ("tests/test_mod.py", ""),
                ("tests/test_other.py", ""),
                ("tests/unit/test_a.py", ""),
                ("src/sum.ts", ""),
                ("src/sum.test.ts", ""),
                ("test/old.js", ""),
                (
                    "package.json",
                    r#"{"scripts":{"test":"vitest run test/","unit":"vitest run","old":"mocha test/**/*.js"}}"#,
                ),
            ],
        );
        // A folder the command names is dropped; a flag's value is kept.
        let n = narrowed(narrow(
            "pytest -q tests",
            None,
            &strings(&["pkg/mod.py"]),
            root.path(),
        ));
        assert_eq!(n.cmd, "pytest -q tests/test_mod.py");
        let n = narrowed(narrow(
            "pytest -k slow tests -p no:cacheprovider",
            None,
            &strings(&["pkg/mod.py"]),
            root.path(),
        ));
        assert_eq!(
            n.cmd,
            "pytest -k slow -p no:cacheprovider tests/test_mod.py"
        );
        let n = narrowed(narrow(
            "pytest --rootdir tests tests/",
            Some("x"),
            &strings(&["pkg/mod.py"]),
            root.path(),
        ));
        assert_eq!(n.cmd, "pytest --rootdir tests -k x tests/test_mod.py");
        // A filter alone leaves the command's paths as they are.
        let n = narrowed(narrow("pytest tests", Some("x"), &[], root.path()));
        assert_eq!(n.cmd, "pytest tests -k x");
        // A path that is not a folder here (a file, a pattern) is refused.
        let why = cannot(narrow(
            "pytest tests/unit/test_a.py",
            None,
            &strings(&["pkg/mod.py"]),
            root.path(),
        ));
        assert!(
            why.contains("`tests/unit/test_a.py` in the command picks tests of its own"),
            "{why}"
        );
        let why = cannot(narrow(
            "npx mocha 'test/**/*.js'",
            None,
            &strings(&["test/old.js"]),
            root.path(),
        ));
        assert!(why.contains("picks tests of its own"), "{why}");
        // vitest's subcommand is not a path; its folder is dropped.
        let n = narrowed(narrow(
            "npx vitest run test",
            None,
            &strings(&["src/sum.ts"]),
            root.path(),
        ));
        assert_eq!(n.cmd, "npx vitest run src/sum.test.ts");
        // A script that names paths itself can't be narrowed by files, only
        // by a filter.
        let why = cannot(narrow(
            "npm test",
            None,
            &strings(&["src/sum.ts"]),
            root.path(),
        ));
        assert!(
            why.contains("the script it runs already names test paths"),
            "{why}"
        );
        let n = narrowed(narrow("npm test", Some("adds"), &[], root.path()));
        assert_eq!(n.cmd, "npm test -- -t adds");
        let n = narrowed(narrow(
            "npm run unit",
            None,
            &strings(&["src/sum.ts"]),
            root.path(),
        ));
        assert_eq!(n.cmd, "npm run unit -- src/sum.test.ts");
        assert!(
            cannot(narrow(
                "yarn run old",
                None,
                &strings(&["test/old.js"]),
                root.path()
            ))
            .contains("already names test paths")
        );
    }

    #[test]
    fn go_keeps_flag_values_and_replaces_every_package_word() {
        let root = tempfile::tempdir().unwrap();
        files(
            root.path(),
            &[("pkg/a.go", ""), ("pkg/a/a.go", ""), ("go.mod", "")],
        );
        let run = |cmd: &str, files: &[&str]| {
            narrowed(narrow(cmd, None, &strings(files), root.path())).cmd
        };
        assert_eq!(
            run("go test -coverprofile ./coverage.out ./...", &["pkg/a.go"]),
            "go test -coverprofile ./coverage.out ./pkg"
        );
        assert_eq!(
            run("go test -exec ./tools/wrap ./...", &["pkg/a.go"]),
            "go test -exec ./tools/wrap ./pkg"
        );
        assert_eq!(run("go test ...", &["pkg/a/a.go"]), "go test ./pkg/a");
        assert_eq!(run("go test all", &["pkg/a/a.go"]), "go test ./pkg/a");
        assert_eq!(run("go test ./...", &["pkg/a/a.go"]), "go test ./pkg/a");
        assert_eq!(
            run(
                "go test -v -count=1 -timeout 30s example.com/m/pkg",
                &["pkg/a.go"]
            ),
            "go test -v -count=1 -timeout 30s ./pkg"
        );
        assert_eq!(
            run("go test -race ./... -run Old", &["pkg/a.go"]),
            "go test -race -run Old ./pkg"
        );
        // A directory with a space is one word.
        let odd = tempfile::tempdir().unwrap();
        files(odd.path(), &[("my pkg/a.go", "")]);
        assert_eq!(
            narrowed(narrow(
                "go test ./...",
                None,
                &strings(&["my pkg/a.go"]),
                odd.path()
            ))
            .cmd,
            "go test './my pkg'"
        );
        // `-args` and everything after it belong to the test binary: the
        // packages and a `-run` go before it.
        assert_eq!(
            run("go test ./... -args -update -golden", &["pkg/a.go"]),
            "go test ./pkg -args -update -golden"
        );
        assert_eq!(
            narrowed(narrow(
                "go test ./... -args -update",
                Some("TestX"),
                &strings(&["pkg/a.go"]),
                root.path()
            ))
            .cmd,
            "go test -run TestX ./pkg -args -update"
        );
        // A quoted value is one word and comes back as it was written.
        assert_eq!(
            run("go test -ldflags \"-s -w\" ./...", &["pkg/a.go"]),
            "go test -ldflags \"-s -w\" ./pkg"
        );
        assert_eq!(
            run("go test -gcflags 'all=-N -l' ./...", &["pkg/a.go"]),
            "go test -gcflags 'all=-N -l' ./pkg"
        );
        // Boolean flags, a doubled dash and the `test.` prefix.
        for flag in [
            "-buildvcs",
            "-artifacts",
            "-modcacherw",
            "-paniconexit0",
            "--race",
            "-test.v",
            "--test.short",
        ] {
            assert_eq!(
                run(&format!("go test {flag} ./..."), &["pkg/a.go"]),
                format!("go test {flag} ./pkg"),
                "{flag}"
            );
        }
        // A flag Ryter does not know keeps a value that is not a package,
        // and drops a package that follows it.
        assert_eq!(
            run("go test -newflag value ./...", &["pkg/a.go"]),
            "go test -newflag value ./pkg"
        );
        assert_eq!(
            run("go test -newbool ./...", &["pkg/a.go"]),
            "go test -newbool ./pkg"
        );
        assert_eq!(
            run("go test -outputdir ./pkg ./...", &["pkg/a.go"]),
            "go test -outputdir ./pkg ./pkg"
        );
        // A flag's value spelt `-args` is the value, not the separator;
        // `-test.args` is a flag like any other.
        assert_eq!(
            run("go test -ldflags -args ./...", &["pkg/a.go"]),
            "go test -ldflags -args ./pkg"
        );
        assert_eq!(
            run("go test -ldflags \"-args\" ./...", &["pkg/a.go"]),
            "go test -ldflags \"-args\" ./pkg"
        );
        assert_eq!(
            run("go test ./... -ldflags -args", &["pkg/a.go"]),
            "go test -ldflags -args ./pkg"
        );
        assert_eq!(
            run("go test -test.args x ./...", &["pkg/a.go"]),
            "go test -test.args x ./pkg"
        );
        assert_eq!(
            run("go test -v -args -update ./pkg", &["pkg/a.go"]),
            "go test -v ./pkg -args -update ./pkg"
        );
    }

    #[test]
    fn expansions_and_assignments_are_kept_as_written() {
        let root = tempfile::tempdir().unwrap();
        files(
            root.path(),
            &[
                ("pkg/a.go", ""),
                ("go.mod", ""),
                ("pkg/mod.py", ""),
                ("pkg/test_mod.py", ""),
                ("tests/.keep", ""),
            ],
        );
        let run = |cmd: &str, files: &[&str]| {
            narrowed(narrow(cmd, None, &strings(files), root.path())).cmd
        };
        // A `$VAR`, a `$(…)` or a backtick the user approved still expands:
        // the word goes back on the line as it was written.
        for (cmd, want) in [
            (
                "go test -skip \"$SKIP\" ./...",
                "go test -skip \"$SKIP\" ./pkg",
            ),
            (
                "go test -ldflags \"-X main.version=$VERSION\" ./...",
                "go test -ldflags \"-X main.version=$VERSION\" ./pkg",
            ),
            ("go test -count $COUNT ./...", "go test -count $COUNT ./pkg"),
            (
                "go test -gcflags \"$(pwd)\" ./...",
                "go test -gcflags \"$(pwd)\" ./pkg",
            ),
            (
                "go test -tags `cat tags` ./...",
                "go test -tags `cat tags` ./pkg",
            ),
            (
                "go test -tags $(cat a b) ./...",
                "go test -tags $(cat a b) ./pkg",
            ),
            (
                "go test -tags \"$(echo \"a )\" b)\" ./...",
                "go test -tags \"$(echo \"a )\" b)\" ./pkg",
            ),
            (
                "go test -tags ${TAGS:-a b} ./...",
                "go test -tags ${TAGS:-a b} ./pkg",
            ),
            ("go test -tags a$ ./...", "go test -tags a$ ./pkg"),
            ("go test -run 'Test$' ./...", "go test -run 'Test$' ./pkg"),
            (
                "go test -ldflags -X\\ main.v=1 ./...",
                "go test -ldflags -X\\ main.v=1 ./pkg",
            ),
        ] {
            assert_eq!(run(cmd, &["pkg/a.go"]), want, "{cmd}");
        }
        // An assignment before the runner stays one, quoted as it was.
        assert_eq!(
            run("TAGS=\"$FOO\" go test ./...", &["pkg/a.go"]),
            "TAGS=\"$FOO\" go test ./pkg"
        );
        assert_eq!(
            run("LDFLAGS=\"-s -w\" go test ./...", &["pkg/a.go"]),
            "LDFLAGS=\"-s -w\" go test ./pkg"
        );
        assert_eq!(
            run("FOO=\"a b\" pytest tests", &["pkg/mod.py"]),
            "FOO=\"a b\" pytest pkg/test_mod.py"
        );
        // A filter alone, too.
        let n = narrowed(narrow(
            "go test -ldflags \"-X main.version=$VERSION\" ./...",
            Some("TestX"),
            &[],
            root.path(),
        ));
        assert_eq!(
            n.cmd,
            "go test -ldflags \"-X main.version=$VERSION\" ./... -run TestX"
        );
        assert_eq!(
            run("pytest -k \"$NAME\" tests", &["pkg/mod.py"]),
            "pytest -k \"$NAME\" pkg/test_mod.py"
        );
        // What Ryter adds is one shell word of its own.
        let n = narrowed(narrow(
            "go test ./...",
            Some("Test$"),
            &strings(&["pkg/a.go"]),
            root.path(),
        ));
        assert_eq!(n.cmd, "go test -run 'Test$' ./pkg");
        // An unclosed substitution is said, like an unclosed quote.
        for cmd in [
            "go test -tags $(cat ./...",
            "go test -tags `cat ./...",
            "go test -skip ${SKIP:-$(echo Test} ./...",
        ] {
            let why = cannot(narrow(cmd, None, &strings(&["pkg/a.go"]), root.path()));
            assert!(
                why.contains("unclosed quote or substitution"),
                "{cmd}: {why}"
            );
        }
        // A substitution inside another ends on its own mark.
        for (cmd, want) in [
            (
                "go test -skip ${SKIP:-$(echo Test} More)} ./...",
                "go test -skip ${SKIP:-$(echo Test} More)} ./pkg",
            ),
            (
                "go test -tags \"$(echo \"${X:-a b}\")\" ./...",
                "go test -tags \"$(echo \"${X:-a b}\")\" ./pkg",
            ),
            (
                "go test -tags `echo $(echo a b)` ./...",
                "go test -tags `echo $(echo a b)` ./pkg",
            ),
            (
                "go test -count $((1+2)) ./...",
                "go test -count $((1+2)) ./pkg",
            ),
            (
                "go test -count $((${X:-1}+2)) ./...",
                "go test -count $((${X:-1}+2)) ./pkg",
            ),
        ] {
            assert_eq!(run(cmd, &["pkg/a.go"]), want, "{cmd}");
        }
    }

    #[test]
    fn a_positional_word_the_shell_expands_is_refused() {
        let root = tempfile::tempdir().unwrap();
        files(
            root.path(),
            &[
                ("pkg/a.go", ""),
                ("go.mod", ""),
                ("pkg/mod.py", ""),
                ("pkg/test_mod.py", ""),
                ("tests/test_mod.py", ""),
                ("tests/test_other.py", ""),
                ("src/sum.ts", ""),
                ("src/sum.test.ts", ""),
            ],
        );
        // A test path Ryter can't see would widen the files' run.
        for cmd in [
            "pytest ${TEST_DIR:-tests}",
            "pytest \"$DIR\"",
            "pytest $(echo tests)",
            "pytest `echo tests`",
            "pytest {tests,other}",
            "pytest ~",
        ] {
            let why = cannot(narrow(cmd, None, &strings(&["pkg/mod.py"]), root.path()));
            assert!(why.contains("expanded by the shell"), "{cmd}: {why}");
        }
        let why = cannot(narrow(
            "npx vitest run ${DIR:-src}",
            None,
            &strings(&["src/sum.ts"]),
            root.path(),
        ));
        assert!(why.contains("expanded by the shell"), "{why}");
        // A Go word that may be flags as well as packages can't be dropped.
        for cmd in [
            "go test ${FLAGS:--race} ./...",
            "go test $(echo -race) ./...",
            "go test $(echo -race -count=1) ./...",
            "go test {-race,-count=1} ./...",
        ] {
            let why = cannot(narrow(cmd, None, &strings(&["pkg/a.go"]), root.path()));
            assert!(why.contains("expanded by the shell"), "{cmd}: {why}");
        }
        // A filter alone keeps every word as written; a flag's value is a
        // value; a literal folder still gives way.
        let n = narrowed(narrow(
            "go test ${FLAGS:--race} ./...",
            Some("TestX"),
            &[],
            root.path(),
        ));
        assert_eq!(n.cmd, "go test ${FLAGS:--race} ./... -run TestX");
        let n = narrowed(narrow(
            "pytest ${TEST_DIR:-tests}",
            Some("x"),
            &[],
            root.path(),
        ));
        assert_eq!(n.cmd, "pytest ${TEST_DIR:-tests} -k x");
        let n = narrowed(narrow(
            "go test -tags ${TAGS:-a b} ./...",
            None,
            &strings(&["pkg/a.go"]),
            root.path(),
        ));
        assert_eq!(n.cmd, "go test -tags ${TAGS:-a b} ./pkg");
        let n = narrowed(narrow(
            "pytest -k \"$NAME\" tests",
            None,
            &strings(&["pkg/mod.py"]),
            root.path(),
        ));
        assert_eq!(n.cmd, "pytest -k \"$NAME\" pkg/test_mod.py");
    }

    #[test]
    fn quoted_folders_and_unknown_flags_in_the_command() {
        let root = tempfile::tempdir().unwrap();
        files(
            root.path(),
            &[
                ("pkg/mod.py", ""),
                ("tests/test_mod.py", ""),
                ("reports/.keep", ""),
                ("coverage/.keep", ""),
                ("src/sum.ts", ""),
                ("src/sum.test.ts", ""),
                (
                    "package.json",
                    r#"{"scripts":{"test":"vitest run \"test\""}}"#,
                ),
                ("test/.keep", ""),
            ],
        );
        // A quoted folder is still a folder.
        let n = narrowed(narrow(
            "pytest -q \"tests\"",
            None,
            &strings(&["pkg/mod.py"]),
            root.path(),
        ));
        assert_eq!(n.cmd, "pytest -q tests/test_mod.py");
        let n = narrowed(narrow(
            "npx vitest run 'test'",
            None,
            &strings(&["src/sum.ts"]),
            root.path(),
        ));
        assert_eq!(n.cmd, "npx vitest run src/sum.test.ts");
        let why = cannot(narrow(
            "npm test",
            None,
            &strings(&["src/sum.ts"]),
            root.path(),
        ));
        assert!(why.contains("already names test paths"), "{why}");
        // Other quoted words keep their meaning.
        let n = narrowed(narrow(
            "pytest -k 'a and b' tests",
            None,
            &strings(&["pkg/mod.py"]),
            root.path(),
        ));
        assert_eq!(n.cmd, "pytest -k 'a and b' tests/test_mod.py");
        assert!(
            cannot(narrow(
                "pytest 'tests",
                None,
                &strings(&["pkg/mod.py"]),
                root.path()
            ))
            .contains("unclosed quote")
        );
        // A flag Ryter does not know, followed by a folder: refused, not
        // dropped; a known boolean flag before a folder is not a value.
        let why = cannot(narrow(
            "pytest --html reports",
            None,
            &strings(&["pkg/mod.py"]),
            root.path(),
        ));
        assert!(why.contains("`--html reports` in the command"), "{why}");
        let why = cannot(narrow(
            "npx vitest run --coverage.reportsDirectory ./coverage",
            None,
            &strings(&["src/sum.ts"]),
            root.path(),
        ));
        assert!(
            why.contains("--coverage.reportsDirectory ./coverage"),
            "{why}"
        );
        let n = narrowed(narrow(
            "pytest --html=reports tests",
            None,
            &strings(&["pkg/mod.py"]),
            root.path(),
        ));
        assert_eq!(n.cmd, "pytest --html=reports tests/test_mod.py");
        let n = narrowed(narrow(
            "pytest -x tests",
            None,
            &strings(&["pkg/mod.py"]),
            root.path(),
        ));
        assert_eq!(n.cmd, "pytest -x tests/test_mod.py");
        // A filter alone touches none of it.
        let n = narrowed(narrow("pytest --html reports", Some("x"), &[], root.path()));
        assert_eq!(n.cmd, "pytest --html reports -k x");
    }

    #[test]
    fn wrappers_reach_the_runner() {
        let root = tempfile::tempdir().unwrap();
        for cmd in [
            "uv run pytest",
            "poetry run pytest -x",
            "python -m pytest",
            "python3.12 -m pytest",
            ".venv/bin/pytest",
            "PYTHONPATH=src pytest",
        ] {
            let n = narrowed(narrow(cmd, Some("x"), &[], root.path()));
            assert_eq!(n.cmd, format!("{cmd} -k x"), "{cmd}");
        }
    }

    #[test]
    fn go_takes_run_and_the_files_directories() {
        let root = tempfile::tempdir().unwrap();
        files(
            root.path(),
            &[
                ("pkg/a/a.go", ""),
                ("pkg/a/a_test.go", ""),
                ("pkg/b/b.go", ""),
                ("main.go", ""),
                ("go.mod", ""),
            ],
        );
        let n = narrowed(narrow("go test ./...", Some("TestAdd"), &[], root.path()));
        assert_eq!(n.cmd, "go test ./... -run TestAdd");
        let n = narrowed(narrow(
            "go test -race ./...",
            None,
            &strings(&["pkg/a/a_test.go", "pkg/b/b.go", "main.go", "go.mod"]),
            root.path(),
        ));
        assert_eq!(n.cmd, "go test -race . ./pkg/a ./pkg/b");
        assert_eq!(n.unmatched, ["`go.mod` is not a Go file"]);
    }

    #[test]
    fn npm_scripts_are_read_and_arguments_pass_through() {
        let root = tempfile::tempdir().unwrap();
        files(
            root.path(),
            &[
                (
                    "package.json",
                    r#"{"scripts":{"test":"jest --ci","unit":"vitest run","old":"mocha test/"}}"#,
                ),
                ("src/sum.ts", ""),
                ("src/sum.test.ts", ""),
                ("src/lone.ts", ""),
                ("test/old.js", ""),
            ],
        );
        let n = narrowed(narrow(
            "npm test",
            Some("adds two"),
            &strings(&["src/sum.ts", "README.md"]),
            root.path(),
        ));
        assert_eq!(
            n.cmd,
            "npm test -- -t 'adds two' --findRelatedTests src/sum.ts"
        );
        assert_eq!(n.unmatched, ["`README.md` is not a file in the project"]);
        let n = narrowed(narrow(
            "pnpm run unit",
            None,
            &strings(&["src/sum.ts", "src/lone.ts"]),
            root.path(),
        ));
        assert_eq!(n.cmd, "pnpm run unit -- src/sum.test.ts");
        assert_eq!(n.unmatched, ["no test file for `src/lone.ts`"]);
        let n = narrowed(narrow("yarn run old", Some("x"), &[], root.path()));
        assert_eq!(n.cmd, "yarn run old --grep x");
        let n = narrowed(narrow(
            "npx vitest",
            Some("x"),
            &strings(&["test/old.js"]),
            root.path(),
        ));
        assert_eq!(n.cmd, "npx vitest -t x test/old.js");
        let n = narrowed(narrow("bun test", Some("x"), &[], root.path()));
        assert_eq!(n.cmd, "bun test -t x");
    }

    #[test]
    fn what_cannot_be_narrowed_is_said() {
        let root = tempfile::tempdir().unwrap();
        files(
            root.path(),
            &[("package.json", r#"{"scripts":{"test":"node run.js"}}"#)],
        );
        assert!(cannot(narrow("make test", Some("x"), &[], root.path())).contains("not a runner"));
        assert!(cannot(narrow("npm test", Some("x"), &[], root.path())).contains("not a runner"));
        assert!(
            cannot(narrow(
                "cargo build && cargo test",
                Some("x"),
                &[],
                root.path()
            ))
            .contains("several commands")
        );
        assert!(
            cannot(narrow("pytest | tee log", Some("x"), &[], root.path()))
                .contains("several commands")
        );
        // A pipe inside quotes is not a second command.
        let n = narrowed(narrow("pytest -k 'a|b'", Some("x"), &[], root.path()));
        assert_eq!(n.cmd, "pytest -k 'a|b' -k x");
    }

    #[test]
    fn paths_outside_the_project_are_said() {
        let root = tempfile::tempdir().unwrap();
        files(root.path(), &[("tests/test_a.py", "")]);
        let n = narrowed(narrow(
            "pytest",
            None,
            &strings(&[
                "tests/test_a.py",
                "../other/test_b.py",
                "/etc/passwd",
                &root.path().join("tests/test_a.py").display().to_string(),
            ]),
            root.path(),
        ));
        assert_eq!(n.cmd, "pytest tests/test_a.py");
        assert_eq!(
            n.unmatched,
            [
                "`../other/test_b.py` is outside the project",
                "`/etc/passwd` is outside the project"
            ]
        );
    }

    #[test]
    fn a_filter_is_one_shell_word() {
        assert_eq!(quote("tests::a"), "tests::a");
        assert_eq!(quote("a b"), "'a b'");
        assert_eq!(quote("it's"), "'it'\\''s'");
        assert_eq!(quote("$(x)"), "'$(x)'");
    }

    #[test]
    fn verbs_are_read_from_the_manifest_at_the_root() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(detect(root.path()), None);
        // Cargo, a single package.
        files(root.path(), &[("Cargo.toml", "[package]\nname = \"x\"\n")]);
        let (run, from) = detect(root.path()).unwrap();
        assert_eq!(from, "Cargo.toml");
        assert_eq!(run.test, ["cargo test"]);
        assert_eq!(run.build.as_deref(), Some("cargo build"));
        assert_eq!(run.check.as_deref(), Some("cargo check"));
        assert_eq!(run.lint.as_deref(), Some("cargo clippy --all-targets"));
        assert_eq!(run.format.as_deref(), Some("cargo fmt"));
        assert_eq!((run.start, run.ready, run.stop), (None, None, None));
        // A workspace.
        files(
            root.path(),
            &[("Cargo.toml", "[workspace]\nmembers = [\"a\"]\n")],
        );
        let (run, _) = detect(root.path()).unwrap();
        assert_eq!(run.test, ["cargo test --workspace"]);
        assert_eq!(run.format.as_deref(), Some("cargo fmt --all"));
    }

    #[test]
    fn node_verbs_follow_the_scripts_and_the_lockfile() {
        let root = tempfile::tempdir().unwrap();
        files(
            root.path(),
            &[(
                "package.json",
                r#"{"scripts":{"test":"vitest run","build":"tsc -p .","lint":"eslint .","fmt":"prettier -w ."}}"#,
            )],
        );
        let (run, from) = detect(root.path()).unwrap();
        assert_eq!(from, "package.json");
        assert_eq!(run.test, ["npm test"]);
        assert_eq!(run.build.as_deref(), Some("npm run build"));
        assert_eq!(run.lint.as_deref(), Some("npm run lint"));
        assert_eq!(run.format.as_deref(), Some("npm run fmt"));
        assert_eq!(run.check, None);
        files(
            root.path(),
            &[("pnpm-lock.yaml", ""), ("tsconfig.json", "{}")],
        );
        let (run, _) = detect(root.path()).unwrap();
        assert_eq!(run.test, ["pnpm test"]);
        assert_eq!(run.check.as_deref(), Some("pnpm exec tsc --noEmit"));
        // No scripts at all: nothing to approve.
        let empty = tempfile::tempdir().unwrap();
        files(empty.path(), &[("package.json", r#"{"name":"x"}"#)]);
        assert_eq!(detect(empty.path()), None);
    }

    #[test]
    fn python_go_and_make_verbs() {
        let root = tempfile::tempdir().unwrap();
        files(
            root.path(),
            &[
                (
                    "pyproject.toml",
                    "[tool.pytest.ini_options]\n[tool.ruff]\n[tool.pyright]\n",
                ),
                ("uv.lock", ""),
            ],
        );
        let (run, from) = detect(root.path()).unwrap();
        assert_eq!(from, "pyproject.toml");
        assert_eq!(run.test, ["uv run pytest"]);
        assert_eq!(run.check.as_deref(), Some("uv run pyright"));
        assert_eq!(run.lint.as_deref(), Some("uv run ruff check ."));
        assert_eq!(run.format.as_deref(), Some("uv run ruff format ."));
        let plain = tempfile::tempdir().unwrap();
        files(plain.path(), &[("setup.py", "")]);
        let (run, _) = detect(plain.path()).unwrap();
        assert_eq!(run.test, ["python3 -m unittest"]);
        assert_eq!(run.lint, None);
        let go = tempfile::tempdir().unwrap();
        files(go.path(), &[("go.mod", "module x\n")]);
        let (run, from) = detect(go.path()).unwrap();
        assert_eq!(from, "go.mod");
        assert_eq!(run.test, ["go test ./..."]);
        assert_eq!(run.check.as_deref(), Some("go vet ./..."));
        let make = tempfile::tempdir().unwrap();
        files(
            make.path(),
            &[(
                "Makefile",
                "build:\n\tcc x.c\n\ntest: build\n\t./t\n\nfmt:\n\tclang-format -i *.c\n",
            )],
        );
        let (run, from) = detect(make.path()).unwrap();
        assert_eq!(from, "Makefile");
        assert_eq!(run.build.as_deref(), Some("make build"));
        assert_eq!(run.test, ["make test"]);
        assert_eq!(run.format.as_deref(), Some("make fmt"));
        assert_eq!(run.lint, None);
        let lower = tempfile::tempdir().unwrap();
        files(lower.path(), &[("GNUmakefile", "test:\n\t./t\n")]);
        let (run, from) = detect(lower.path()).unwrap();
        assert_eq!(
            (from, run.test),
            ("GNUmakefile", vec!["make test".to_string()])
        );
        // With both, the one make reads.
        files(lower.path(), &[("Makefile", "check:\n\t./c\n")]);
        let (run, from) = detect(lower.path()).unwrap();
        assert_eq!(
            (from, run.test, run.check),
            ("GNUmakefile", vec!["make test".to_string()], None)
        );
    }

    #[test]
    fn a_formatters_changes_are_read_from_two_snapshots() {
        let root = tempfile::tempdir().unwrap();
        files(
            root.path(),
            &[
                ("src/a.rs", "fn a(){}\n"),
                ("src/b.rs", "fn b() {}\n"),
                ("target/x", "built"),
                (".gitignore", "target/\n"),
            ],
        );
        let before = snapshot(root.path());
        assert!(before.contains_key(Path::new("src/a.rs")));
        assert!(!before.contains_key(Path::new("target/x")));
        assert!(changed(&before, &snapshot(root.path())).is_empty());
        // A formatter rewrites one file and adds another; one it wrote
        // back as it was is not changed, whatever its mtime says.
        std::fs::write(root.path().join("src/a.rs"), "fn a() {}\n").unwrap();
        std::fs::write(root.path().join("src/b.rs"), "fn b() {}\n").unwrap();
        std::fs::write(root.path().join("src/c.rs"), "").unwrap();
        let after = snapshot(root.path());
        assert_eq!(changed(&before, &after), ["src/a.rs", "src/c.rs"]);
    }

    #[test]
    fn a_run_of_no_test_is_read() {
        assert!(ran_nothing(
            "running 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 81 filtered out\n"
        ));
        assert!(!ran_nothing(
            "test result: ok. 0 passed; 0 failed; 0 ignored\ntest result: ok. 2 passed; 0 failed; 0 ignored\n"
        ));
        assert!(ran_nothing("===== no tests ran in 0.01s ====="));
        assert!(ran_nothing("===== no tests ran in 0.01s =====\n[exit 5]"));
        assert!(ran_nothing("===== 12 deselected in 0.01s ====="));
        assert!(!ran_nothing("===== 1 passed, 12 deselected in 0.01s ====="));
        assert!(!ran_nothing("===== 1 error, 2 deselected in 0.05s ====="));
        assert!(!ran_nothing("===== 2 errors in 0.05s ====="));
        assert!(ran_nothing("ok  \tpkg/a\t0.002s [no tests to run]"));
        assert!(!ran_nothing("ok  \tpkg/a\t0.002s"));
        assert!(ran_nothing("No tests found, exiting with code 0"));
        assert!(ran_nothing("No test files found, exiting with code 1"));
        assert!(ran_nothing("  0 passing (2ms)"));
        assert!(!ran_nothing("  3 passing (2ms)"));
        assert!(!ran_nothing(
            "  0 passing (12ms)\n  1 failing\n\n  1) adds up:\n     AssertionError"
        ));
    }
}
