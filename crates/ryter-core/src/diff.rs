//! Line diffs of a file edit, for people to read.
//!
//! `write` and `search_replace` know the file before and after, so they
//! measure what changed here, with real line numbers, instead of leaving the
//! chat to guess from the model's arguments. The model never sees this: it
//! rides on the tool result event, not in the transcript.

use serde::{Deserialize, Serialize};
use similar::{Algorithm, ChangeTag, DiffOp, TextDiff};
use std::time::Duration;

/// Unchanged lines kept around each change.
pub const CONTEXT: usize = 2;
/// Lines kept across all hunks; the rest are counted in `elided`.
const MAX_LINES: usize = 400;
/// Characters kept per line.
const MAX_LINE_CHARS: usize = 400;
/// Files larger than this (either side) get counts only, no hunks.
const MAX_BYTES: usize = 2_000_000;
/// Myers gives up and falls back to a coarser diff after this long.
const DEADLINE: Duration = Duration::from_millis(250);

/// What one line of a hunk is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    /// Unchanged, shown for context.
    Context,
    /// Only in the new file.
    Added,
    /// Only in the old file.
    Removed,
}

/// One line of a hunk, with its number on each side it exists on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    /// Context, added, or removed.
    pub kind: LineKind,
    /// 1-based line in the old file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<usize>,
    /// 1-based line in the new file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new: Option<usize>,
    /// The line, without its line ending.
    pub text: String,
}

/// A run of changes and the context around them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hunk {
    /// Lines in file order.
    pub lines: Vec<DiffLine>,
}

/// What an edit did to one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiff {
    /// Path as the user knows it (relative to the project when inside it).
    pub path: String,
    /// The file did not exist before.
    pub created: bool,
    /// Lines added.
    pub added: usize,
    /// Lines removed.
    pub removed: usize,
    /// Changes with context. May be shorter than the whole diff.
    pub hunks: Vec<Hunk>,
    /// Hunk lines left out to keep the event small.
    #[serde(default)]
    pub elided: usize,
}

impl FileDiff {
    /// Diff `old` (none for a new file) against `new`.
    pub fn new(path: impl Into<String>, old: Option<&str>, new: &str) -> Self {
        Self::build(path.into(), old, new, MAX_LINES, MAX_LINE_CHARS)
    }

    /// [`Self::new`] with nothing left out: every changed line, and every
    /// line to its end. For a change whose only view is this diff, where a
    /// yes to it is a yes to all of it. `None` when a side is too large to
    /// diff at all.
    pub fn whole(path: impl Into<String>, old: Option<&str>, new: &str) -> Option<Self> {
        if old.is_some_and(|o| o.len() > MAX_BYTES) || new.len() > MAX_BYTES {
            return None;
        }
        Some(Self::build(path.into(), old, new, usize::MAX, usize::MAX))
    }

    fn build(
        path: String,
        old: Option<&str>,
        new: &str,
        max_lines: usize,
        max_line_chars: usize,
    ) -> Self {
        let created = old.is_none();
        let old = old.unwrap_or("");
        if old.len() > MAX_BYTES || new.len() > MAX_BYTES {
            let count = |s: &str| s.lines().count();
            return Self {
                path,
                created,
                added: if created { count(new) } else { 0 },
                removed: 0,
                hunks: Vec::new(),
                elided: 0,
            };
        }
        let diff = TextDiff::configure()
            .algorithm(Algorithm::Myers)
            .timeout(DEADLINE)
            .diff_lines(old, new);
        let mut added = 0;
        let mut removed = 0;
        for op in diff.ops() {
            for change in diff.iter_changes(op) {
                match change.tag() {
                    ChangeTag::Insert => added += 1,
                    ChangeTag::Delete => removed += 1,
                    ChangeTag::Equal => {}
                }
            }
        }
        let mut hunks = Vec::new();
        let mut kept = 0usize;
        let mut elided = 0usize;
        for group in join_close(diff.grouped_ops(CONTEXT)) {
            let mut lines = Vec::new();
            for op in &group {
                for change in diff.iter_changes(op) {
                    if kept >= max_lines {
                        elided += 1;
                        continue;
                    }
                    kept += 1;
                    let kind = match change.tag() {
                        ChangeTag::Equal => LineKind::Context,
                        ChangeTag::Insert => LineKind::Added,
                        ChangeTag::Delete => LineKind::Removed,
                    };
                    let text = change.value().trim_end_matches(['\n', '\r']);
                    lines.push(DiffLine {
                        kind,
                        old: change.old_index().map(|i| i + 1),
                        new: change.new_index().map(|i| i + 1),
                        text: text.chars().take(max_line_chars).collect(),
                    });
                }
            }
            if !lines.is_empty() {
                hunks.push(Hunk { lines });
            }
        }
        Self {
            path,
            created,
            added,
            removed,
            hunks,
            elided,
        }
    }

