//! Decisions: `.ryter/decisions.md` in the project.
//!
//! A plan the user approved stays as they approved it. Where the work ends
//! up differing from it, on their say-so or because the plan could not be
//! followed as written, the difference and its reason are recorded here,
//! under the plan they belong to. A review reads them, so a difference that
//! was decided is not reported as a defect.
//!
//! The choices made in the work are recorded here too, under `## work`: a
//! library, a data model, a structure, a workaround, picked among others,
//! with the reason. They used to live in the chat alone, so the next
//! session, or the next hat, did not know them and redid or undid them.
//! Every hat is shown the file at the start of a session.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// The file, under the project's top.
pub const FILE: &str = ".ryter/decisions.md";

/// The most one line of an entry may be. An entry is a reason, not a report.
const MAX_LINE: usize = 400;

const HEADER: &str = "# Decisions\n\n\
Where the work differs from a plan you approved, and the choices made in the work, \
each with its reason. Ryter adds an entry when one is decided; every hat reads them at \
the start of a session, and an audit reads them. Remove an entry you don't agree with.\n";

/// The heading the work's own choices are filed under.
pub const WORK: &str = "## work";

/// The most of the file a prompt carries.
pub const SHOWN_CHARS: usize = 6_000;

/// One difference from the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// A few words: what was decided.
    pub title: String,
    /// What the plan says.
    pub plan_said: String,
    /// What was built instead.
    pub built_instead: String,
    /// The reason.
    pub why: String,
    /// Who decided: `you`, or the hat and its model.
    pub by: String,
}

/// `text` as one line of an entry: no line breaks, no heading marks at the
/// front, and no longer than a line should be.
fn line(text: &str) -> String {
    let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let one = one.trim_start_matches(['#', '-', ' ']).trim();
    if one.chars().count() <= MAX_LINE {
        return one.to_string();
    }
    let mut cut: String = one.chars().take(MAX_LINE - 1).collect();
    cut.push('…');
    cut
}

/// The name a plan's entries are filed under: its file's name.
fn plan_name(plan_file: &str) -> &str {
    plan_file.rsplit('/').next().unwrap_or(plan_file)
}

fn heading(plan_file: &str) -> String {
    format!("## plan: {}", plan_name(plan_file))
}

/// A choice made in the work, filed under [`WORK`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// A few words: what was decided.
    pub title: String,
    /// What was chosen, and over what.
    pub chose: String,
    /// The reason.
    pub why: String,
    /// Who decided: `you`, or the hat and its model.
    pub by: String,
}

/// The lines of `text` that are the section for `plan_file`: the index after
/// its heading, and the index of the next plan's heading (or the end).
fn section(lines: &[&str], plan_file: &str) -> Option<(usize, usize)> {
    section_under(lines, &heading(plan_file))
}

/// The lines of `text` under `head`: the index after it, and the index of
/// the next `## ` heading (or the end).
fn section_under(lines: &[&str], head: &str) -> Option<(usize, usize)> {
    let start = lines.iter().position(|l| l.trim_end() == head)? + 1;
    let end = lines[start..]
        .iter()
        .position(|l| l.starts_with("## "))
        .map_or(lines.len(), |i| start + i);
    Some((start, end))
}

/// Record `entry` against `plan_file` (a path in the project, as the session
/// holds it) at `stamp` (`YYYY-MM-DD HH:MM`). Returns the file.
pub fn record_at(root: &Path, plan_file: &str, entry: &Entry, stamp: &str) -> Result<PathBuf> {
    let path = root.join(FILE);
    let io = |e: std::io::Error| Error::Io(format!("{}: {e}", path.display()));
    let old = match crate::plan::read_own(root, FILE) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => HEADER.to_string(),
        Err(e) => return Err(io(e)),
    };
    let block = [
        format!("### {}", line(&entry.title)),
        format!("- Plan said: {}", line(&entry.plan_said)),
        format!("- Built instead: {}", line(&entry.built_instead)),
        format!("- Why: {}", line(&entry.why)),
        format!("- Decided by: {} · {stamp}", line(&entry.by)),
    ];
    crate::plan::write_own(root, FILE, &with_entry(&old, &heading(plan_file), &block))
}

