//! What changed in the user's files, undoing one file, and committing: the
//! `/changes` and `/commit` loop.
//!
//! The "now" side is a snapshot of the working files (tracked and untracked,
//! not ignored), taken with a private index like an undo checkpoint, so new
//! files show up and the user's staging area is never touched to look.

use std::path::Path;

use crate::error::{Error, Result};
use crate::git::{self, git};

/// Git's empty tree: the base when a repository has no commit yet.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// Longest diff text sent to the model for a commit message.
const DRAFT_DIFF_CHARS: usize = 24_000;
/// Longest diff for one file within that.
const DRAFT_FILE_CHARS: usize = 6_000;

/// How a file changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// New file.
    Added,
    /// Changed file.
    Modified,
    /// Removed file.
    Deleted,
}

impl Status {
    /// One letter, as `git status` shows it.
    pub fn letter(self) -> char {
        match self {
            Status::Added => 'A',
            Status::Modified => 'M',
            Status::Deleted => 'D',
        }
    }
}

/// One changed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// Path from the repository root.
    pub path: String,
    /// How it changed.
    pub status: Status,
    /// Lines added.
    pub added: u32,
    /// Lines removed.
    pub removed: u32,
    /// Git treats it as binary (no line counts).
    pub binary: bool,
}

/// What changed between `base` and the files now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changes {
    /// Commit (or tree) compared from.
    pub base: String,
    /// Snapshot of the files now.
    pub now: String,
    /// Changed files, by path.
    pub files: Vec<FileChange>,
}

impl Changes {
    /// Lines added and removed across every file.
    pub fn totals(&self) -> (u32, u32) {
        self.files
            .iter()
            .fold((0, 0), |(a, r), f| (a + f.added, r + f.removed))
    }
}

/// The repository's top folder, where git's paths start.
pub fn root(dir: &Path) -> Result<std::path::PathBuf> {
    Ok(git(dir, &["rev-parse", "--show-toplevel"])?.trim().into())
}

/// The base for "uncommitted": `HEAD`, or the empty tree before a first commit.
pub fn head_base(dir: &Path) -> String {
    git::head(dir)
        .map(|h| h.trim().to_string())
        .unwrap_or_else(|_| EMPTY_TREE.into())
}

