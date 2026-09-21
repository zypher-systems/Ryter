//! Project memory: `ROADMAP.md`, `DECISIONS.md`, and `notes/*.md`.

use std::fs;
use std::path::Path;

const ROADMAP_TEMPLATE: &str = "\
# Roadmap

Living plan for this repo. The lead updates this as work lands.
Do not paste full chat logs here.

## Now

- (what we are doing this week)

## Next

- (queued after Now)

## Later

- (ideas, not committed)

## Done

- (shipped; one line each)

## Blocked

- (waiting on a decision or a person)
";

const DECISIONS_TEMPLATE: &str = "\
# Decisions

Why, not what. The lead records non-obvious choices here, its own and the crew's.
The lead reads this when the user asks why. Do not dump transcripts.

## Template

### YYYY-MM-DD — short title
- **By:** lead | architect | builder | auditor
- **Decision:** …
- **Chosen vs rejected:** …
- **Why:** …
- **Where:** `path` / symbol
- **Residual risk:** …
";

const MEMORY_CAP: usize = 48_000;

/// Create `ROADMAP.md` and `DECISIONS.md` if they are missing.
pub fn ensure_project_memory(project_root: &Path) -> std::io::Result<()> {
    let roadmap = project_root.join("ROADMAP.md");
    if !roadmap.exists() {
        fs::write(&roadmap, ROADMAP_TEMPLATE)?;
    }
    let decisions = project_root.join("DECISIONS.md");
    if !decisions.exists() {
        fs::write(&decisions, DECISIONS_TEMPLATE)?;
    }
    let notes = project_root.join("notes");
    if !notes.exists() {
        fs::create_dir_all(&notes)?;
    }
    Ok(())
}