/// `old` with `block` added at the end of the section under `head`, which
/// is made at the end of the file if it isn't there yet.
fn with_entry(old: &str, head: &str, block: &[String]) -> String {
    let lines: Vec<&str> = old.lines().collect();
    let (mut before, after): (Vec<String>, Vec<String>) = match section_under(&lines, head) {
        Some((_, end)) => (
            lines[..end].iter().map(|l| l.to_string()).collect(),
            lines[end..].iter().map(|l| l.to_string()).collect(),
        ),
        None => {
            let mut b: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
            while b.last().is_some_and(|l| l.trim().is_empty()) {
                b.pop();
            }
            b.push(String::new());
            b.push(head.to_string());
            (b, Vec::new())
        }
    };
    while before.last().is_some_and(|l| l.trim().is_empty()) {
        before.pop();
    }
    before.push(String::new());
    before.extend(block.iter().cloned());
    if !after.is_empty() {
        before.push(String::new());
        before.extend(after);
    }
    let mut text = before.join("\n");
    text.push('\n');
    text
}

/// Record a choice made in the work under [`WORK`], at `stamp`.
pub fn record_choice_at(root: &Path, choice: &Choice, stamp: &str) -> Result<PathBuf> {
    let path = root.join(FILE);
    let io = |e: std::io::Error| Error::Io(format!("{}: {e}", path.display()));
    let old = match crate::plan::read_own(root, FILE) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => HEADER.to_string(),
        Err(e) => return Err(io(e)),
    };
    let block = [
        format!("### {}", line(&choice.title)),
        format!("- Chose: {}", line(&choice.chose)),
        format!("- Why: {}", line(&choice.why)),
        format!("- Decided by: {} · {stamp}", line(&choice.by)),
    ];
    crate::plan::write_own(root, FILE, &with_entry(&old, WORK, &block))
}

/// [`record_choice_at`] now, in local time.
pub fn record_choice(root: &Path, choice: &Choice) -> Result<PathBuf> {
    record_choice_at(root, choice, &crate::clock::stamp())
}

/// How many choices are recorded under [`WORK`].
pub fn count_choices(root: &Path) -> usize {
    let Ok(text) = crate::plan::read_own(root, FILE) else {
        return 0;
    };
    let lines: Vec<&str> = text.lines().collect();
    section_under(&lines, WORK).map_or(0, |(start, end)| {
        lines[start..end]
            .iter()
            .filter(|l| l.starts_with("### "))
            .count()
    })
}

/// The file as a prompt carries it: whole when it fits `cap`, otherwise its
/// newest `cap` characters from an entry's start, with a line saying so.
/// `None` when there is no entry to show.
pub fn shown(root: &Path, cap: usize) -> Option<String> {
    let text = crate::plan::read_own(root, FILE).ok()?;
    if !text.contains("\n### ") {
        return None;
    }
    // From the first section on: the file's own title and the line under
    // it say what the prompt's introduction says.
    let text = text
        .find("\n## ")
        .map_or(text.as_str(), |i| &text[i + 1..])
        .trim_end();
    if text.len() <= cap {
        return Some(text.to_string());
    }
    let tail = &text[text.len() - cap..];
    let from = tail.find("\n### ").map_or(0, |i| i + 1);
    Some(format!(
        "…(earlier entries left out; the whole file is `{FILE}`)\n\n{}",
        tail[from..].trim_end()
    ))
}

/// What a reader of the work is told about the choices made in it:
/// nothing when there are none.
pub fn choices_pointer(root: &Path) -> Option<String> {
    let n = count_choices(root);
    (n > 0).then(|| {
        format!(
            "The choices made in the work ({n}) are under `{WORK}` in `{FILE}`, each with \
             its reason: a choice recorded there was decided, and is not a finding unless \
             its reason is wrong or it breaks something."
        )
    })
}

/// [`record_at`] now, in local time.
pub fn record(root: &Path, plan_file: &str, entry: &Entry) -> Result<PathBuf> {
    record_at(root, plan_file, entry, &crate::clock::stamp())
}

/// How many decisions are recorded against `plan_file`.
pub fn count_for(root: &Path, plan_file: &str) -> usize {
    let Ok(text) = crate::plan::read_own(root, FILE) else {
        return 0;
    };
    let lines: Vec<&str> = text.lines().collect();
    section(&lines, plan_file).map_or(0, |(start, end)| {
        lines[start..end]
            .iter()
            .filter(|l| l.starts_with("### "))
            .count()
    })
}