    /// True when the edit changed nothing.
    pub fn is_empty(&self) -> bool {
        self.added == 0 && self.removed == 0
    }

    /// Hunk lines, including any left out.
    pub fn len(&self) -> usize {
        self.hunks.iter().map(|h| h.lines.len()).sum::<usize>() + self.elided
    }
}

/// `new` with hunk `index` (as [`FileDiff::new`] numbers them) put back as
/// it was in `old`, and every other change kept. `None` when there is no
/// such hunk.
pub fn revert_hunk(old: &str, new: &str, index: usize) -> Option<String> {
    let diff = TextDiff::configure()
        .algorithm(Algorithm::Myers)
        .timeout(DEADLINE)
        .diff_lines(old, new);
    let groups = join_close(diff.grouped_ops(CONTEXT));
    let group = groups.get(index)?;
    let (first, last) = (group.first()?, group.last()?);
    let old_lines: Vec<&str> = old.split_inclusive('\n').collect();
    let new_lines: Vec<&str> = new.split_inclusive('\n').collect();
    let (os, oe) = (first.old_range().start, last.old_range().end);
    let (ns, ne) = (first.new_range().start, last.new_range().end);
    let mut out = String::with_capacity(new.len());
    new_lines[..ns.min(new_lines.len())]
        .iter()
        .for_each(|l| out.push_str(l));
    old_lines[os.min(old_lines.len())..oe.min(old_lines.len())]
        .iter()
        .for_each(|l| out.push_str(l));
    new_lines[ne.min(new_lines.len())..]
        .iter()
        .for_each(|l| out.push_str(l));
    Some(out)
}

/// Gaps between hunks at most this long are shown, not folded: a `⋯`
/// standing in for one line hides more than it saves.
const JOIN_GAP: usize = 2;

