//! Project memory: `ROADMAP.md`, `DECISIONS.md`, and `notes/*.md`.

use std::fs;
use std::path::Path;

const ROADMAP_TEMPLATE: &str = "\
# Roadmap

Living plan for this repo. Orchestrator and specialists update this as work lands.
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

Why, not what. Each specialist appends when they make a non-obvious choice.
The orchestrator reads this when the user asks why. Do not dump transcripts.

## Template

### YYYY-MM-DD — short title
- **By:** planner | architect | builder | auditor | orchestrator
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
    let _ = ensure_project_memory(root);
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
