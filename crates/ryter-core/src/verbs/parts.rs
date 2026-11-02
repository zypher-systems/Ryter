//! The parts of a project: every manifest that says how some of it builds
//! and tests, at the top and in the folders under it. A Rust backend with
//! a frontend is two parts, and a suite that runs one of them is half a
//! suite. Discovery proposes commands for all of them; what is approved
//! stays the user's, and the parts are then read against it, so a run file
//! that leaves one out says so instead of passing for the whole project.

use std::path::Path;

use super::{expands, is_assignment, quote, shell_words};
use crate::run::RunFile;

/// How many folders under the top are looked in.
const MAX_DEPTH: usize = 3;

/// The most parts proposed in one run file; the rest are named as left
/// out. A card of forty packages is not one anybody reads.
const MAX_PARTS: usize = 8;

/// The most parts named in one line of what a suite leaves out.
const MAX_NAMED: usize = 8;

/// Folders whose manifests aren't the project's own: what its tests and
/// benchmarks use and its samples, next to the build and dependency
/// folders.
const NOT_PARTS: &[&str] = &[
    "__tests__",
    "bench",
    "benches",
    "benchmark",
    "benchmarks",
    "example",
    "examples",
    "fixture",
    "fixtures",
    "sample",
    "samples",
    "template",
    "templates",
    "test",
    "testdata",
    "tests",
    "third-party",
    "third_party",
];

/// What kind of manifest a part has, and so which programs run it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ecosystem {
    Cargo,
    Node,
    Go,
    Python,
    Make,
}

/// One part of a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// Its folder, relative to the project's top; empty for the top.
    dir: String,
    /// The manifest's file name.
    manifest: &'static str,
    ecosystem: Ecosystem,
    /// Its commands, as run from its own folder.
    run: RunFile,
    /// Its test command is a default nothing in the project asked for.
    assumed_test: bool,
}

impl Part {
    /// The manifest's path from the project's top, which is what the part
    /// is called on the card and in a suite's report.
    fn name(&self) -> String {
        if self.dir.is_empty() {
            self.manifest.to_string()
        } else {
            format!("{}/{}", self.dir, self.manifest)
        }
    }

    /// `cmd` as run from the project's top.
    fn run_from_top(&self, cmd: &str) -> String {
        if self.dir.is_empty() {
            cmd.to_string()
        } else {
            format!("cd {} && {cmd}", quote(&self.dir))
        }
    }
}

/// Whether `name` is in `dir` or a folder above it, up to the top: a
/// workspace keeps its lockfile at its root.
fn near(root: &Path, dir: &str, name: &str) -> bool {
    Path::new(dir)
        .ancestors()
        .any(|d| root.join(d).join(name).exists())
}

fn read(here: &Path, name: &str) -> String {
    std::fs::read_to_string(here.join(name)).unwrap_or_default()
}

fn cargo(root: &Path, dir: &str) -> Option<Part> {
    let here = root.join(dir);
    let manifest = std::fs::read_to_string(here.join("Cargo.toml")).ok()?;
    let workspace = manifest
        .parse::<toml::Value>()
        .ok()
        .is_some_and(|v| v.get("workspace").is_some());
    let scope = if workspace { " --workspace" } else { "" };
    Some(Part {
        dir: dir.to_string(),
        manifest: "Cargo.toml",
        ecosystem: Ecosystem::Cargo,
        run: RunFile {
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
        },
        assumed_test: false,
    })
}

fn node(root: &Path, dir: &str) -> Option<Part> {
    let here = root.join(dir);
    let json: serde_json::Value = serde_json::from_str(&read(&here, "package.json")).ok()?;
    let scripts = json.get("scripts").and_then(|s| s.as_object());
    let script = |name: &str| {
        let text = scripts.and_then(|s| s.get(name)).and_then(|s| s.as_str());
        // `npm init`'s placeholder fails on purpose: it is no test.
        text.is_some_and(|t| name != "test" || !t.contains("no test specified"))
    };
    let runner = if near(root, dir, "pnpm-lock.yaml") {
        "pnpm"
    } else if near(root, dir, "yarn.lock") {
        "yarn"
    } else if near(root, dir, "bun.lockb") || near(root, dir, "bun.lock") {
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
            here.join("tsconfig.json")
                .exists()
                .then(|| format!("{} tsc --noEmit", exec_of(runner)))
        }),
        lint: first(&["lint"]),
        format: first(&["format", "fmt"]),
        ..RunFile::default()
    };
    Some(Part {
        dir: dir.to_string(),
        manifest: "package.json",
        ecosystem: Ecosystem::Node,
        run,
        assumed_test: false,
    })
}

fn go(root: &Path, dir: &str) -> Option<Part> {
    root.join(dir).join("go.mod").exists().then(|| Part {
        dir: dir.to_string(),
        manifest: "go.mod",
        ecosystem: Ecosystem::Go,
        run: RunFile {
            build: Some("go build ./...".into()),
            test: vec!["go test ./...".into()],
            check: Some("go vet ./...".into()),
            format: Some("gofmt -l -w .".into()),
            ..RunFile::default()
        },
        assumed_test: false,
    })
}

