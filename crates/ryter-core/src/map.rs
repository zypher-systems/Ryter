//! The project map: the project's folders and the signatures of its source
//! files, put in front of the model when a session's first turn begins, so
//! it goes to a file instead of searching for it.
//!
//! A model new to a project spends its first rounds listing folders and
//! reading files to find where things are, and reads whole files to find
//! one function; what it learns that way fills its context with files it
//! looked at once. The map is what a developer knows after a week in the
//! project: which folders hold what, and what each source file defines.
//! Signatures only, no bodies and no line numbers: the file is still read
//! before it is edited. The map is bounded, so a large project is not shown
//! whole; `glob` and `grep` find the rest.

use std::collections::BTreeMap;
use std::path::Path;

/// The most characters a map may run to, unless `[map] chars` says otherwise.
pub const DEFAULT_CHARS: usize = 12_000;
/// The most entries the walk visits: a map of a project this large is a
/// sample anyway.
const MAX_ENTRIES: usize = 20_000;
/// A source file larger than this is named, not read for signatures.
const MAX_FILE_BYTES: u64 = 1_000_000;
/// How much of a source file is read for signatures.
const READ_BYTES: usize = 262_144;
/// Signatures one file may show.
const SIGNATURES_PER_FILE: usize = 24;
/// Characters one signature may run to.
const SIGNATURE_CHARS: usize = 100;
/// Files named on the root line, and files without signatures named in a
/// folder's line, before they are counted instead.
const NAMED: usize = 24;
/// Folders that hold build output or other people's code, skipped even
/// when nothing ignores them.
const SKIP_DIRS: &[&str] = &[
    "target",
    "node_modules",
    "dist",
    "build",
    "__pycache__",
    ".venv",
    "venv",
    "vendor",
    "coverage",
    ".next",
];
/// Extensions of files that are data, documents or media: counted per
/// folder, named only at the root.
const DATA: &[&str] = &[
    "md", "txt", "rst", "json", "yaml", "yml", "lock", "png", "jpg", "jpeg", "gif", "svg", "ico",
    "webp", "pdf", "csv", "tsv", "xml", "log", "snap", "map", "wasm", "woff", "woff2", "ttf",
    "eot", "mp3", "mp4", "zip", "gz", "tar", "bin", "dat",
];

/// What a source file comes to in the map.
#[derive(Debug, Default)]
struct Dir {
    /// Source files, by name, with their signatures (none for a language
    /// the map has no reader for, or a file too large or not text).
    files: BTreeMap<String, Vec<String>>,
    /// Data files, counted.
    data: usize,
}

