//! Project memory: `ROADMAP.md`, `DECISIONS.md`, and `notes/*.md`.
//!
//! The prompt gets at most [`MEMORY_CAP`] bytes of them, shared out: a
//! third to each of the three sources, a source that needs less leaving
//! the rest to the others, and a roadmap that does not fit giving up its
//! Done section first. What was cut is named at the end, so the model
//! knows what to `read` for the rest. Appending the files in order into
//! one cap used to let a roadmap of 100 KB fill it alone, and the
//! decisions and the notes never reached the prompt.

use std::fs;
use std::path::Path;

use crate::project_file::TRUNCATED;

const MEMORY_CAP: usize = 48_000;
/// A file is read whole, up to this, before it is cut to its share.
const WHOLE: usize = 1 << 20;
/// Notes named one by one in the cut line, before "and N more".
const NAMED: usize = 5;
/// Kept back from the shares for the cut line at the end, which is held
/// to this length, so the whole never passes [`MEMORY_CAP`].
const RESERVE: usize = 512;

/// Roadmap + decisions + `notes/*.md` for prompts. Missing files are skipped.
pub fn load_project_memory(project_root: Option<&Path>) -> Option<String> {
    let root = project_root?;
    let roadmap = read_whole(root, "ROADMAP.md").map(done_last);
    let decisions = read_whole(root, "DECISIONS.md");
    let (notes, limited) = note_files(root);
    let notes: Vec<(String, String)> = notes
        .into_iter()
        .filter_map(|name| {
            let label = format!("notes/{name}");
            read_whole(root, &label).map(|body| (label, body))
        })
        .collect();
    let needs = [
        roadmap.as_ref().map_or(0, |b| entry("ROADMAP.md", b).len()),
        decisions
            .as_ref()
            .map_or(0, |b| entry("DECISIONS.md", b).len()),
        notes.iter().map(|(l, b)| entry(l, b).len()).sum::<usize>()
            + if limited { LIMITED.len() } else { 0 },
    ];
    // The notes take what fits of their share, whole notes in order; what
    // they leave of it goes to the roadmap and the decisions.
    let first = allot(MEMORY_CAP - RESERVE, &needs);
    let (kept, omitted, used) = fit_notes(&notes, limited, first[2]);
    let shares = allot(MEMORY_CAP - RESERVE, &[needs[0], needs[1], used]);
    let mut out = String::new();
    let mut cut: Vec<String> = Vec::new();
    push_entry(&mut out, "ROADMAP.md", roadmap, shares[0], true, &mut cut);
    push_entry(
        &mut out,
        "DECISIONS.md",
        decisions,
        shares[1],
        false,
        &mut cut,
    );
    for text in kept {
        out.push_str(&text);
    }
    if !omitted.is_empty() {
        let shown: Vec<&str> = omitted.iter().take(NAMED).map(String::as_str).collect();
        let more = omitted.len().saturating_sub(NAMED);
        cut.push(format!(
            "notes: {} omitted ({}{})",
            omitted.len(),
            shown.join(", "),
            if more > 0 {
                format!(" and {more} more")
            } else {
                String::new()
            }
        ));
    }
    if !cut.is_empty() {
        let line = format!(
            "[memory cut to fit: {}. `read` a file for the rest.]\n",
            cut.join("; ")
        );
        out.push_str(&cut_to(line, RESERVE));
    }
    if out.is_empty() { None } else { Some(out) }
}

const LIMITED: &str = "[memory limited to the first 128 note filenames]\n";

/// The notes that fit `share` whole, in order (the limit line first when
/// there is one), the labels of those that do not, and the bytes taken.
fn fit_notes(
    notes: &[(String, String)],
    limited: bool,
    share: usize,
) -> (Vec<String>, Vec<String>, usize) {
    let mut kept = Vec::new();
    let mut omitted = Vec::new();
    let mut used = 0;
    if limited && LIMITED.len() <= share {
        kept.push(LIMITED.to_string());
        used += LIMITED.len();
    }
    for (label, body) in notes {
        let text = entry(label, body);
        if used + text.len() <= share {
            used += text.len();
            kept.push(text);
        } else {
            omitted.push(label.clone());
        }
    }
    (kept, omitted, used)
}

/// The first 128 Markdown names under `notes/`, sorted, and whether there
/// were more. Scanning a large notes directory must not allocate a path
/// for every entry.
fn note_files(root: &Path) -> (Vec<String>, bool) {
    let Ok(rd) = fs::read_dir(root.join("notes")) else {
        return (Vec::new(), false);
    };
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
    let names = files
        .into_iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    (names, limited)
}

