//! Project memory: `ROADMAP.md`, `DECISIONS.md`, and `notes/*.md`.

use std::fs;
use std::path::Path;

const MEMORY_CAP: usize = 48_000;

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

/// The project's memory files: `ROADMAP.md`, `DECISIONS.md`, `notes/*.md`.
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
    fn load_includes_roadmap_and_notes() {
        let dir = TempDir::new().unwrap();
        assert!(load_project_memory(Some(dir.path())).is_none());
        fs::write(dir.path().join("ROADMAP.md"), "# Roadmap\n## Now\n").unwrap();
        fs::create_dir_all(dir.path().join("notes")).unwrap();
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