fn python(root: &Path, dir: &str) -> Option<Part> {
    let here = root.join(dir);
    let has = |name: &str| here.join(name).exists();
    let manifest = ["pyproject.toml", "setup.py", "pytest.ini", "setup.cfg"]
        .into_iter()
        .find(|n| has(n))?;
    let pyproject = read(&here, "pyproject.toml");
    let table = pyproject.parse::<toml::Value>().ok();
    let tool = |name: &str| {
        table
            .as_ref()
            .and_then(|t| t.get("tool"))
            .and_then(|t| t.get(name))
            .is_some()
    };
    let prefix = if near(root, dir, "uv.lock") {
        "uv run "
    } else if near(root, dir, "poetry.lock") {
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
    Some(Part {
        dir: dir.to_string(),
        manifest,
        ecosystem: Ecosystem::Python,
        run,
        assumed_test: !pytest && !has("tests") && !has("test"),
    })
}

/// In the order GNU make reads them, by the exact name: on a
/// case-insensitive filesystem `exists("makefile")` is true of `Makefile`.
fn make(root: &Path) -> Option<Part> {
    let entries: Vec<String> = std::fs::read_dir(root)
        .map(|d| {
            d.flatten()
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let manifest = ["GNUmakefile", "makefile", "Makefile"]
        .into_iter()
        .find(|n| entries.iter().any(|e| e == n))?;
    let makefile = read(root, manifest);
    let target = |name: &str| {
        makefile
            .lines()
            .any(|l| l.starts_with(&format!("{name}:")) && !l.starts_with(&format!("{name}:=")))
            .then(|| format!("make {name}"))
    };
    Some(Part {
        dir: String::new(),
        manifest,
        ecosystem: Ecosystem::Make,
        run: RunFile {
            build: target("build"),
            test: target("test").into_iter().collect(),
            check: target("check"),
            lint: target("lint"),
            format: target("format").or_else(|| target("fmt")),
            ..RunFile::default()
        },
        assumed_test: false,
    })
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

/// A folder's path as Ryter will write it after `cd`: what can't be
/// shown on the card as it is, or reads as an option, is no part.
fn usable(dir: &str) -> bool {
    !dir.starts_with('-') && !dir.chars().any(char::is_control)
}

/// The manifests under the top, shallowest first: a workspace is read
/// before its members. The ignore files are honoured, links not followed.
fn nested_manifests(root: &Path) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for entry in super::walker(root, NOT_PARTS)
        .max_depth(Some(MAX_DEPTH + 1))
        .build()
        .take(super::MAX_WALK)
        .flatten()
    {
        if entry.depth() < 2 || !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Some(name) = entry.file_name().to_str() else {
            continue;
        };
        if !matches!(
            name,
            "Cargo.toml" | "package.json" | "go.mod" | "pyproject.toml"
        ) {
            continue;
        }
        let Some(dir) = entry
            .path()
            .parent()
            .and_then(|p| p.strip_prefix(root).ok())
            .and_then(Path::to_str)
        else {
            continue;
        };
        if usable(dir) {
            found.push((dir.to_string(), name.to_string()));
        }
    }
    found.sort_by_key(|(dir, name)| (dir.matches('/').count(), dir.clone(), name.clone()));
    found
}

/// `dir` under `above`, as the path between them. The top is above
/// everything.
fn under<'a>(dir: &'a str, above: &str) -> Option<&'a str> {
    if above.is_empty() {
        return Some(dir);
    }
    dir.strip_prefix(above)?.strip_prefix('/')
}

/// Whether the Cargo workspace at `workspace` has the package in `dir` as
/// a member, so that its `--workspace` commands already run it. Cargo
/// refuses a package under a workspace that neither lists nor excludes
/// it, so one that isn't excluded and isn't a workspace itself is a member.
fn cargo_member(root: &Path, workspace: &str, dir: &str) -> bool {
    let Some(inside) = under(dir, workspace) else {
        return false;
    };
    let table = |folder: &str| {
        read(&root.join(folder), "Cargo.toml")
            .parse::<toml::Value>()
            .ok()
    };
    let Some(top) = table(workspace) else {
        return false;
    };
    let Some(table_of_workspace) = top.get("workspace") else {
        return false;
    };
    if table(dir).is_some_and(|t| t.get("workspace").is_some()) {
        return false;
    }
    let excluded = table_of_workspace
        .get("exclude")
        .and_then(|e| e.as_array())
        .is_some_and(|list| {
            list.iter()
                .filter_map(|e| e.as_str())
                .any(|e| under(inside, e.trim_end_matches('/')).is_some() || inside == e)
        });
    !excluded
}

/// The folders a Node workspace at `workspace` names, as written: from
/// `package.json`'s `workspaces`, or `pnpm-workspace.yaml`'s `packages`.
/// `None` when it isn't a workspace.
fn node_workspaces(root: &Path, workspace: &str) -> Option<Vec<String>> {
    let here = root.join(workspace);
    if here.join("pnpm-workspace.yaml").exists() {
        let text = read(&here, "pnpm-workspace.yaml");
        let mut in_packages = false;
        let mut globs = Vec::new();
        for line in text.lines() {
            let line = line.split(" #").next().unwrap_or(line);
            if line.trim_start().starts_with('#') || line.trim().is_empty() {
                continue;
            }
            if !line.starts_with([' ', '\t', '-']) {
                in_packages = line.trim_end() == "packages:";
            } else if let Some(item) = line.trim().strip_prefix("- ").filter(|_| in_packages) {
                globs.push(item.trim().trim_matches(['\'', '"']).to_string());
            }
        }
        if globs.is_empty() {
            globs.push("**".to_string());
        }
        return Some(globs);
    }
    let json: serde_json::Value = serde_json::from_str(&read(&here, "package.json")).ok()?;
    let spaces = json.get("workspaces")?;
    let list = spaces
        .as_array()
        .or_else(|| spaces.get("packages").and_then(|p| p.as_array()))?;
    Some(
        list.iter()
            .filter_map(|g| g.as_str())
            .map(str::to_string)
            .collect(),
    )
}

/// Whether the Node workspace at `workspace` has the package in `dir` as
/// a member, a `!` pattern having the last word on it as it does in
/// npm's and pnpm's own reading.
fn node_member(root: &Path, workspace: &str, dir: &str) -> bool {
    let Some(inside) = under(dir, workspace) else {
        return false;
    };
    let Some(globs) = node_workspaces(root, workspace) else {
        return false;
    };
    // `packages/*` is the folders in `packages`, not those below them.
    let options = glob::MatchOptions {
        require_literal_separator: true,
        ..glob::MatchOptions::new()
    };
    let matches = |glob: &str| {
        let glob = glob.trim_start_matches("./").trim_end_matches('/');
        glob::Pattern::new(glob).is_ok_and(|p| p.matches_with(inside, options))
    };
    let mut member = false;
    for glob in &globs {
        match glob.strip_prefix('!') {
            Some(not) if matches(not) => member = false,
            None if matches(glob) => member = true,
            _ => {}
        }
    }
    member
}

/// Every part of the project, the top's first. A part with nothing to run
/// is none, and a default test command only stands when nothing else in
/// the project says how it runs: `pyproject.toml` beside `package.json`
/// is usually a linter's settings, and `unittest` would fail for finding
/// no test.
pub fn parts(root: &Path) -> Vec<Part> {
    let mut found: Vec<Part> = [
        cargo(root, ""),
        node(root, ""),
        go(root, ""),
        python(root, ""),
    ]
    .into_iter()
    .flatten()
    .filter(|p| !p.run.is_empty())
    .collect();
    if found.is_empty() {
        found.extend(make(root).filter(|p| !p.run.is_empty()));
    }
    for (dir, name) in nested_manifests(root) {
        let part = match name.as_str() {
            "Cargo.toml" => {
                let member = found
                    .iter()
                    .any(|p| p.ecosystem == Ecosystem::Cargo && cargo_member(root, &p.dir, &dir));
                if member { None } else { cargo(root, &dir) }
            }
            "package.json" => {
                // Its root's tests run it: a workspace's root with no tests
                // of its own leaves each member to bring its own.
                let member = found.iter().any(|p| {
                    p.ecosystem == Ecosystem::Node
                        && !p.run.test.is_empty()
                        && node_member(root, &p.dir, &dir)
                });
                if member { None } else { node(root, &dir) }
            }
            "go.mod" => go(root, &dir),
            _ => python(root, &dir),
        };
        found.extend(part.filter(|p| !p.run.is_empty()));
    }
    if found.len() > 1 {
        for part in &mut found {
            if part.assumed_test {
                part.run.test.clear();
            }
        }
        found.retain(|p| !p.run.is_empty());
    }
    found
}

/// One verb's commands over several parts, as one line: the run file has
/// one `build`, one `check`, one `lint` and one `format`. A folder's
/// command is in a subshell, so the `cd` ends with it.
fn chain(parts: &[Part], verb: impl Fn(&RunFile) -> Option<&String>) -> Option<String> {
    let cmds: Vec<(&Part, &String)> = parts
        .iter()
        .filter_map(|p| verb(&p.run).map(|c| (p, c)))
        .collect();
    match cmds.as_slice() {
        [] => None,
        [(part, cmd)] => Some(part.run_from_top(cmd)),
        many => Some(
            many.iter()
                .map(|(part, cmd)| {
                    if part.dir.is_empty() {
                        (*cmd).clone()
                    } else {
                        format!("({})", part.run_from_top(cmd))
                    }
                })
                .collect::<Vec<_>>()
                .join(" && "),
        ),
    }
}

/// `a`, `a and b`, `a, b and c`.
fn listed(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// What the project's manifests say its verbs are: a run file with the
/// commands they imply, and the files they were read from. `None` when
/// nothing says how the project builds or tests. Every command runs from
/// the project's top. The product's own `start`, `ready` and `stop` are
/// never guessed: a server is the model's to read and propose.
pub fn detect(root: &Path) -> Option<(RunFile, String)> {
    let found = parts(root);
    let shown = &found[..found.len().min(MAX_PARTS)];
    let run = RunFile {
        build: chain(shown, |r| r.build.as_ref()),
        test: shown
            .iter()
            .flat_map(|p| p.run.test.iter().map(|t| p.run_from_top(t)))
            .collect(),
        check: chain(shown, |r| r.check.as_ref()),
        lint: chain(shown, |r| r.lint.as_ref()),
        format: chain(shown, |r| r.format.as_ref()),
        ..RunFile::default()
    };
    let names: Vec<String> = shown.iter().map(Part::name).collect();
    (!run.is_empty()).then(|| (run, listed(&names)))
}

/// One program in a command, as far as it can be read.
struct Step {
    /// The folder it runs in, from the top; `None` when that can't be read.
    folder: Option<String>,
    /// `None` for a program that says nothing of what it runs.
    ecosystem: Option<Ecosystem>,
    words: Vec<String>,
}

fn ecosystem_of(program: &str) -> Option<Ecosystem> {
    match program.rsplit('/').next().unwrap_or(program) {
        "cargo" => Some(Ecosystem::Cargo),
        "go" | "gofmt" => Some(Ecosystem::Go),
        "npm" | "npx" | "pnpm" | "yarn" | "bun" | "bunx" | "node" | "jest" | "vitest" | "mocha"
        | "tsc" | "eslint" | "prettier" => Some(Ecosystem::Node),
        "pytest" | "py.test" | "uv" | "poetry" | "pipenv" | "hatch" | "pdm" | "ruff" | "mypy"
        | "pyright" | "tox" | "nox" => Some(Ecosystem::Python),
        p if p.starts_with("python") => Some(Ecosystem::Python),
        "make" | "gmake" => Some(Ecosystem::Make),
        _ => None,
    }
}

/// `folder` after `cd word`: `None` when that can't be read from the line.
fn moved(folder: Option<&String>, word: Option<&super::Word>) -> Option<String> {
    let word = word?;
    if expands(&word.raw) {
        return None;
    }
    let mut path: Vec<&str> = folder?.split('/').filter(|s| !s.is_empty()).collect();
    for segment in Path::new(&word.text).components() {
        match segment {
            std::path::Component::Normal(name) => path.push(name.to_str()?),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                path.pop()?;
            }
            _ => return None,
        }
    }
    Some(path.join("/"))
}

/// The programs `cmd` runs, read as far as it can be: split at `&&`, `||`,
/// `;`, `|` and a subshell's brackets, a `cd` followed, and the folder
/// put back when a subshell ends. Anything it can't follow reads as a
/// program that says nothing, which is what keeps a reading honest.
fn steps(cmd: &str) -> Vec<Step> {
    fn take(piece: &mut String, folders: &mut [Option<String>], out: &mut Vec<Step>) {
        let text = std::mem::take(piece);
        if text.trim().is_empty() {
            return;
        }
        let folder = folders.last_mut().expect("there is always a folder");
        let unread = |folder: &Option<String>| Step {
            folder: folder.clone(),
            ecosystem: None,
            words: Vec::new(),
        };
        let Some(words) = shell_words(&text) else {
            out.push(unread(folder));
            return;
        };
        let words: Vec<_> = words
            .iter()
            .skip_while(|w| is_assignment(&w.text))
            .collect();
        let Some(first) = words.first() else {
            return;
        };
        if expands(&first.raw) {
            out.push(unread(folder));
            return;
        }
        match first.text.as_str() {
            "cd" => *folder = moved(folder.as_ref(), words.get(1).copied()),
            "echo" | "true" | ":" | "set" | "export" => {}
            program => out.push(Step {
                folder: folder.clone(),
                ecosystem: ecosystem_of(program),
                words: words.iter().map(|w| w.text.clone()).collect(),
            }),
        }
    }
    let mut out = Vec::new();
    let mut folders = vec![Some(String::new())];
    let mut piece = String::new();
    let mut quote_mark: Option<char> = None;
    let mut chars = cmd.chars().peekable();
    let mut previous = ' ';
    while let Some(c) = chars.next() {
        match quote_mark {
            Some(q) => {
                if c == q {
                    quote_mark = None;
                }
                piece.push(c);
            }
            None => match c {
                '\'' | '"' => {
                    quote_mark = Some(c);
                    piece.push(c);
                }
                // `2>&1` and `&>file` are a redirection, not a separator.
                '&' if previous == '>' || chars.peek() == Some(&'>') => piece.push(c),
                ';' | '&' | '|' | '\n' => take(&mut piece, &mut folders, &mut out),
                '(' => {
                    take(&mut piece, &mut folders, &mut out);
                    let inner = folders.last().cloned().flatten();
                    folders.push(inner);
                }
                ')' => {
                    take(&mut piece, &mut folders, &mut out);
                    if folders.len() > 1 {
                        folders.pop();
                    }
                }
                _ => piece.push(c),
            },
        }
        previous = c;
    }
    take(&mut piece, &mut folders, &mut out);
    out
}

/// Whether a word names `dir`, as a path or as an option's value:
/// `web`, `./web/tests`, `--manifest-path=web/Cargo.toml`.
fn names_folder(word: &str, dir: &str) -> bool {
    let value = word.rsplit('=').next().unwrap_or(word);
    let value = value.strip_prefix("./").unwrap_or(value);
    value == dir || under(value, dir).is_some()
}

/// What a set of commands does for a part.
#[derive(Debug, PartialEq, Eq)]
enum Reach {
    Runs,
    /// A command that might, and can't be read: that command.
    Unknown(String),
    Misses,
}

fn reach(cmds: &[&str], part: &Part) -> Reach {
    let mut unknown = None;
    for cmd in cmds {
        for step in steps(cmd) {
            let in_it = step.folder.as_deref() == Some(part.dir.as_str());
            let named =
                !part.dir.is_empty() && step.words.iter().any(|w| names_folder(w, &part.dir));
            let maybe = match step.ecosystem {
                Some(e) if e == part.ecosystem => {
                    if in_it || named {
                        return Reach::Runs;
                    }
                    // pytest and jest walk the folders under them; cargo
                    // and go stop at a package's own edge.
                    let walks = matches!(e, Ecosystem::Node | Ecosystem::Python);
                    step.folder
                        .as_deref()
                        .is_none_or(|f| walks && under(&part.dir, f).is_some())
                }
                Some(Ecosystem::Make) | None => true,
                Some(_) => false,
            };
            if maybe && unknown.is_none() {
                unknown = Some((*cmd).to_string());
            }
        }
    }
    unknown.map_or(Reach::Misses, Reach::Unknown)
}

/// `names`, cut short with a count of the rest.
fn named(names: &[String]) -> String {
    if names.len() <= MAX_NAMED {
        return listed(names);
    }
    let shown = &names[..MAX_NAMED];
    format!("{} and {} more", shown.join(", "), names.len() - MAX_NAMED)
}

/// How far a set of commands goes over the parts they could run.
#[derive(Debug, Default)]
struct Coverage {
    runs: Vec<String>,
    misses: Vec<String>,
    unsure: Vec<String>,
    /// The first command that might run one of `unsure`.
    unread: String,
}

/// `cmds` over `parts`, out of a project of `all` parts. A wrapper over a
/// project of one part is taken to run it: only with several is there
/// anything to ask of it.
fn coverage(cmds: &[&str], parts: &[&Part], all: usize) -> Coverage {
    let mut seen = Coverage::default();
    for part in parts {
        match reach(cmds, part) {
            Reach::Runs => seen.runs.push(part.name()),
            Reach::Misses => seen.misses.push(part.name()),
            Reach::Unknown(_) if all < 2 => seen.runs.push(part.name()),
            Reach::Unknown(cmd) => {
                seen.unsure.push(part.name());
                if seen.unread.is_empty() {
                    seen.unread = cmd;
                }
            }
        }
    }
    seen
}

fn test_commands(run: &RunFile) -> Vec<&str> {
    run.test.iter().map(String::as_str).collect()
}

/// What a run file reaches of the project, for the approval card.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Reaches {
    /// What its commands run: read after them.
    pub covers: Vec<String>,
    /// What they leave out, or may: read before anything is approved.
    pub leaves: Vec<String>,
}