/// A file read whole (up to [`WHOLE`]); `None` when missing, unreadable
/// or blank.
fn read_whole(root: &Path, label: &str) -> Option<String> {
    let body = crate::project_file::read(root, Path::new(label), WHOLE).ok()?;
    if body.trim().is_empty() {
        return None;
    }
    Some(body)
}

/// A file's entry in the memory: its header, its body, a blank line.
fn entry(label: &str, body: &str) -> String {
    let mut s = format!("### {label}\n");
    s.push_str(body);
    if !body.ends_with('\n') {
        s.push('\n');
    }
    s.push('\n');
    s
}

/// Share `cap` out among sources that need `needs` bytes: each gets up to
/// a third, and what a source leaves of its third goes to the ones still
/// short, in order.
fn allot(cap: usize, needs: &[usize; 3]) -> [usize; 3] {
    let share = cap / needs.len();
    let mut got = [0; 3];
    let mut left = cap;
    for (g, need) in got.iter_mut().zip(needs) {
        *g = (*need).min(share);
        left -= *g;
    }
    for (g, need) in got.iter_mut().zip(needs) {
        let more = need.saturating_sub(*g).min(left);
        *g += more;
        left -= more;
    }
    got
}

/// Append `body` under its header, cut to `share` bytes with the marker
/// when it does not fit, and say so in `cut`. A share too small for the
/// marker omits the file, and says that instead.
fn push_entry(
    out: &mut String,
    label: &str,
    body: Option<String>,
    share: usize,
    done_last: bool,
    cut: &mut Vec<String>,
) {
    let Some(body) = body else {
        return;
    };
    let whole = entry(label, &body).len();
    if whole <= share {
        out.push_str(&entry(label, &body));
        return;
    }
    let header = format!("### {label}\n");
    let room = share.saturating_sub(header.len() + 2);
    if room < TRUNCATED.len() + 1 {
        cut.push(format!("{label} omitted ({} bytes)", body.len()));
        return;
    }
    let kept = cut_to(body.clone(), room);
    out.push_str(&entry(label, &kept));
    cut.push(format!(
        "{label} {} of {} bytes{}",
        kept.len(),
        body.len(),
        if done_last {
            " (its Done section last)"
        } else {
            ""
        }
    ));
}

/// `body` cut to `cap` bytes on a character boundary, ending in the
/// marker. `body` is returned whole when it fits.
fn cut_to(mut body: String, cap: usize) -> String {
    if body.len() <= cap {
        return body;
    }
    let mut end = cap.saturating_sub(TRUNCATED.len());
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    body.truncate(end);
    body.push_str(TRUNCATED);
    body
}