/// The map of the project at `root`, within `chars` characters; `None`
/// for a root that is not a folder or holds no files.
pub fn build(root: &Path, chars: usize) -> Option<String> {
    if chars == 0 || !root.is_dir() {
        return None;
    }
    let mut walker = ignore::WalkBuilder::new(root);
    walker
        .hidden(true)
        .git_ignore(true)
        .git_exclude(true)
        .require_git(false)
        .sort_by_file_name(|a, b| a.cmp(b))
        .filter_entry(|e| {
            !(e.file_type().is_some_and(|t| t.is_dir())
                && e.file_name()
                    .to_str()
                    .is_some_and(|n| SKIP_DIRS.contains(&n)))
        });
    let mut dirs: BTreeMap<String, Dir> = BTreeMap::new();
    let mut langs: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut files = 0usize;
    let mut sampled = false;
    for (n, entry) in walker.build().flatten().enumerate() {
        if n >= MAX_ENTRIES {
            sampled = true;
            break;
        }
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        let Some(name) = rel.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let parent = rel
            .parent()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let ext = rel
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        files += 1;
        *langs.entry(language(&ext)).or_default() += 1;
        let dir = dirs.entry(parent).or_default();
        if DATA.contains(&ext.as_str()) {
            dir.data += 1;
            continue;
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(u64::MAX);
        let signatures = if size <= MAX_FILE_BYTES {
            read_head(entry.path())
                .map(|text| signatures(&ext, &text))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        dir.files.insert(name.to_string(), signatures);
    }
    if files == 0 {
        return None;
    }
    Some(render(root, &dirs, &langs, files, sampled, chars))
}

/// The section of the prompt the map goes in.
pub fn section(map: &str) -> String {
    format!(
        "## Project map\nThe project's folders and the signatures of its source files, read \
         when this session's first turn began; what has been edited since is not in it \
         until the conversation is compacted. Go to the file it names instead of searching \
         for it. It has no line numbers and no bodies: read a file before you edit it. A \
         large project is not shown whole; `glob` and `grep` find what is not here.\n\n\
         {map}\n"
    )
}

/// The first `READ_BYTES` of a text file; `None` for one that is not text.
fn read_head(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::with_capacity(READ_BYTES.min(65_536));
    file.by_ref()
        .take(READ_BYTES as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.iter().take(8_192).any(|b| *b == 0) {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn render(
    root: &Path,
    dirs: &BTreeMap<String, Dir>,
    langs: &BTreeMap<&'static str, usize>,
    files: usize,
    sampled: bool,
    chars: usize,
) -> String {
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string());
    let mut counts: Vec<(&str, usize)> = langs.iter().map(|(l, n)| (*l, *n)).collect();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let counted: Vec<String> = counts
        .iter()
        .take(6)
        .map(|(l, n)| format!("{l} {n}"))
        .collect();
    let mut out = format!(
        "{name}/ · {files} file{} · {}{}\n",
        if files == 1 { "" } else { "s" },
        counted.join(", "),
        if sampled {
            " · sampled: the walk stopped early"
        } else {
            ""
        }
    );
    // The root's own files: the manifests and documents that say what the
    // project is, named, and a source file at the top (`app.py`, `main.go`)
    // with its signatures like any other.
    if let Some(top) = dirs.get("") {
        let mut named: Vec<&str> = top
            .files
            .iter()
            .filter(|(_, sigs)| sigs.is_empty())
            .map(|(n, _)| n.as_str())
            .collect();
        let more = named.len().saturating_sub(NAMED) + top.data;
        named.truncate(NAMED);
        let top_data = root_data_names(root);
        let more = more.saturating_sub(top_data.len());
        named.extend(top_data.iter().map(String::as_str));
        if more > 0 {
            out.push_str(&format!("{}  +{more} more\n", named.join("  ")));
        } else if !named.is_empty() {
            out.push_str(&named.join("  "));
            out.push('\n');
        }
        for (name, sigs) in top.files.iter().filter(|(_, sigs)| !sigs.is_empty()) {
            out.push_str(&file_line(name, sigs));
        }
    }
    // Folders shallowest first: the shape before the detail, so a budget
    // that runs out leaves the deep folders unmapped, not the top ones.
    let mut order: Vec<&String> = dirs.keys().filter(|k| !k.is_empty()).collect();
    order.sort_by_key(|k| (k.matches('/').count(), (*k).clone()));
    let mut left_files = 0usize;
    let mut left_dirs = 0usize;
    let mut full = false;
    for key in order {
        let dir = &dirs[key];
        if full {
            left_dirs += 1;
            left_files += dir.files.len() + dir.data;
            continue;
        }
        let block = dir_block(key, dir);
        if out.len() + block.len() > chars {
            full = true;
            left_dirs += 1;
            left_files += dir.files.len() + dir.data;
            continue;
        }
        out.push_str(&block);
    }
    if left_files > 0 {
        out.push_str(&format!(
            "… {left_files} more file{} in {left_dirs} folder{} not shown; glob and grep find them.\n",
            if left_files == 1 { "" } else { "s" },
            if left_dirs == 1 { "" } else { "s" }
        ));
    }
    out.trim_end().to_string()
}

/// The root's data files by name (`README.md`, `Cargo.lock`): the walk
/// counted them, and the root's are worth naming.
fn root_data_names(root: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| {
            !n.starts_with('.')
                && Path::new(n)
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| DATA.contains(&e.to_ascii_lowercase().as_str()))
        })
        .collect();
    names.sort();
    names.truncate(NAMED);
    names
}

fn dir_block(key: &str, dir: &Dir) -> String {
    let total = dir.files.len() + dir.data;
    let mut s = format!(
        "{key}/ · {total} file{}\n",
        if total == 1 { "" } else { "s" }
    );
    let mut unread: Vec<&str> = Vec::new();
    for (name, sigs) in &dir.files {
        if sigs.is_empty() {
            unread.push(name);
            continue;
        }
        s.push_str(&file_line(name, sigs));
    }
    if !unread.is_empty() {
        let more = unread.len().saturating_sub(NAMED);
        unread.truncate(NAMED);
        s.push_str(&format!("  {}", unread.join("  ")));
        if more > 0 {
            s.push_str(&format!("  +{more} more"));
        }
        s.push('\n');
    }
    if dir.data > 0 {
        s.push_str(&format!(
            "  +{} data file{}\n",
            dir.data,
            if dir.data == 1 { "" } else { "s" }
        ));
    }
    s
}

/// A source file's line: its name and its signatures, the most a file shows.
fn file_line(name: &str, sigs: &[String]) -> String {
    let shown: Vec<&str> = sigs
        .iter()
        .take(SIGNATURES_PER_FILE)
        .map(String::as_str)
        .collect();
    let mut s = format!("  {name}: {}", shown.join(" · "));
    if sigs.len() > shown.len() {
        s.push_str(&format!(" · +{} more", sigs.len() - shown.len()));
    }
    s.push('\n');
    s
}

/// The language a file extension says, for the count on the first line.
fn language(ext: &str) -> &'static str {
    match ext {
        "rs" => "Rust",
        "py" | "pyi" => "Python",
        "ts" | "tsx" | "mts" | "cts" => "TypeScript",
        "js" | "jsx" | "mjs" | "cjs" => "JavaScript",
        "go" => "Go",
        "c" | "h" => "C",
        "cpp" | "cc" | "cxx" | "hpp" | "hh" => "C++",
        "java" => "Java",
        "kt" | "kts" => "Kotlin",
        "swift" => "Swift",
        "cs" => "C#",
        "rb" => "Ruby",
        "php" => "PHP",
        "sh" | "bash" | "zsh" => "Shell",
        "lua" => "Lua",
        "zig" => "Zig",
        "ex" | "exs" => "Elixir",
        "hs" => "Haskell",
        "scala" => "Scala",
        "dart" => "Dart",
        "sql" => "SQL",
        "html" | "htm" => "HTML",
        "css" | "scss" | "less" => "CSS",
        "md" => "Markdown",
        "toml" => "TOML",
        "json" => "JSON",
        "yaml" | "yml" => "YAML",
        _ => "other",
    }
}

/// The signatures in `text`, by its language; none for a language the map
/// has no reader for.
fn signatures(ext: &str, text: &str) -> Vec<String> {
    match ext {
        "rs" => rust(text),
        "py" | "pyi" => python(text),
        "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" => typescript(text),
        "go" => go(text),
        _ => Vec::new(),
    }
}

/// `line` up to its body or its value: the first of the `stops` found.
fn head(line: &str, stops: &[&str]) -> String {
    head_from(line, 0, stops)
}

/// [`head`], with the `stops` looked for from `from` on: past a
/// `pub(crate)` whose parenthesis is not the one meant.
fn head_from(line: &str, from: usize, stops: &[&str]) -> String {
    let mut end = line.len();
    for stop in stops {
        if let Some(i) = line[from.min(line.len())..].find(stop) {
            end = end.min(from + i);
        }
    }
    let mut s = line[..end].trim().to_string();
    if s.chars().count() > SIGNATURE_CHARS {
        s = s.chars().take(SIGNATURE_CHARS - 1).collect();
        s.push('…');
    }
    s
}

/// Rust: public items and `impl` blocks, with `fn main`.
fn rust(text: &str) -> Vec<String> {
    const ITEMS: &[&str] = &[
        "fn ",
        "async fn ",
        "const fn ",
        "unsafe fn ",
        "struct ",
        "enum ",
        "trait ",
        "type ",
        "mod ",
        "const ",
        "static ",
        "union ",
    ];
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        // The item with its visibility, and where the keyword starts past
        // `pub`, `pub(crate)` or `pub(super)`.
        let item = if let Some(rest) = t.strip_prefix("pub") {
            let rest = match rest.strip_prefix('(') {
                Some(r) => r.split_once(')').map_or("", |(_, r)| r),
                None => rest,
            };
            rest.strip_prefix(' ')
                .filter(|r| ITEMS.iter().any(|i| r.starts_with(i)))
                .map(|r| (t, t.len() - r.len()))
        } else if t.starts_with("impl ") || t.starts_with("impl<") || t.starts_with("fn main(") {
            Some((t, 0))
        } else {
            None
        };
        let Some((item, from)) = item else {
            continue;
        };
        let keyword = &item[from..];
        let stops: &[&str] = if keyword.contains("fn ") {
            &[" {", ";", " where"]
        } else if keyword.starts_with("impl") {
            &[" {", " where"]
        } else {
            &[" {", ";", " where", " = ", "("]
        };
        out.push(head_from(item, from, stops));
    }
    out
}