/// `run` read against the project. Nothing for a project of one part
/// that it runs.
pub fn reaches(run: &RunFile, parts: &[Part]) -> Reaches {
    let mut out = Reaches::default();
    let mut say = |cmds: &[&str], wanted: Vec<&Part>, words: [&str; 3]| {
        if cmds.is_empty() {
            return;
        }
        let [cover, leave, may_leave] = words;
        let seen = coverage(cmds, &wanted, parts.len());
        if parts.len() > 1 && !seen.runs.is_empty() {
            out.covers.push(format!("{cover} {}", named(&seen.runs)));
        }
        if !seen.misses.is_empty() {
            out.leaves.push(format!("{leave} {}", named(&seen.misses)));
        }
        if !seen.unsure.is_empty() {
            out.leaves.push(format!(
                "{may_leave} {}: Ryter can't read `{}`",
                named(&seen.unsure),
                seen.unread
            ));
        }
    };
    say(
        &test_commands(run),
        parts.iter().filter(|p| !p.run.test.is_empty()).collect(),
        ["tests cover", "tests leave out", "tests may leave out"],
    );
    let check: Vec<&str> = run.check.iter().map(String::as_str).collect();
    say(
        &check,
        parts.iter().filter(|p| p.run.check.is_some()).collect(),
        [
            "the check covers",
            "the check leaves out",
            "the check may leave out",
        ],
    );
    out
}