/// The tree of a commit: the same for the same files, whenever it was
/// made. `None` when it can't be read.
pub fn tree_of(dir: &Path, commit: &str) -> Option<String> {
    git(dir, &["rev-parse", &format!("{commit}^{{tree}}")])
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// When `HEAD` was committed, unix millis.
pub fn head_time_ms(dir: &Path) -> Option<u64> {
    git(dir, &["log", "-1", "--format=%ct", "HEAD"])
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(|s| s * 1000)
}

/// Everything that differs between `base` and the files now.
pub fn changes(dir: &Path, base: &str) -> Result<Changes> {
    if !git::is_repo(dir) {
        return Err(Error::Io("not a git repository".into()));
    }
    let now =
        git::checkpoint(dir, "review")?.ok_or_else(|| Error::Io("not a git repository".into()))?;
    let dir = &root(dir)?;
    let range = [base, now.as_str()];
    let status = git(
        dir,
        &[
            "diff",
            "--no-renames",
            "--name-status",
            "-z",
            range[0],
            range[1],
        ],
    )?;
    let numstat = git(
        dir,
        &[
            "diff",
            "--no-renames",
            "--numstat",
            "-z",
            range[0],
            range[1],
        ],
    )?;
    let mut counts = std::collections::HashMap::new();
    for rec in numstat.split('\0').filter(|r| !r.is_empty()) {
        let mut parts = rec.splitn(3, '\t');
        let (Some(a), Some(r), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        counts.insert(
            path.to_string(),
            (a.parse::<u32>().ok(), r.parse::<u32>().ok()),
        );
    }
    let mut files = Vec::new();
    let mut fields = status.split('\0').filter(|f| !f.is_empty());
    while let (Some(code), Some(path)) = (fields.next(), fields.next()) {
        let status = match code.chars().next() {
            Some('A') => Status::Added,
            Some('D') => Status::Deleted,
            _ => Status::Modified,
        };
        let (added, removed) = counts.get(path).copied().unwrap_or((None, None));
        files.push(FileChange {
            path: path.to_string(),
            status,
            added: added.unwrap_or(0),
            removed: removed.unwrap_or(0),
            binary: added.is_none(),
        });
    }
    Ok(Changes {
        base: base.to_string(),
        now,
        files,
    })
}

/// The patch for one file.
pub fn file_diff(dir: &Path, c: &Changes, path: &str) -> String {
    let Ok(top) = root(dir) else {
        return String::new();
    };
    git(
        &top,
        &[
            "diff",
            "--no-color",
            "--no-renames",
            &c.base,
            &c.now,
            "--",
            path,
        ],
    )
    .unwrap_or_else(|e| e.to_string())
}

/// Put one file back as `base` had it: restore its content, or remove it if
/// `base` didn't have it. The user's staging area is left alone.
pub fn revert_file(dir: &Path, base: &str, path: &str) -> Result<()> {
    let top = root(dir)?;
    let in_base = git(&top, &["cat-file", "-e", &format!("{base}:{path}")]).is_ok();
    if in_base {
        git(
            &top,
            &["restore", "--source", base, "--worktree", "--", path],
        )?;
    } else {
        std::fs::remove_file(top.join(path)).map_err(|e| Error::Io(format!("{path}: {e}")))?;
    }
    Ok(())
}

/// `path` as `base` had it (`None`: not there) and as it is now (`None`:
/// deleted), for a diff that can be taken apart hunk by hunk.
pub fn file_versions(
    dir: &Path,
    base: &str,
    path: &str,
) -> Result<(Option<String>, Option<String>)> {
    let top = root(dir)?;
    let old = git(&top, &["show", &format!("{base}:{path}")]).ok();
    let new = std::fs::read_to_string(top.join(path)).ok();
    Ok((old, new))
}

/// Put back one hunk of `path` as `base` had it, keeping its other changes.
/// Hunks are numbered as [`crate::diff::FileDiff::new`] numbers them.
pub fn revert_hunk(dir: &Path, base: &str, path: &str, hunk: usize) -> Result<()> {
    let top = root(dir)?;
    let (old, new) = file_versions(dir, base, path)?;
    let new = new.ok_or_else(|| Error::Io(format!("{path} is gone; undo the file instead")))?;
    let out = crate::diff::revert_hunk(old.as_deref().unwrap_or(""), &new, hunk)
        .ok_or_else(|| Error::Io(format!("{path} has no change {}", hunk + 1)))?;
    if old.is_none() && out.is_empty() {
        // All of a new file taken back: it was never there.
        return std::fs::remove_file(top.join(path)).map_err(|e| Error::Io(format!("{path}: {e}")));
    }
    std::fs::write(top.join(path), out).map_err(|e| Error::Io(format!("{path}: {e}")))
}

/// Commit exactly `paths` as they are now, with the user's identity and
/// hooks. Anything else they staged stays staged, and out of this commit.
/// Returns `<short sha> <subject>`.
pub fn commit(dir: &Path, paths: &[String], message: &str) -> Result<String> {
    if paths.is_empty() {
        return Err(Error::Io("no files chosen".into()));
    }
    if message.trim().is_empty() {
        return Err(Error::Io("the commit message is empty".into()));
    }
    let top = root(dir)?;
    let msg_file = git(&top, &["rev-parse", "--git-path", "RYTER_COMMIT_MSG"])?;
    let msg_file = top.join(msg_file.trim());
    std::fs::write(&msg_file, message).map_err(|e| Error::Io(e.to_string()))?;
    let mut add = vec!["add", "-A", "--"];
    add.extend(paths.iter().map(String::as_str));
    let result = git(&top, &add).and_then(|_| {
        let file = msg_file.to_string_lossy();
        let mut args = vec![
            "commit",
            "--cleanup=whitespace",
            "-F",
            &file,
            "--only",
            "--",
        ];
        args.extend(paths.iter().map(String::as_str));
        git(&top, &args)
    });
    let _ = std::fs::remove_file(&msg_file);
    result?;
    Ok(git(&top, &["log", "-1", "--format=%h %s"])?
        .trim()
        .to_string())
}

/// Recent commit subjects, so a drafted message matches the project's style.
pub fn recent_subjects(dir: &Path, n: usize) -> Vec<String> {
    git(dir, &["log", &format!("-{n}"), "--format=%s"])
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// The diff of `paths`, file by file, capped for a model's context. The stat
/// comes first, so a cut never hides the scope.
pub fn draft_diff(dir: &Path, c: &Changes, paths: &[String]) -> String {
    diff_within(dir, c, paths, DRAFT_DIFF_CHARS, DRAFT_FILE_CHARS)
}

/// The file list, then each file's diff, cut at `per_file` characters each
/// and `total` in all.
pub fn diff_within(
    dir: &Path,
    c: &Changes,
    paths: &[String],
    total: usize,
    per_file: usize,
) -> String {
    let mut out = String::new();
    for f in c.files.iter().filter(|f| paths.contains(&f.path)) {
        out.push_str(&format!(
            "{} {} (+{} −{})\n",
            f.status.letter(),
            f.path,
            f.added,
            f.removed
        ));
    }
    out.push('\n');
    for f in c.files.iter().filter(|f| paths.contains(&f.path)) {
        if out.len() >= total {
            out.push_str("[more files' diffs left out]\n");
            break;
        }
        let d = file_diff(dir, c, &f.path);
        if d.len() > per_file {
            let cut = (0..=per_file)
                .rev()
                .find(|&i| d.is_char_boundary(i))
                .unwrap_or(0);
            out.push_str(&d[..cut]);
            out.push_str("\n[rest of this file's diff left out]\n");
        } else {
            out.push_str(&d);
        }
    }
    out
}

/// What went into a change, for the commit's `Ryter:` trailer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Receipt {
    /// Models used, most spent first.
    pub models: Vec<String>,
    /// Priced spend.
    pub usd: f64,
    /// Some calls had no price, so `usd` is a floor.
    pub partial: bool,
    /// The latest test result, if tests ran after the last edit.
    pub tests: Option<String>,
    /// Tests ran, but files changed after.
    pub tests_stale: bool,
    /// The review hat's last verdict on this work.
    pub review: Reviewed,
}

/// What the review hat said of the work being committed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Reviewed {
    /// No review ran.
    #[default]
    No,
    /// One ran, and the files changed after.
    Stale,
    /// `VERDICT: PASS`, by this model.
    Pass(String),
    /// `VERDICT: FAIL`, by this model.
    Fail(String),
    /// It ended without a verdict.
    NoVerdict(String),
}

impl Reviewed {
    /// The verdict a review gave on `tree`, for a commit of `now`.
    pub fn of(mark: Option<&(Option<String>, String, Option<bool>)>, now: Option<&str>) -> Self {
        let Some((tree, model, verdict)) = mark else {
            return Self::No;
        };
        if tree.is_none() || tree.as_deref() != now {
            return Self::Stale;
        }
        match verdict {
            Some(true) => Self::Pass(model.clone()),
            Some(false) => Self::Fail(model.clone()),
            None => Self::NoVerdict(model.clone()),
        }
    }
}

impl Receipt {
    /// `deepseek-pro · $0.34 · tests ✓ 13 passed`.
    pub fn line(&self) -> String {
        let models = match self.models.as_slice() {
            [] => "no model calls".to_string(),
            [one] => short_model(one),
            [a, b] => format!("{}, {}", short_model(a), short_model(b)),
            [a, b, rest @ ..] => format!(
                "{}, {} +{} more",
                short_model(a),
                short_model(b),
                rest.len()
            ),
        };
        let cost = if self.partial {
            format!(
                "{}+ (some prices unknown)",
                crate::format_usd(Some(self.usd))
            )
        } else {
            crate::format_usd(Some(self.usd))
        };
        let tests = match (&self.tests, self.tests_stale) {
            (Some(t), false) => format!("tests {t}"),
            (_, true) => "tests not rerun after the last edit".into(),
            (None, false) => "no tests run".into(),
        };
        let review = match &self.review {
            Reviewed::No => "not reviewed".to_string(),
            Reviewed::Stale => "not reviewed after the last change".into(),
            Reviewed::Pass(m) => format!("review ✓ {}", short_model(m)),
            Reviewed::Fail(m) => format!("review ✗ {}", short_model(m)),
            Reviewed::NoVerdict(m) => format!("review by {} gave no verdict", short_model(m)),
        };
        format!("{models} · {cost} · {tests} · {review}")
    }
}

/// `deepseek/deepseek-pro-latest` → `deepseek-pro-latest`.
fn short_model(id: &str) -> String {
    id.trim_start_matches('~')
        .rsplit('/')
        .next()
        .unwrap_or(id)
        .to_string()
}

/// `message` with the receipt as a `Ryter:` trailer.
pub fn with_receipt(message: &str, receipt: &Receipt) -> String {
    format!("{}\n\nRyter: {}\n", message.trim_end(), receipt.line())
}

/// Instructions for drafting a commit message.
pub const DRAFT_SYSTEM: &str = "You write git commit messages. Reply with the message only: no \
code fences, no preamble, no trailers.\n\n\
- First line: a summary of at most 72 characters. Follow the style of the project's recent \
subjects when they show one (a prefix like `fix:` or `0.2.6:`, capitalization); otherwise use the \
imperative mood (\"Add\", \"Fix\").\n\
- Then a blank line and a short body, wrapped at 72 columns: why the change was made and anything \
a reviewer should know. Use the conversation for the why; don't list every file.\n\
- Skip the body when the summary says it all.\n\
- Describe only what the diff shows.";

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn repo() -> tempfile::TempDir {
        let d = tempfile::TempDir::new().unwrap();
        let p = d.path();
        git(p, &["init", "-q"]).unwrap();
        git(p, &["config", "user.email", "t@t"]).unwrap();
        git(p, &["config", "user.name", "t"]).unwrap();
        fs::write(p.join("keep.txt"), "one\ntwo\n").unwrap();
        fs::write(p.join("gone.txt"), "bye\n").unwrap();
        git(p, &["add", "."]).unwrap();
        git(p, &["commit", "-qm", "first"]).unwrap();
        d
    }

    #[test]
    fn changes_cover_new_changed_and_deleted_files() {
        let d = repo();
        let p = d.path();
        fs::write(p.join("keep.txt"), "one\n2\nthree\n").unwrap();
        fs::remove_file(p.join("gone.txt")).unwrap();
        fs::write(p.join("new.txt"), "a\nb\n").unwrap();
        let c = changes(p, &head_base(p)).unwrap();
        let got: Vec<(char, &str, u32, u32)> = c
            .files
            .iter()
            .map(|f| (f.status.letter(), f.path.as_str(), f.added, f.removed))
            .collect();
        assert_eq!(
            got,
            vec![
                ('D', "gone.txt", 0, 1),
                ('M', "keep.txt", 2, 1),
                ('A', "new.txt", 2, 0)
            ]
        );
        assert_eq!(c.totals(), (4, 2));
        assert!(file_diff(p, &c, "keep.txt").contains("+three"));
        // Looking doesn't stage anything.
        assert_eq!(git(p, &["diff", "--cached", "--name-only"]).unwrap(), "");
    }

    #[test]
    fn reverting_one_file_leaves_the_others() {
        let d = repo();
        let p = d.path();
        fs::write(p.join("keep.txt"), "changed\n").unwrap();
        fs::remove_file(p.join("gone.txt")).unwrap();
        fs::write(p.join("new.txt"), "a\n").unwrap();
        let base = head_base(p);
        revert_file(p, &base, "keep.txt").unwrap();
        revert_file(p, &base, "gone.txt").unwrap();
        assert_eq!(
            fs::read_to_string(p.join("keep.txt")).unwrap(),
            "one\ntwo\n"
        );
        assert_eq!(fs::read_to_string(p.join("gone.txt")).unwrap(), "bye\n");
        assert!(p.join("new.txt").exists());
        revert_file(p, &base, "new.txt").unwrap();
        assert!(!p.join("new.txt").exists());
        assert!(changes(p, &base).unwrap().files.is_empty());
    }

    #[test]
    fn commit_takes_only_the_chosen_files() {
        let d = repo();
        let p = d.path();
        fs::write(p.join("keep.txt"), "changed\n").unwrap();
        fs::write(p.join("new.txt"), "a\n").unwrap();
        fs::write(p.join("later.txt"), "not yet\n").unwrap();
        // Something the user staged themselves stays staged and out.
        fs::write(p.join("staged.txt"), "mine\n").unwrap();
        git(p, &["add", "staged.txt"]).unwrap();
        fs::remove_file(p.join("gone.txt")).unwrap();
        let paths = vec!["keep.txt".into(), "new.txt".into(), "gone.txt".into()];
        let out = commit(
            p,
            &paths,
            "Change keep, add new\n\nBecause.\n\nRyter: x · $0.10 · no tests run\n",
        )
        .unwrap();
        assert!(out.ends_with("Change keep, add new"), "{out}");
        let files = git(p, &["show", "--name-status", "--format=", "HEAD"]).unwrap();
        assert_eq!(files, "D\tgone.txt\nM\tkeep.txt\nA\tnew.txt\n");
        let body = git(p, &["log", "-1", "--format=%B"]).unwrap();
        assert!(body.contains("Ryter: x · $0.10"), "{body}");
        assert_eq!(
            git(p, &["diff", "--cached", "--name-only"]).unwrap(),
            "staged.txt\n"
        );
        assert!(p.join("later.txt").exists());
    }

    #[test]
    fn a_repository_without_commits_compares_to_nothing() {
        let d = tempfile::TempDir::new().unwrap();
        let p = d.path();
        git(p, &["init", "-q"]).unwrap();
        fs::write(p.join("a.txt"), "x\n").unwrap();
        let c = changes(p, &head_base(p)).unwrap();
        assert_eq!(c.files.len(), 1);
        assert_eq!(c.files[0].status, Status::Added);
    }

    #[test]
    fn the_receipt_says_what_it_knows() {
        let mut r = Receipt {
            models: vec!["~deepseek/deepseek-pro-latest".into()],
            usd: 0.34,
            tests: Some("✓ 13 passed".into()),
            ..Receipt::default()
        };
        assert_eq!(
            r.line(),
            "deepseek-pro-latest · $0.34 · tests ✓ 13 passed · not reviewed"
        );
        r.tests_stale = true;
        assert!(
            r.line().contains("tests not rerun after the last edit"),
            "{}",
            r.line()
        );
        r.tests = None;
        r.tests_stale = false;
        r.partial = true;
        assert!(r.line().contains("(some prices unknown)"), "{}", r.line());
        assert!(r.line().contains("no tests run"), "{}", r.line());
        // The review's verdict is about the files it read, and no others.
        let mark = (
            Some("t1".to_string()),
            "x-ai/grok-4.7".to_string(),
            Some(true),
        );
        r.review = Reviewed::of(Some(&mark), Some("t1"));
        assert!(r.line().ends_with("review ✓ grok-4.7"), "{}", r.line());
        r.review = Reviewed::of(Some(&mark), Some("t2"));
        assert!(
            r.line().ends_with("not reviewed after the last change"),
            "{}",
            r.line()
        );
        let failed = (Some("t1".to_string()), "m".to_string(), Some(false));
        assert_eq!(
            Reviewed::of(Some(&failed), Some("t1")),
            Reviewed::Fail("m".into())
        );
        let silent = (Some("t1".to_string()), "m".to_string(), None);
        assert_eq!(
            Reviewed::of(Some(&silent), Some("t1")),
            Reviewed::NoVerdict("m".into())
        );
        assert_eq!(Reviewed::of(None, Some("t1")), Reviewed::No);
        let m = with_receipt("Subject\n\nBody.\n", &r);
        assert!(m.starts_with("Subject\n\nBody.\n\nRyter: "), "{m}");
    }
}