/// Python: top-level classes and functions, and a class's methods by name.
fn python(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut class: Option<String> = None;
    for line in text.lines() {
        let indented = line.starts_with("    ") || line.starts_with('\t');
        let t = line.trim();
        if !indented {
            if let Some(rest) = t.strip_prefix("class ") {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                out.push(head(t.trim_end_matches(':'), &[]));
                class = Some(name);
            } else if t.starts_with("def ") || t.starts_with("async def ") {
                let mut sig = head(t.trim_end_matches(':'), &[]);
                if !sig.contains(')') {
                    sig.push_str("…)");
                }
                out.push(sig);
                class = None;
            } else if !t.is_empty() && !t.starts_with('#') && !t.starts_with('@') {
                class = None;
            }
            continue;
        }
        let Some(class) = &class else {
            continue;
        };
        let def = t
            .strip_prefix("async def ")
            .or_else(|| t.strip_prefix("def "));
        if let Some(rest) = def {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.starts_with("__") || name == "__init__" {
                out.push(format!("{class}.{name}"));
            }
        }
    }
    out
}

/// TypeScript and JavaScript: exports, and top-level functions and classes.
fn typescript(text: &str) -> Vec<String> {
    const DECLS: &[&str] = &[
        "async function ",
        "function ",
        "function* ",
        "class ",
        "abstract class ",
        "interface ",
        "type ",
        "enum ",
        "const enum ",
        "const ",
        "let ",
        "var ",
        "namespace ",
    ];
    let mut out = Vec::new();
    for line in text.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        let t = line.trim_end();
        let rest = t.strip_prefix("export ").unwrap_or(t);
        let rest = rest.strip_prefix("default ").unwrap_or(rest);
        let rest = rest.strip_prefix("declare ").unwrap_or(rest);
        let exported = t.starts_with("export ");
        let decl = DECLS.iter().any(|d| rest.starts_with(d));
        if !decl {
            continue;
        }
        // An unexported variable is not the file's shape.
        if !exported
            && (rest.starts_with("const ") || rest.starts_with("let ") || rest.starts_with("var "))
        {
            continue;
        }
        let stops: &[&str] = if rest.starts_with("const ")
            || rest.starts_with("let ")
            || rest.starts_with("var ")
            || rest.starts_with("type ")
        {
            &[" = ", " ="]
        } else {
            &[" {", "{", ";"]
        };
        out.push(head(t, stops));
    }
    out
}