/// What a reader of the work is told about the decisions for `plan_file`:
/// nothing when there are none.
pub fn pointer(root: &Path, plan_file: &str) -> Option<String> {
    let n = count_for(root, plan_file);
    (n > 0).then(|| {
        format!(
            "Where the work differs from that plan on purpose is recorded in `{FILE}`, \
             under `{}` ({n} {}). Read {}: a difference explained there was decided, and \
             is not a finding unless its reason is wrong or the change breaks something.",
            heading(plan_file),
            if n == 1 { "entry" } else { "entries" },
            if n == 1 { "it" } else { "them" },
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const CMS: &str = ".ryter/plans/2026-10-01-cms.md";

    fn entry(title: &str) -> Entry {
        Entry {
            title: title.into(),
            plan_said: "step 4, an Export button on the page list".into(),
            built_instead: "no export".into(),
            why: "you said \"skip the export button for now\"".into(),
            by: "you".into(),
        }
    }

    #[test]
    fn a_decision_is_filed_under_its_plan() {
        let root = TempDir::new().unwrap();
        let path = record_at(
            root.path(),
            CMS,
            &entry("No export button in this pass"),
            "2026-10-01 14:20",
        )
        .unwrap();
        assert_eq!(path, root.path().join(".ryter/decisions.md"));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.starts_with("# Decisions\n\nWhere the work differs"),
            "{text}"
        );
        assert!(
            text.ends_with(
                "\n## plan: 2026-10-01-cms.md\n\n\
                 ### No export button in this pass\n\
                 - Plan said: step 4, an Export button on the page list\n\
                 - Built instead: no export\n\
                 - Why: you said \"skip the export button for now\"\n\
                 - Decided by: you · 2026-10-01 14:20\n"
            ),
            "{text}"
        );
        assert_eq!(count_for(root.path(), CMS), 1);
    }

    /// A second decision for the same plan goes under the same heading; one
    /// for another plan gets its own; and one more for the first plan goes
    /// back under the first, not at the end of the file.
    #[test]
    fn each_plan_keeps_its_own_decisions_together() {
        let root = TempDir::new().unwrap();
        let other = ".ryter/plans/2026-10-02-search.md";
        for (plan, title) in [(CMS, "one"), (CMS, "two"), (other, "three"), (CMS, "four")] {
            record_at(root.path(), plan, &entry(title), "2026-10-01 14:20").unwrap();
        }
        let text = std::fs::read_to_string(root.path().join(FILE)).unwrap();
        let order: Vec<&str> = text
            .lines()
            .filter(|l| l.starts_with("## ") || l.starts_with("### "))
            .collect();
        assert_eq!(
            order,
            [
                "## plan: 2026-10-01-cms.md",
                "### one",
                "### two",
                "### four",
                "## plan: 2026-10-02-search.md",
                "### three",
            ],
            "{text}"
        );
        assert_eq!(text.matches("## plan: 2026-10-01-cms.md").count(), 1);
        assert_eq!(count_for(root.path(), CMS), 3);
        assert_eq!(count_for(root.path(), other), 1);
        assert_eq!(count_for(root.path(), ".ryter/plans/none.md"), 0);
        // One blank line between blocks, none doubled.
        assert!(!text.contains("\n\n\n"), "{text}");
    }

    /// What the user wrote in the file by hand is kept as it is.
    #[test]
    fn the_users_own_words_in_the_file_are_kept() {
        let root = TempDir::new().unwrap();
        let path = root.path().join(FILE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "# Decisions\n\nMy own note.\n\n## plan: 2026-10-01-cms.md\n\n### mine\n- Why: because\n",
        )
        .unwrap();
        record_at(root.path(), CMS, &entry("theirs"), "2026-10-01 14:20").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("My own note.\n"), "{text}");
        assert!(
            text.contains("### mine\n- Why: because\n\n### theirs\n"),
            "{text}"
        );
        assert_eq!(count_for(root.path(), CMS), 2);
    }

    /// An entry can't break the file's shape: no headings of its own, no
    /// line breaks, and no essay.
    #[test]
    fn an_entry_stays_one_entry() {
        let root = TempDir::new().unwrap();
        let mut e = entry("## plan: other.md\n### fake");
        e.why = format!("first line\nsecond line {}", "x".repeat(900));
        record_at(root.path(), CMS, &e, "2026-10-01 14:20").unwrap();
        let text = std::fs::read_to_string(root.path().join(FILE)).unwrap();
        assert_eq!(text.matches("\n## ").count(), 1, "{text}");
        assert_eq!(text.matches("\n### ").count(), 1, "{text}");
        assert!(text.contains("### plan: other.md ### fake\n"), "{text}");
        let why = text.lines().find(|l| l.starts_with("- Why: ")).unwrap();
        assert!(
            why.starts_with("- Why: first line second line xxx"),
            "{why}"
        );
        assert!(
            why.ends_with('…') && why.chars().count() <= 7 + MAX_LINE,
            "{why}"
        );
    }

    #[test]
    fn a_choice_is_filed_under_work_beside_the_plans_entries() {
        let root = TempDir::new().unwrap();
        let choice = Choice {
            title: "SQLite through better-sqlite3".into(),
            chose: "better-sqlite3 over an ORM".into(),
            why: "one file, synchronous, nothing to configure".into(),
            by: "build hat (grok-4.6)".into(),
        };
        record_choice_at(root.path(), &choice, "2026-10-01 14:30").unwrap();
        record_at(
            root.path(),
            CMS,
            &entry("No export button"),
            "2026-10-01 14:40",
        )
        .unwrap();
        let second = Choice {
            title: "Dates as ISO strings".into(),
            chose: "ISO-8601 text columns over epoch integers".into(),
            why: "readable in the database and sorted by the collation".into(),
            by: "you".into(),
        };
        record_choice_at(root.path(), &second, "2026-10-01 14:50").unwrap();
        let text = std::fs::read_to_string(root.path().join(FILE)).unwrap();
        assert!(
            text.starts_with("# Decisions\n\nWhere the work differs"),
            "{text}"
        );
        let work = text.find("## work").unwrap();
        let plan = text.find("## plan: 2026-10-01-cms.md").unwrap();
        assert!(work < plan, "{text}");
        assert!(
            text[work..plan].contains(
                "### SQLite through better-sqlite3\n- Chose: better-sqlite3 over an ORM\n\
                 - Why: one file, synchronous, nothing to configure\n\
                 - Decided by: build hat (grok-4.6) · 2026-10-01 14:30\n\n\
                 ### Dates as ISO strings\n"
            ),
            "{text}"
        );
        assert_eq!(count_choices(root.path()), 2);
        assert_eq!(count_for(root.path(), CMS), 1);
        assert!(
            choices_pointer(root.path())
                .unwrap()
                .starts_with("The choices made in the work (2)")
        );
        assert_eq!(choices_pointer(TempDir::new().unwrap().path()), None);
        // Shown whole when it fits, newest entries when it does not.
        let whole = shown(root.path(), 10_000).unwrap();
        assert!(whole.starts_with("## work\n\n### SQLite"), "{whole}");
        assert!(
            whole.ends_with("- Decided by: you · 2026-10-01 14:40"),
            "{whole}"
        );
        let cut = shown(root.path(), 260).unwrap();
        assert!(cut.starts_with("…(earlier entries left out"), "{cut}");
        assert!(cut.contains("### No export button"), "{cut}");
        assert!(!cut.contains("SQLite"), "{cut}");
        assert_eq!(shown(TempDir::new().unwrap().path(), 10_000), None);
    }

    #[test]
    fn a_reader_is_pointed_at_the_decisions_only_when_there_are_some() {
        let root = TempDir::new().unwrap();
        assert_eq!(pointer(root.path(), CMS), None);
        record_at(root.path(), CMS, &entry("one"), "2026-10-01 14:20").unwrap();
        let p = pointer(root.path(), CMS).unwrap();
        assert!(
            p.contains(
                "`.ryter/decisions.md`, under `## plan: 2026-10-01-cms.md` (1 entry). Read it:"
            ),
            "{p}"
        );
        record_at(root.path(), CMS, &entry("two"), "2026-10-01 14:20").unwrap();
        let p = pointer(root.path(), CMS).unwrap();
        assert!(p.contains("(2 entries). Read them:"), "{p}");
    }
}
