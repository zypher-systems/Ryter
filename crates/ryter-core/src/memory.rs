//! Project memory: `ROADMAP.md`, `DECISIONS.md`, and `notes/*.md`.

use std::fs;
use std::path::Path;

const MEMORY_CAP: usize = 48_000;

/// Roadmap + decisions + `notes/*.md` for prompts. Missing files are skipped.
pub fn load_project_memory(project_root: Option<&Path>) -> Option<String> {
    let root = project_root?;
    let mut out = String::new();
    append_file(&mut out, root, "ROADMAP.md");
    append_file(&mut out, root, "DECISIONS.md");
    if let Ok(rd) = fs::read_dir(root.join("notes")) {
        // Keep only the first 128 Markdown names in sorted order. Scanning a
        // large notes directory must not allocate a path for every entry.
        let mut files = std::collections::BTreeSet::new();
        let mut limited = false;
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            files.insert(path);
            if files.len() > 128 {
                files.pop_last();
                limited = true;
            }
        }
        if limited {
            let note = "[memory limited to the first 128 note filenames]\n";
            if out.len() + note.len() <= MEMORY_CAP {
                out.push_str(note);
            }
        }
        for p in files {
            if out.len() >= MEMORY_CAP {
                break;
            }
            if p.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            append_file(&mut out, root, &format!("notes/{name}"));
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

fn append_file(out: &mut String, root: &Path, label: &str) {
    let header = format!("### {label}\n");
    let cap = MEMORY_CAP.saturating_sub(out.len() + header.len() + 2);
    if cap < crate::project_file::TRUNCATED.len() {
        return;
    }
    let Ok(body) = crate::project_file::read(root, Path::new(label), cap) else {
        return;
    };
    if body.trim().is_empty() {
        return;
    }
    out.push_str(&header);
    out.push_str(&body);
    if !body.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
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
    fn note_enumeration_keeps_a_bounded_sorted_set() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("notes")).unwrap();
        for i in (0..256).rev() {
            fs::write(
                dir.path().join(format!("notes/{i:03}.md")),
                format!("note-{i:03}"),
            )
            .unwrap();
        }
        let memory = load_project_memory(Some(dir.path())).unwrap();
        assert!(memory.contains("first 128 note filenames"));
        assert!(memory.contains("note-000") && memory.contains("note-127"));
        assert!(!memory.contains("note-128") && !memory.contains("note-255"));
    }

    #[test]
    fn memory_has_one_aggregate_byte_limit() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("notes")).unwrap();
        for file in ["ROADMAP.md", "DECISIONS.md", "notes/one.md", "notes/two.md"] {
            fs::write(dir.path().join(file), "é".repeat(8_000)).unwrap();
        }
        let memory = load_project_memory(Some(dir.path())).unwrap();
        assert!(memory.len() <= MEMORY_CAP);
        assert!(memory.contains("ROADMAP.md") && memory.contains("DECISIONS.md"));
        assert!(memory.contains("truncated"));
    }

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