/// Roadmap + decisions + `notes/*.md` for prompts. Missing files are skipped.
pub fn load_project_memory(project_root: Option<&Path>) -> Option<String> {
    let root = project_root?;
    let mut out = String::new();
    append_file(&mut out, "ROADMAP.md", &root.join("ROADMAP.md"));
    append_file(&mut out, "DECISIONS.md", &root.join("DECISIONS.md"));
    if let Ok(rd) = fs::read_dir(root.join("notes")) {
        let mut files: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        files.sort();
        for p in files {
            if p.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            append_file(&mut out, &format!("notes/{name}"), &p);
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Relevant project memory for a builder or auditor.
///
/// Every specialist used to receive all of ROADMAP.md, DECISIONS.md, and every
/// note on every round — about 9k tokens on this repository, and growing with
/// each decision recorded. A builder needs the current design and the
/// decisions about the files it owns; it does not write memory and does not
/// need the roadmap.
pub fn load_scoped_memory(project_root: Option<&Path>, scope: &[String]) -> Option<String> {
    let root = project_root?;
    let mut out = String::new();
    append_file(
        &mut out,
        "notes/architect.md (current design)",
        &root.join("notes/architect.md"),
    );
    let decisions = fs::read_to_string(root.join("DECISIONS.md")).unwrap_or_default();
    let relevant = relevant_decisions(&decisions, scope);
    if !relevant.is_empty() {
        out.push_str("### DECISIONS.md (entries about the files you own)\n");
        out.push_str(&relevant);
        out.push('\n');
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Cap on relevant decisions shown to one specialist.
const SCOPED_DECISIONS_CAP: usize = 8_000;

/// DECISIONS.md entries that name one of `scope`'s paths or file names.
fn relevant_decisions(decisions: &str, scope: &[String]) -> String {
    let needles: Vec<String> = scope
        .iter()
        .flat_map(|p| {
            let p = p.trim_matches('/').to_string();
            let name = p.rsplit('/').next().unwrap_or(&p).to_string();
            // A bare name like `mod.rs` matches everything; keep the path only.
            let generic = matches!(
                name.as_str(),
                "mod.rs" | "lib.rs" | "main.rs" | "index.ts" | "__init__.py"
            );
            if name.len() >= 5 && name != p && !generic {
                vec![p, name]
            } else {
                vec![p]
            }
        })
        .filter(|n| !n.is_empty())
        .collect();
    if needles.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for entry in decisions.split("\n### ").skip(1) {
        if needles.iter().any(|n| entry.contains(n.as_str())) {
            let block = format!("### {}\n", entry.trim_end());
            if out.len() + block.len() > SCOPED_DECISIONS_CAP {
                out.push_str("…(more decisions touch these files; read DECISIONS.md)\n");
                break;
            }
            out.push_str(&block);
        }
    }
    out
}

fn append_file(out: &mut String, label: &str, path: &Path) {
    let Ok(raw) = fs::read_to_string(path) else {
        return;
    };
    if raw.trim().is_empty() {
        return;
    }
    let body = truncate(&raw);
    out.push_str("### ");
    out.push_str(label);
    out.push('\n');
    out.push_str(&body);
    if !body.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
}

fn truncate(s: &str) -> String {
    if s.len() <= MEMORY_CAP {
        return s.to_string();
    }
    let mut cut = MEMORY_CAP;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}\n\n…(truncated)\n", &s[..cut])
}

/// Workspace-relative files the orchestrator may write (not product source).
pub fn is_memory_file(workspace: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(workspace) else {
        return false;
    };
    let name = rel
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let parent = rel.parent();
    let at_root = parent.is_none_or(|p| p.as_os_str().is_empty());
    if at_root && (name == "roadmap.md" || name == "decisions.md") {
        return true;
    }
    parent.is_some_and(|p| p == Path::new("notes")) && name.ends_with(".md")
}

#[cfg(test)]
mod scoped_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn builders_see_the_design_and_their_decisions_only() {
        let d = TempDir::new().unwrap();
        fs::write(d.path().join("ROADMAP.md"), "# Roadmap\nNow: lots\n").unwrap();
        fs::write(
            d.path().join("DECISIONS.md"),
            "# Decisions\n\n### A — parser\n- **Where:** `src/parse.rs`\n\n### B — tui\n- **Where:** `src/tui/draw.rs`\n",
        )
        .unwrap();
        fs::create_dir_all(d.path().join("notes")).unwrap();
        fs::write(d.path().join("notes/architect.md"), "shape: parser first").unwrap();
        let m = load_scoped_memory(Some(d.path()), &["src/parse.rs".into()]).unwrap();
        assert!(m.contains("shape: parser first"));
        assert!(m.contains("A — parser"));
        assert!(!m.contains("B — tui"), "{m}");
        assert!(!m.contains("Roadmap"), "builders do not need the roadmap");
    }

    #[test]
    fn a_generic_file_name_does_not_match_everything() {
        let decisions = "# D\n\n### A\nsrc/a/mod.rs\n\n### B\nsrc/b/mod.rs\n";
        let got = relevant_decisions(decisions, &["src/a/mod.rs".into()]);
        assert!(got.contains("### A") && !got.contains("### B"), "{got}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn ensure_creates_templates() {
        let dir = TempDir::new().unwrap();
        ensure_project_memory(dir.path()).unwrap();
        let road = fs::read_to_string(dir.path().join("ROADMAP.md")).unwrap();
        assert!(road.contains("## Now"));
        let dec = fs::read_to_string(dir.path().join("DECISIONS.md")).unwrap();
        assert!(dec.contains("**Why:**"));
        assert!(dir.path().join("notes").is_dir());
    }

    #[test]
    fn load_includes_roadmap_and_notes() {
        let dir = TempDir::new().unwrap();
        ensure_project_memory(dir.path()).unwrap();
        fs::write(dir.path().join("notes/plan.md"), "ship a CLI flag\n").unwrap();
        let mem = load_project_memory(Some(dir.path())).unwrap();
        assert!(mem.contains("ROADMAP.md"));
        assert!(mem.contains("notes/plan.md"));
        assert!(mem.contains("ship a CLI flag"));
    }

    #[test]
    fn memory_paths() {
        let root = Path::new("/tmp/proj");
        assert!(is_memory_file(root, Path::new("/tmp/proj/ROADMAP.md")));
        assert!(is_memory_file(root, Path::new("/tmp/proj/DECISIONS.md")));
        assert!(is_memory_file(
            root,
            Path::new("/tmp/proj/notes/architect.md")
        ));
        assert!(!is_memory_file(root, Path::new("/tmp/proj/src/lib.rs")));
        assert!(!is_memory_file(
            root,
            Path::new("/tmp/proj/docs/ROADMAP.md")
        ));
    }
}
