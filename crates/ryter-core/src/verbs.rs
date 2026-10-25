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
use std::path::{Component, Path, PathBuf};

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
        Runner::Cargo => cargo(cmd, filter, &matched, root, &mut unmatched),
        Runner::Pytest => pytest(cmd, &found, filter, &matched, root, &mut unmatched),
        Runner::Go => go(cmd, filter, &matched, &mut unmatched),
        Runner::Jest => jest(cmd, &found, filter, &matched, &mut unmatched),
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
        if last.contains("deselected") && !last.contains("passed") && !last.contains("failed") {
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
    // jest, vitest, mocha.
    lines.iter().any(|l| {
        l.starts_with("No tests found")
            || l.starts_with("No test files found")
            || (*l == "0 passing" || l.starts_with("0 passing ("))
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
    let words: Vec<&str> = cmd
        .split_whitespace()
        .skip_while(|w| is_assignment(w))
        .collect();
    recognise_words(&words, root, 0)
}

fn recognise_words(words: &[&str], root: &Path, depth: u8) -> Option<Recognised> {
    let (first, rest) = words.split_first()?;
    let first = first.rsplit('/').next().unwrap_or(first);
    let direct = |runner| Some(Recognised { runner, joint: " " });
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
    let words: Vec<&str> = line
        .split_whitespace()
        .skip_while(|w| is_assignment(w))
        .collect();
    let inner = recognise_words(&words, root, depth + 1)?;
    Some(Recognised {
        runner: inner.runner,
        joint,
    })
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
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '@'))
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
) -> Option<String> {
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
        let stem = f.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("");
        let mut hits: Vec<String> = Vec::new();
        if ext == "py" && !stem.is_empty() {
            let wants = [format!("test_{stem}.py"), format!("{stem}_test.py")];
            hits = index
                .iter()
                .filter(|t| {
                    t.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| wants.iter().any(|w| w == n))
                })
                .map(|t| quote(&t.display().to_string()))
                .collect();
        }
        if hits.is_empty() {
            unmatched.push(format!("no test file for `{}`", f.display()));
        } else {
            paths.extend(hits);
        }
    }
    if paths.is_empty() && !files.is_empty() {
        return None;
    }
    paths.sort();
    paths.dedup();
    let mut args = Vec::new();
    if let Some(f) = filter {
        args.push("-k".to_string());
        args.push(quote(f));
    }
    args.extend(paths);
    Some(append(cmd, found, &args))
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
    unmatched: &mut Vec<String>,
) -> Option<String> {
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
        return None;
    }
    let words: Vec<&str> = cmd.split_whitespace().collect();
    let mut out: Vec<String> = Vec::new();
    for w in &words {
        // A package pattern the command names (`./...`, `./pkg`, `.`)
        // gives way to the files' own.
        if !dirs.is_empty() && (w.starts_with("./") || *w == "." || w.ends_with("/...")) {
            continue;
        }
        out.push((*w).to_string());
    }
    if let Some(f) = filter {
        out.push("-run".into());
        out.push(quote(f));
    }
    out.extend(dirs);
    Some(out.join(" "))
}

/// `jest`: `-t` for the filter, `--findRelatedTests` for the files, which
/// jest maps to their tests itself (source or test).
fn jest(
    cmd: &str,
    found: &Recognised,
    filter: Option<&str>,
    files: &[PathBuf],
    unmatched: &mut Vec<String>,
) -> Option<String> {
    let mut related = Vec::new();
    for f in files {
        if is_script(f) {
            related.push(quote(&f.display().to_string()));
        } else {
            unmatched.push(format!("`{}` is not a script jest runs", f.display()));
        }
    }
    if related.is_empty() && !files.is_empty() {
        return None;
    }
    let mut args = Vec::new();
    if let Some(f) = filter {
        args.push("-t".to_string());
        args.push(quote(f));
    }
    if !related.is_empty() {
        args.push("--findRelatedTests".into());
        args.extend(related);
    }
    Some(append(cmd, found, &args))
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
) -> Option<String> {
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
        let stem = f.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let mut hits: Vec<String> = Vec::new();
        if is_script(f) && !stem.is_empty() {
            hits = index
                .iter()
                .filter(|t| {
                    let name = t.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    let base = name.split('.').next().unwrap_or("");
                    base == stem
                })
                .map(|t| quote(&t.display().to_string()))
                .collect();
        }
        if hits.is_empty() {
            unmatched.push(format!("no test file for `{}`", f.display()));
        } else {
            paths.extend(hits);
        }
    }
    if paths.is_empty() && !files.is_empty() {
        return None;
    }
    paths.sort();
    paths.dedup();
    let mut args = Vec::new();
    if let Some(f) = filter {
        args.push(flag.to_string());
        args.push(quote(f));
    }
    args.extend(paths);
    Some(append(cmd, found, &args))
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
        assert!(ran_nothing("ok  \tpkg/a\t0.002s [no tests to run]"));
        assert!(!ran_nothing("ok  \tpkg/a\t0.002s"));
        assert!(ran_nothing("No tests found, exiting with code 0"));
        assert!(ran_nothing("No test files found, exiting with code 1"));
        assert!(ran_nothing("  0 passing (2ms)"));
        assert!(!ran_nothing("  3 passing (2ms)"));
    }
}