/// What a run of the whole suite did not run, for the model: a suite that
/// passes over half a project is not the project's tests passing.
pub fn suite_note(run: &RunFile, parts: &[Part]) -> Option<String> {
    let wanted: Vec<&Part> = parts.iter().filter(|p| !p.run.test.is_empty()).collect();
    let seen = coverage(&test_commands(run), &wanted, parts.len());
    let mut lines = Vec::new();
    if !seen.misses.is_empty() {
        lines.push(format!(
            "[not in this suite: {}; those have not been tested]",
            named(&seen.misses)
        ));
    }
    if !seen.unsure.is_empty() {
        lines.push(format!(
            "[maybe not in this suite: {}; Ryter can't read `{}`]",
            named(&seen.unsure),
            seen.unread
        ));
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::super::{Scope, narrow};
    use super::*;

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), files);
        root
    }

    fn write(root: &Path, files: &[(&str, &str)]) {
        for (path, text) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    fn names(root: &Path) -> Vec<String> {
        parts(root).iter().map(Part::name).collect()
    }

    const TESTS: &str = r#"{"scripts":{"test":"vitest run"}}"#;
    const FULL: &str = r#"{"scripts":{"test":"vitest run","build":"vite build","lint":"eslint .","format":"prettier -w ."}}"#;
    const CRATE: &str = "[package]\nname = \"x\"\n";

    #[test]
    fn a_rust_project_with_a_frontend_gets_both_at_the_top() {
        let root = project(&[("Cargo.toml", CRATE), ("package.json", FULL)]);
        let (run, from) = detect(root.path()).unwrap();
        assert_eq!(from, "Cargo.toml and package.json");
        assert_eq!(run.test, ["cargo test", "npm test"]);
        assert_eq!(run.build.as_deref(), Some("cargo build && npm run build"));
        assert_eq!(
            run.lint.as_deref(),
            Some("cargo clippy --all-targets && npm run lint")
        );
        assert_eq!(run.format.as_deref(), Some("cargo fmt && npm run format"));
        // A part with no check has no place in the check.
        assert_eq!(run.check.as_deref(), Some("cargo check"));
    }

    #[test]
    fn a_nested_part_runs_from_its_own_folder() {
        let root = project(&[
            ("Cargo.toml", CRATE),
            ("web/package.json", FULL),
            ("services/api/go.mod", "module x\n"),
        ]);
        let (run, from) = detect(root.path()).unwrap();
        assert_eq!(from, "Cargo.toml, web/package.json and services/api/go.mod");
        assert_eq!(
            run.test,
            [
                "cargo test",
                "cd web && npm test",
                "cd services/api && go test ./..."
            ]
        );
        assert_eq!(
            run.build.as_deref(),
            Some("cargo build && (cd web && npm run build) && (cd services/api && go build ./...)")
        );
        // Alone, it needs no subshell.
        assert_eq!(
            run.lint.as_deref(),
            Some("cargo clippy --all-targets && (cd web && npm run lint)")
        );
        let only = project(&[("frontend/package.json", FULL)]);
        let (run, from) = detect(only.path()).unwrap();
        assert_eq!(from, "frontend/package.json");
        assert_eq!(run.test, ["cd frontend && npm test"]);
        assert_eq!(run.build.as_deref(), Some("cd frontend && npm run build"));
    }

    #[test]
    fn a_folder_with_a_space_is_quoted() {
        let root = project(&[("my app/package.json", TESTS)]);
        let (run, _) = detect(root.path()).unwrap();
        assert_eq!(run.test, ["cd 'my app' && npm test"]);
    }

    #[test]
    fn a_cargo_workspace_runs_its_members() {
        let root = project(&[
            (
                "Cargo.toml",
                "[workspace]\nmembers = [\"crates/*\"]\nexclude = [\"tools/own\"]\n",
            ),
            ("crates/a/Cargo.toml", CRATE),
            ("crates/b/Cargo.toml", CRATE),
            ("tools/own/Cargo.toml", CRATE),
            ("tools/other/Cargo.toml", "[workspace]\n"),
        ]);
        assert_eq!(
            names(root.path()),
            [
                "Cargo.toml",
                "tools/other/Cargo.toml",
                "tools/own/Cargo.toml"
            ]
        );
        let (run, _) = detect(root.path()).unwrap();
        assert_eq!(run.test[0], "cargo test --workspace");
        assert_eq!(run.test[1], "cd tools/other && cargo test --workspace");
        assert_eq!(run.test[2], "cd tools/own && cargo test");
    }

    #[test]
    fn a_package_beside_a_plain_crate_is_a_part_of_its_own() {
        let root = project(&[("Cargo.toml", CRATE), ("sub/Cargo.toml", CRATE)]);
        assert_eq!(names(root.path()), ["Cargo.toml", "sub/Cargo.toml"]);
    }

    #[test]
    fn a_node_workspace_with_tests_runs_its_members() {
        let root = project(&[
            (
                "package.json",
                r#"{"workspaces":["packages/*","!packages/own"],"scripts":{"test":"turbo test"}}"#,
            ),
            ("packages/a/package.json", TESTS),
            ("packages/own/package.json", TESTS),
            ("packages/a/deep/package.json", TESTS),
            ("site/package.json", TESTS),
        ]);
        assert_eq!(
            names(root.path()),
            [
                "package.json",
                "site/package.json",
                "packages/own/package.json",
                "packages/a/deep/package.json"
            ]
        );
    }

    #[test]
    fn a_workspace_with_no_tests_of_its_own_leaves_them_to_its_members() {
        let root = project(&[
            (
                "package.json",
                r#"{"workspaces":{"packages":["packages/*"]}}"#,
            ),
            ("pnpm-lock.yaml", ""),
            ("packages/a/package.json", TESTS),
            ("packages/b/package.json", TESTS),
        ]);
        let (run, _) = detect(root.path()).unwrap();
        // The lockfile at the workspace's root picks the runner.
        assert_eq!(
            run.test,
            ["cd packages/a && pnpm test", "cd packages/b && pnpm test"]
        );
    }

    #[test]
    fn pnpms_own_file_names_a_workspace() {
        let root = project(&[
            ("package.json", TESTS),
            (
                "pnpm-workspace.yaml",
                "# the packages\npackages:\n  - 'apps/*' # the apps\n  - \"libs/**\"\nother:\n  - no\n",
            ),
            ("apps/a/package.json", TESTS),
            ("libs/x/y/package.json", TESTS),
            ("no/package.json", TESTS),
        ]);
        assert_eq!(names(root.path()), ["package.json", "no/package.json"]);
    }

    #[test]
    fn what_is_not_the_projects_own_is_not_a_part() {
        let root = project(&[
            ("Cargo.toml", CRATE),
            ("tests/fixtures/app/package.json", TESTS),
            ("bench/task/repo/Cargo.toml", CRATE),
            ("examples/demo/package.json", TESTS),
            ("node_modules/left-pad/package.json", TESTS),
            ("target/package/Cargo.toml", CRATE),
            (".hidden/package.json", TESTS),
            ("ignored/package.json", TESTS),
            ("a/b/c/d/package.json", TESTS),
            (".gitignore", "ignored/\n"),
        ]);
        assert_eq!(names(root.path()), ["Cargo.toml"]);
    }

    #[test]
    fn the_top_is_walked_whatever_it_is_called() {
        let root = tempfile::tempdir().unwrap();
        let top = root.path().join("tests");
        write(&top, &[("web/package.json", TESTS)]);
        assert_eq!(names(&top), ["web/package.json"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_a_folder_is_not_followed() {
        let outside = project(&[("package.json", TESTS)]);
        let root = project(&[("Cargo.toml", CRATE)]);
        std::os::unix::fs::symlink(outside.path(), root.path().join("web")).unwrap();
        assert_eq!(names(root.path()), ["Cargo.toml"]);
    }

    #[test]
    fn a_folder_that_reads_as_an_option_is_no_part() {
        let root = project(&[("-/package.json", TESTS), ("--x/package.json", TESTS)]);
        assert_eq!(detect(root.path()), None);
    }

    #[test]
    fn npms_placeholder_is_no_test() {
        let root = project(&[
            ("Cargo.toml", CRATE),
            (
                "web/package.json",
                r#"{"scripts":{"test":"echo \"Error: no test specified\" && exit 1"}}"#,
            ),
        ]);
        assert_eq!(names(root.path()), ["Cargo.toml"]);
    }

    #[test]
    fn a_broken_manifest_leaves_the_others_standing() {
        let root = project(&[("package.json", "{ not json"), ("go.mod", "module x\n")]);
        assert_eq!(names(root.path()), ["go.mod"]);
    }

    #[test]
    fn a_linters_settings_are_no_test_suite() {
        let linted = "[tool.ruff]\n";
        let mixed = project(&[("package.json", TESTS), ("pyproject.toml", linted)]);
        let (run, from) = detect(mixed.path()).unwrap();
        assert_eq!(from, "package.json and pyproject.toml");
        assert_eq!(run.test, ["npm test"]);
        assert_eq!(run.lint.as_deref(), Some("ruff check ."));
        // Settings for nothing Ryter runs leave no part at all.
        let plain = project(&[("package.json", TESTS), ("setup.cfg", "[flake8]\n")]);
        assert_eq!(names(plain.path()), ["package.json"]);
        // Alone, the default stands, as it did.
        let alone = project(&[("pyproject.toml", linted)]);
        assert_eq!(
            detect(alone.path()).unwrap().0.test,
            ["python3 -m unittest"]
        );
        // With tests of its own, it stands beside the others.
        let tested = project(&[
            ("package.json", TESTS),
            ("pyproject.toml", linted),
            ("tests/test_x.py", ""),
        ]);
        assert_eq!(
            detect(tested.path()).unwrap().0.test,
            ["npm test", "python3 -m unittest"]
        );
    }

    #[test]
    fn a_makefile_stands_only_with_nothing_else_at_the_top() {
        let root = project(&[("Makefile", "test:\n\t./t\n"), ("Cargo.toml", CRATE)]);
        assert_eq!(names(root.path()), ["Cargo.toml"]);
        let aggregator = project(&[("Makefile", "test:\n\t./t\n"), ("web/package.json", TESTS)]);
        assert_eq!(names(aggregator.path()), ["Makefile", "web/package.json"]);
        let no_scripts = project(&[("package.json", "{}"), ("Makefile", "test:\n\t./t\n")]);
        assert_eq!(names(no_scripts.path()), ["Makefile"]);
    }

    #[test]
    fn a_large_project_says_what_it_left_out() {
        let packages: Vec<(String, &str)> = (0..11)
            .map(|i| (format!("packages/p{i:02}/package.json"), TESTS))
            .collect();
        let files: Vec<(&str, &str)> = packages.iter().map(|(p, t)| (p.as_str(), *t)).collect();
        let root = project(&files);
        let (run, _) = detect(root.path()).unwrap();
        assert_eq!(run.test.len(), MAX_PARTS);
        let lines = scope_lines(&run, &parts(root.path()));
        assert!(lines[0].starts_with("tests cover packages/p00/package.json, "));
        assert_eq!(
            lines[1],
            "tests leave out packages/p08/package.json, packages/p09/package.json and \
             packages/p10/package.json"
        );
    }

    fn scope_lines(run: &RunFile, parts: &[Part]) -> Vec<String> {
        let found = reaches(run, parts);
        found.covers.into_iter().chain(found.leaves).collect()
    }

    fn with_tests(tests: &[&str]) -> RunFile {
        RunFile {
            test: tests.iter().map(|t| (*t).to_string()).collect(),
            ..RunFile::default()
        }
    }

    #[test]
    fn a_project_of_one_part_says_nothing_of_its_scope() {
        let root = project(&[("Cargo.toml", CRATE)]);
        let found = parts(root.path());
        let (run, _) = detect(root.path()).unwrap();
        assert!(scope_lines(&run, &found).is_empty());
        assert_eq!(suite_note(&run, &found), None);
        // Its own wrapper is taken to run it.
        let wrapped = with_tests(&["./scripts/test.sh"]);
        assert!(scope_lines(&wrapped, &found).is_empty());
        assert_eq!(suite_note(&wrapped, &found), None);
    }

    #[test]
    fn what_was_found_is_covered_by_what_was_proposed() {
        let root = project(&[
            ("Cargo.toml", CRATE),
            ("web/package.json", FULL),
            ("web/tsconfig.json", "{}"),
        ]);
        let found = parts(root.path());
        let (run, _) = detect(root.path()).unwrap();
        assert_eq!(
            scope_lines(&run, &found),
            [
                "tests cover Cargo.toml and web/package.json",
                "the check covers Cargo.toml and web/package.json"
            ]
        );
        assert_eq!(suite_note(&run, &found), None);
    }

    #[test]
    fn a_run_file_that_leaves_a_part_out_is_read_as_it_stands() {
        let root = project(&[("Cargo.toml", CRATE), ("web/package.json", TESTS)]);
        let found = parts(root.path());
        let run = with_tests(&["cargo test"]);
        assert_eq!(
            scope_lines(&run, &found),
            ["tests cover Cargo.toml", "tests leave out web/package.json"]
        );
        assert_eq!(
            suite_note(&run, &found).unwrap(),
            "[not in this suite: web/package.json; those have not been tested]"
        );
    }

    #[test]
    fn however_a_command_gets_to_a_part_it_runs_it() {
        let root = project(&[("Cargo.toml", CRATE), ("web/package.json", TESTS)]);
        let found = parts(root.path());
        let web = &found[1];
        for cmd in [
            "cd web && npm test",
            "cd ./web && npm test",
            "(cd web && npm test)",
            "cargo test && cd web && npm test",
            "cargo test; cd web; npm test",
            "cd web && CI=1 npm test 2>&1",
            "npm --prefix web test",
            "pnpm --dir=web test",
            "npx jest ./web/tests",
            "export CI=1 && cd web && npm test",
            "cd web/src && cd .. && npm test",
        ] {
            assert_eq!(reach(&[cmd], web), Reach::Runs, "{cmd}");
        }
        for cmd in [
            "cargo test",
            "cd web && cargo test",
            "(cd web && cargo build) && cargo test",
            "cd other && npm test",
            "(cd web) && cargo test --workspace",
        ] {
            assert_eq!(reach(&[cmd], web), Reach::Misses, "{cmd}");
        }
    }

    #[test]
    fn a_subshell_ends_its_own_cd() {
        let root = project(&[("package.json", TESTS), ("web/package.json", TESTS)]);
        let found = parts(root.path());
        assert_eq!(found.len(), 2);
        // Run from the top, after the subshell: the top's, and not web's...
        let cmd = "(cd web && true) && npm test";
        assert_eq!(reach(&[cmd], &found[0]), Reach::Runs);
        // ...which a runner above it may walk into, so it can't be said.
        assert_eq!(reach(&[cmd], &found[1]), Reach::Unknown(cmd.to_string()));
    }

    #[test]
    fn what_cant_be_read_is_said_to_be_unread() {
        let root = project(&[("Cargo.toml", CRATE), ("web/package.json", TESTS)]);
        let found = parts(root.path());
        for cmd in [
            "make test",
            "./scripts/test.sh",
            "docker compose run app test",
            "cd \"$WEB\" && npm test",
            "cd /elsewhere && npm test",
            "$RUNNER test",
        ] {
            assert_eq!(
                reach(&[cmd], &found[1]),
                Reach::Unknown(cmd.to_string()),
                "{cmd}"
            );
        }
        let run = with_tests(&["make test"]);
        assert_eq!(
            scope_lines(&run, &found),
            ["tests may leave out Cargo.toml and web/package.json: Ryter can't read `make test`"]
        );
        assert_eq!(
            suite_note(&run, &found).unwrap(),
            "[maybe not in this suite: Cargo.toml and web/package.json; Ryter can't read \
             `make test`]"
        );
    }

    #[test]
    fn a_project_of_several_parts_says_which_have_the_tests() {
        let root = project(&[
            ("go.mod", "module x\n"),
            ("web/package.json", r#"{"scripts":{"build":"vite build"}}"#),
        ]);
        let (run, _) = detect(root.path()).unwrap();
        assert_eq!(run.test, ["go test ./..."]);
        assert_eq!(
            scope_lines(&run, &parts(root.path())),
            ["tests cover go.mod", "the check covers go.mod"]
        );
    }

    #[test]
    fn a_go_module_under_another_is_its_own() {
        let root = project(&[("go.mod", "module x\n"), ("sub/go.mod", "module y\n")]);
        let found = parts(root.path());
        assert_eq!(reach(&["go test ./..."], &found[1]), Reach::Misses);
        assert_eq!(
            detect(root.path()).unwrap().0.test,
            ["go test ./...", "cd sub && go test ./..."]
        );
    }

    #[test]
    fn a_check_that_leaves_a_part_out_is_named_apart_from_the_tests() {
        let root = project(&[
            ("Cargo.toml", CRATE),
            ("web/package.json", TESTS),
            ("web/tsconfig.json", "{}"),
        ]);
        let run = RunFile {
            test: vec!["cargo test".into(), "cd web && npm test".into()],
            check: Some("cargo check".into()),
            ..RunFile::default()
        };
        assert_eq!(
            scope_lines(&run, &parts(root.path())),
            [
                "tests cover Cargo.toml and web/package.json",
                "the check covers Cargo.toml",
                "the check leaves out web/package.json"
            ]
        );
    }

    fn narrowed(cmd: &str, filter: Option<&str>, files: &[&str], root: &Path) -> String {
        let files: Vec<String> = files.iter().map(|f| (*f).to_string()).collect();
        match narrow(cmd, filter, &files, root) {
            Scope::Narrowed(n) => n.cmd,
            Scope::Cannot(why) => format!("cannot: {why}"),
        }
    }

    #[test]
    fn a_folders_command_narrows_in_that_folder() {
        let root = project(&[
            ("Cargo.toml", CRATE),
            ("src/lib.rs", ""),
            ("web/package.json", TESTS),
            ("web/src/a.test.ts", ""),
            ("web/src/a.ts", ""),
        ]);
        let cmd = "cd web && npm test";
        assert_eq!(
            narrowed(cmd, Some("adds"), &[], root.path()),
            "cd web && npm test -- -t adds"
        );
        assert_eq!(
            narrowed(cmd, None, &["web/src/a.test.ts"], root.path()),
            "cd web && npm test -- src/a.test.ts"
        );
        // A source file finds its test from the folder's own index.
        assert_eq!(
            narrowed(cmd, None, &["web/src/a.ts"], root.path()),
            "cd web && npm test -- src/a.test.ts"
        );
        // Another part's file is not this command's.
        assert_eq!(
            narrowed(cmd, None, &["src/lib.rs"], root.path()),
            "cannot: none of those files is in web"
        );
        // Nor is a filter run here for it.
        assert_eq!(
            narrowed(cmd, Some("adds"), &["src/lib.rs"], root.path()),
            "cannot: none of those files is in web"
        );
        // Written with its `./`, it is the same folder.
        assert_eq!(
            narrowed(
                "cd ./web && npm test",
                None,
                &["web/src/a.test.ts"],
                root.path()
            ),
            "cd ./web && npm test -- src/a.test.ts"
        );
        // And the top's command is left to say what it says of web's.
        assert!(narrowed("cargo test", None, &["web/src/a.ts"], root.path()).starts_with("cannot"));
    }

    #[test]
    fn what_isnt_a_plain_folder_stays_a_compound_command() {
        let root = project(&[("web/package.json", TESTS), ("package.json", TESTS)]);
        for cmd in [
            "cd ../web && npm test",
            "cd /tmp && npm test",
            "cd missing && npm test",
            "cd \"$WEB\" && npm test",
            "cd web; x && npm test",
            "cd web && npm test && npm run lint",
            "(cd web && npm test)",
        ] {
            assert!(
                narrowed(cmd, Some("x"), &[], root.path()).starts_with("cannot: "),
                "{cmd}"
            );
        }
        // Whole, it runs as written.
        assert_eq!(
            narrowed("cd missing && npm test", None, &[], root.path()),
            "cd missing && npm test"
        );
    }
}