/// Go: functions, methods and types.
fn go(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim_end();
        if t.starts_with("func ") || t.starts_with("type ") {
            out.push(head(t, &[" {"]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn put(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn project() -> TempDir {
        let d = TempDir::new().unwrap();
        let r = d.path();
        put(r, "Cargo.toml", "[package]\nname = \"app\"\n");
        put(r, "README.md", "# app\n");
        put(r, "build.rs", "fn main() {}\n");
        put(
            r,
            "src/lib.rs",
            "//! doc\npub struct Agent {\n    x: u32,\n}\n\nimpl Agent {\n    pub fn new(x: u32) -> Self {\n        Self { x }\n    }\n    fn hidden(&self) {}\n    pub async fn turn(&mut self, text: &str) -> Result<TurnResult, Error>\n    where\n        Error: Send,\n    {\n        todo!()\n    }\n}\n\nimpl std::fmt::Display for Agent {\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { Ok(()) }\n}\npub(crate) const CAP: usize = 3;\npub enum Mode { A, B }\nfn private() {}\n",
        );
        put(
            r,
            "tools/app.py",
            "import os\n\nclass Runner(Base):\n    def __init__(self, x):\n        self.x = x\n\n    def run(self, task):\n        pass\n\n    def __repr__(self):\n        return ''\n\ndef main(argv=None):\n    pass\n\nasync def fetch(url,\n                timeout=3):\n    pass\n",
        );
        put(
            r,
            "web/index.ts",
            "import x from 'y';\nexport function debounce<T>(fn: T, wait: number): T {\n  return fn;\n}\nexport const limit = 3;\nconst secret = 1;\nexport interface Options {\n  wait: number;\n}\nexport default class Timer {\n}\nfunction helper() {}\nexport type Id = string;\n",
        );
        put(
            r,
            "cmd/main.go",
            "package main\n\ntype Server struct {\n\tport int\n}\n\nfunc (s *Server) Start() error {\n\treturn nil\n}\n\nfunc main() {\n}\n",
        );
        put(r, "web/styles.css", "a {}\n");
        put(r, "web/data.json", "{}\n");
        put(
            r,
            "node_modules/left-pad/index.js",
            "export function leftPad() {}\n",
        );
        put(r, "target/debug/app.rs", "pub fn built() {}\n");
        put(r, ".hidden/secret.rs", "pub fn hidden() {}\n");
        put(r, ".gitignore", "generated/\n");
        put(r, "generated/out.rs", "pub fn generated() {}\n");
        std::fs::write(r.join("src/blob.rs"), b"pub fn x() {}\0\0\0binary").unwrap();
        d
    }

    #[test]
    fn the_map_names_the_folders_and_the_signatures_and_skips_what_is_not_the_projects() {
        let d = project();
        let map = build(d.path(), DEFAULT_CHARS).unwrap();
        let name = d.path().file_name().unwrap().to_str().unwrap();
        let first = map.lines().next().unwrap();
        assert!(
            first.starts_with(&format!("{name}/ · 10 files · ")),
            "{first}"
        );
        assert!(first.contains("Rust 3"), "{first}");
        assert_eq!(map.lines().nth(1).unwrap(), "Cargo.toml  README.md");
        assert_eq!(map.lines().nth(2).unwrap(), "  build.rs: fn main()");
        for s in [
            "src/ · 2 files",
            "  lib.rs: pub struct Agent · impl Agent · pub fn new(x: u32) -> Self · pub async fn turn(&mut self, text: &str) -> Result<TurnResult, Error> · impl std::fmt::Display for Agent · pub(crate) const CAP: usize · pub enum Mode",
            "  blob.rs",
            "tools/ · 1 file",
            "  app.py: class Runner(Base) · Runner.__init__ · Runner.run · def main(argv=None) · async def fetch(url,…)",
            "web/ · 3 files",
            "  index.ts: export function debounce<T>(fn: T, wait: number): T · export const limit · export interface Options · export default class Timer · function helper() · export type Id",
            "  styles.css",
            "  +1 data file",
            "cmd/ · 1 file",
            "  main.go: type Server struct · func (s *Server) Start() error · func main()",
        ] {
            assert!(map.contains(s), "missing {s:?} in:\n{map}");
        }
        for absent in [
            "hidden",
            "leftPad",
            "built",
            "generated",
            "fn private",
            "fn hidden",
            "secret = 1",
            "__repr__",
            "node_modules",
            "target/",
        ] {
            assert!(!map.contains(absent), "{absent:?} in:\n{map}");
        }
        // Deterministic: the same project maps the same.
        assert_eq!(build(d.path(), DEFAULT_CHARS).unwrap(), map);
    }

    #[test]
    fn a_small_budget_keeps_the_top_and_says_what_was_left() {
        let d = TempDir::new().unwrap();
        for i in 0..40 {
            put(
                d.path(),
                &format!("crates/c{i:02}/src/lib.rs"),
                "pub fn f() {}\n",
            );
            put(
                d.path(),
                &format!("crates/c{i:02}/Cargo.toml"),
                "[package]\n",
            );
        }
        put(d.path(), "Cargo.toml", "[workspace]\n");
        let map = build(d.path(), 1_200).unwrap();
        assert!(map.len() <= 1_400, "{}", map.len());
        assert!(map.contains("crates/c00/ · 1 file"), "{map}");
        let last = map.lines().last().unwrap();
        assert!(
            last.starts_with("… ") && last.ends_with("not shown; glob and grep find them."),
            "{last}"
        );
        let whole = build(d.path(), 100_000).unwrap();
        assert!(!whole.contains("not shown"), "{whole}");
        assert!(
            whole.contains("crates/c39/src/ · 1 file\n  lib.rs: pub fn f()"),
            "{whole}"
        );
    }

    #[test]
    fn a_root_of_source_files_alone_has_no_empty_line() {
        let d = TempDir::new().unwrap();
        put(d.path(), "app.py", "def add(a, b):\n    return a + b\n");
        let map = build(d.path(), DEFAULT_CHARS).unwrap();
        let lines: Vec<&str> = map.lines().collect();
        assert_eq!(lines.len(), 2, "{map}");
        assert_eq!(lines[1], "  app.py: def add(a, b)");
    }

    #[test]
    fn nothing_maps_to_nothing() {
        let d = TempDir::new().unwrap();
        assert_eq!(build(d.path(), DEFAULT_CHARS), None);
        assert_eq!(build(&d.path().join("missing"), DEFAULT_CHARS), None);
        put(d.path(), "a.rs", "pub fn a() {}\n");
        assert_eq!(build(d.path(), 0), None);
    }

    #[test]
    fn signatures_are_cut_at_their_bodies_and_their_length() {
        assert_eq!(
            rust("pub fn f(a: u32) -> u32 { a }"),
            vec!["pub fn f(a: u32) -> u32"]
        );
        assert_eq!(
            rust("pub type Result<T> = std::result::Result<T, Error>;"),
            vec!["pub type Result<T>"]
        );
        assert_eq!(
            rust("impl<T: Clone> From<T> for Wrap<T> where T: Send {"),
            vec!["impl<T: Clone> From<T> for Wrap<T>"]
        );
        let long = format!("pub fn {}() {{}}", "x".repeat(200));
        let sig = &rust(&long)[0];
        assert_eq!(sig.chars().count(), SIGNATURE_CHARS);
        assert!(sig.ends_with('…'));
        assert_eq!(
            typescript("export const handler = async (req: Request): Promise<Response> => {"),
            vec!["export const handler"]
        );
        assert_eq!(
            go("func New(port int) (*Server, error) {"),
            vec!["func New(port int) (*Server, error)"]
        );
    }
}