/// The roadmap with its `## Done` section(s) moved to the end, so that a
/// cut takes the history before Now, Next, Later and Blocked. A roadmap
/// without one is returned as it is.
fn done_last(body: String) -> String {
    let mut starts: Vec<usize> = body.match_indices("\n## ").map(|(i, _)| i + 1).collect();
    if body.starts_with("## ") {
        starts.insert(0, 0);
    }
    let Some(&first) = starts.first() else {
        return body;
    };
    let mut sections: Vec<&str> = starts.windows(2).map(|w| &body[w[0]..w[1]]).collect();
    sections.push(&body[*starts.last().unwrap_or(&first)..]);
    let is_done = |s: &str| {
        s.trim_start_matches("## ")
            .split_whitespace()
            .next()
            .is_some_and(|w| w.eq_ignore_ascii_case("done"))
    };
    if !sections.iter().any(|s| is_done(s)) {
        return body;
    }
    let mut out = String::with_capacity(body.len());
    out.push_str(&body[..first]);
    for s in sections.iter().filter(|s| !is_done(s)) {
        out.push_str(s);
    }
    for s in sections.iter().filter(|s| is_done(s)) {
        out.push_str(s);
    }
    out
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
        // Four files of 16 KB into 48 KB: the notes are left out whole and
        // what they leave goes to the roadmap and the decisions.
        assert!(
            memory.contains("notes: 2 omitted (notes/one.md, notes/two.md)"),
            "{}",
            memory.lines().last().unwrap()
        );
    }

    /// Each source has a share: a roadmap larger than the whole cap no
    /// longer crowds the decisions and the notes out, the cut is named so
    /// the model knows what to read, and multibyte text is cut on a
    /// character boundary.
    #[test]
    fn each_source_gets_its_share_and_the_cut_is_named() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("notes")).unwrap();
        let roadmap = format!(
            "# Roadmap\n\n## Now\nnow-row\n\n## Done\n{}\n",
            "é".repeat(40_000)
        );
        fs::write(dir.path().join("ROADMAP.md"), &roadmap).unwrap();
        fs::write(
            dir.path().join("DECISIONS.md"),
            format!("# Decisions\n\n### why\n{}\n", "ü".repeat(40_000)),
        )
        .unwrap();
        fs::write(dir.path().join("notes/a.md"), "note-a\n").unwrap();
        fs::write(dir.path().join("notes/b.md"), "ö".repeat(20_000)).unwrap();
        fs::write(dir.path().join("notes/c.md"), "note-c\n").unwrap();
        let memory = load_project_memory(Some(dir.path())).unwrap();
        assert!(memory.len() <= MEMORY_CAP, "{}", memory.len());
        assert!(memory.contains("### ROADMAP.md\n# Roadmap\n\n## Now\nnow-row\n"));
        assert!(memory.contains("### DECISIONS.md\n# Decisions\n\n### why\n"));
        assert!(memory.contains("### notes/a.md\nnote-a\n"));
        assert!(memory.contains("### notes/c.md\nnote-c\n"));
        let tail = memory.lines().last().unwrap();
        assert!(
            tail.starts_with("[memory cut to fit: ROADMAP.md "),
            "{tail}"
        );
        assert!(
            tail.contains(&format!(
                "of {} bytes (its Done section last); DECISIONS.md ",
                roadmap.len()
            )),
            "{tail}"
        );
        assert!(
            tail.ends_with("notes: 1 omitted (notes/b.md). `read` a file for the rest.]"),
            "{tail}"
        );
        assert!(std::str::from_utf8(memory.as_bytes()).is_ok());
    }

    /// A source that needs less than its third leaves the rest to the
    /// others: a small roadmap and no notes let the decisions have nearly
    /// the whole cap.
    #[test]
    fn a_small_source_leaves_its_share_to_the_others() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("ROADMAP.md"), "# Roadmap\n## Now\nx\n").unwrap();
        fs::write(dir.path().join("DECISIONS.md"), "d".repeat(100_000)).unwrap();
        let memory = load_project_memory(Some(dir.path())).unwrap();
        assert!(memory.len() <= MEMORY_CAP);
        assert!(
            memory.len() > MEMORY_CAP - RESERVE - 200,
            "{}",
            memory.len()
        );
        assert!(memory.contains("### ROADMAP.md\n# Roadmap\n## Now\nx\n"));
        assert!(
            memory.contains("[memory cut to fit: DECISIONS.md 47"),
            "{}",
            memory.lines().last().unwrap()
        );
        assert!(!memory.contains("ROADMAP.md "), "the roadmap was not cut");
    }

    /// A note that does not fit is left out whole, and what the notes
    /// leave of their share goes to the others: a roadmap of 100 KB
    /// beside one 40 KB note gets nearly the whole cap.
    #[test]
    fn a_note_left_out_leaves_its_share_to_the_others() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("notes")).unwrap();
        fs::write(
            dir.path().join("ROADMAP.md"),
            format!("## Now\n{}", "r".repeat(100_000)),
        )
        .unwrap();
        fs::write(dir.path().join("notes/big.md"), "n".repeat(40_000)).unwrap();
        let memory = load_project_memory(Some(dir.path())).unwrap();
        assert!(memory.len() <= MEMORY_CAP);
        assert!(
            memory.len() > MEMORY_CAP - RESERVE - 200,
            "{}",
            memory.len()
        );
        assert!(!memory.contains("### notes/big.md"));
        assert!(
            memory.contains("notes: 1 omitted (notes/big.md)"),
            "{memory}"
        );
    }

    /// A roadmap that does not fit gives up its Done section first: Now,
    /// Next and Blocked come whole, the history is what the cut takes.
    #[test]
    fn the_roadmaps_done_section_is_cut_first() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("ROADMAP.md"),
            format!(
                "# Roadmap\n\n## Now\nnow-row\n\n## Next\nnext-row\n\n## Done\n{}\n\n## Blocked\nblocked-row\n",
                "h".repeat(60_000)
            ),
        )
        .unwrap();
        let memory = load_project_memory(Some(dir.path())).unwrap();
        let i = |s: &str| memory.find(s).unwrap_or_else(|| panic!("{s} missing"));
        assert!(i("now-row") < i("next-row") && i("next-row") < i("blocked-row"));
        assert!(i("blocked-row") < i("## Done"), "Done comes last");
        assert!(memory.contains("…(truncated)"));
        // Without a Done section the roadmap is left in its own order.
        assert_eq!(done_last("## A\na\n## B\nb\n".into()), "## A\na\n## B\nb\n");
        assert_eq!(
            done_last("intro\n## Done\nd\n## Now\nn\n".into()),
            "intro\n## Now\nn\n## Done\nd\n"
        );
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
        assert!(!mem.contains("[memory cut"), "nothing to cut: {mem}");
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