/// Join hunks separated by a gap of `JOIN_GAP` lines or fewer, with the gap's
/// lines as context.
fn join_close(groups: Vec<Vec<DiffOp>>) -> Vec<Vec<DiffOp>> {
    let mut out: Vec<Vec<DiffOp>> = Vec::with_capacity(groups.len());
    for group in groups {
        let (Some(prev), Some(first)) = (out.last_mut(), group.first()) else {
            out.push(group);
            continue;
        };
        let Some(last) = prev.last() else {
            out.push(group);
            continue;
        };
        let (old_end, new_end) = (last.old_range().end, last.new_range().end);
        let gap = first.new_range().start.saturating_sub(new_end);
        if gap <= JOIN_GAP && first.old_range().start.saturating_sub(old_end) == gap {
            if gap > 0 {
                prev.push(DiffOp::Equal {
                    old_index: old_end,
                    new_index: new_end,
                    len: gap,
                });
            }
            prev.extend(group);
        } else {
            out.push(group);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signs(d: &FileDiff) -> Vec<String> {
        d.hunks
            .iter()
            .map(|h| {
                h.lines
                    .iter()
                    .map(|l| {
                        let s = match l.kind {
                            LineKind::Context => ' ',
                            LineKind::Added => '+',
                            LineKind::Removed => '-',
                        };
                        format!("{s}{}", l.text)
                    })
                    .collect::<Vec<_>>()
                    .join("|")
            })
            .collect()
    }

    #[test]
    fn an_edit_in_the_middle_keeps_two_lines_of_context_and_real_numbers() {
        let old = "a\nb\nc\nd\ne\nf\ng\n";
        let new = "a\nb\nc\nD\ne\nf\ng\n";
        let d = FileDiff::new("x.txt", Some(old), new);
        assert_eq!((d.added, d.removed, d.created), (1, 1, false));
        assert_eq!(signs(&d), vec![" b| c|-d|+D| e| f"]);
        let changed: Vec<_> = d.hunks[0]
            .lines
            .iter()
            .filter(|l| l.kind != LineKind::Context)
            .collect();
        assert_eq!((changed[0].old, changed[0].new), (Some(4), None));
        assert_eq!((changed[1].old, changed[1].new), (None, Some(4)));
    }

    #[test]
    fn far_apart_changes_are_separate_hunks() {
        let old: String = (1..=30).map(|i| format!("{i}\n")).collect();
        let new: String = (1..=30)
            .map(|i| match i {
                3 => "three\n".to_string(),
                27 => "twenty-seven\n".to_string(),
                _ => format!("{i}\n"),
            })
            .collect();
        let d = FileDiff::new("n.txt", Some(&old), &new);
        assert_eq!(d.hunks.len(), 2);
        assert_eq!((d.added, d.removed), (2, 2));
    }

    /// Two changes one line apart read as one hunk, with that line shown.
    #[test]
    fn changes_a_line_apart_are_one_hunk() {
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
        let new = "1\nTWO\n3\n4\n5\n6\n7\nEIGHT\n9\n";
        let d = FileDiff::new("n.txt", Some(old), new);
        assert_eq!(d.hunks.len(), 1, "{:?}", signs(&d));
        assert_eq!(signs(&d), vec![" 1|-2|+TWO| 3| 4| 5| 6| 7|-8|+EIGHT| 9"]);
    }

    #[test]
    fn a_new_file_is_all_added_and_crlf_does_not_leak() {
        let d = FileDiff::new("new.rs", None, "fn main() {}\r\n// done\r\n");
        assert!(d.created);
        assert_eq!(d.added, 2);
        assert_eq!(signs(&d), vec!["+fn main() {}|+// done"]);
    }

    #[test]
    fn a_huge_diff_is_capped_and_counts_what_it_left_out() {
        let new: String = (0..1000).map(|i| format!("line {i}\n")).collect();
        let d = FileDiff::new("big.txt", None, &new);
        assert_eq!(d.added, 1000);
        assert_eq!(d.len(), 1000);
        assert_eq!(d.elided, 1000 - MAX_LINES);
        assert!(!d.is_empty());
        assert!(FileDiff::new("same", Some("x\n"), "x\n").is_empty());
    }

    /// One hunk goes back; the other stays. Hunks are counted as the diff
    /// shows them.
    #[test]
    fn a_hunk_reverts_alone() {
        let old: String = (1..=30).map(|i| format!("line {i}\n")).collect();
        let new = old
            .replace("line 3\n", "line three\n")
            .replace("line 25\n", "line twenty-five\nextra\n");
        let d = FileDiff::new("f", Some(&old), &new);
        assert_eq!(d.hunks.len(), 2);
        let back = revert_hunk(&old, &new, 1).unwrap();
        assert!(back.contains("line three\n") && back.contains("line 25\n"));
        assert!(!back.contains("extra"));
        let back = revert_hunk(&old, &new, 0).unwrap();
        assert!(back.contains("line 3\n") && back.contains("extra\n"));
        assert_eq!(revert_hunk(&old, &new, 2), None);
        // A new file's only hunk taken back leaves nothing.
        assert_eq!(revert_hunk("", "a\nb\n", 0).unwrap(), "");
    }
}
