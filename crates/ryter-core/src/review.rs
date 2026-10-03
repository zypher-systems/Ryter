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

    /// The same changes, without Ryter's own bookkeeping: the work.
    pub fn work(mut self) -> Self {
        self.files.retain(|f| !is_bookkeeping(&f.path));
        self
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

/// Ryter's own record of the work, kept in the project: approved plans,
/// decisions, the run file, test reports. They are not the work. A review
/// doesn't read them as changes, and writing one doesn't make a review or a
/// test out of date.
pub fn is_bookkeeping(path: &str) -> bool {
    // Wherever the project sits in its repository: a project in a
    // subfolder keeps its `.ryter/` there.
    let own = match path.rsplit_once(".ryter/") {
        Some((before, own)) if before.is_empty() || before.ends_with('/') => own,
        _ => return false,
    };
    own.starts_with("plans/")
        || own.starts_with("tests/")
        || own == "decisions.md"
        || own == "run.toml"
}

/// What identifies the work in a commit: the same for the same files,
/// whenever it was made, and unchanged by Ryter's own bookkeeping
/// ([`is_bookkeeping`]). `None` when it can't be read.
///
/// It was the commit's tree. A test writes its report into the project, so
/// a review that passed read as "not reviewed after the last change" as
/// soon as the work was tested.
pub fn tree_of(dir: &Path, commit: &str) -> Option<String> {
    use sha2::{Digest, Sha256};
    // The whole tree, from the repository's top, wherever `dir` is in it:
    // from a subfolder `ls-tree` lists that folder alone, so the commit
    // panel and the review hashed different things and the receipt could
    // never say "reviewed".
    let listing = git(dir, &["ls-tree", "-r", "-z", "--full-tree", commit]).ok()?;
    if listing.is_empty() && git(dir, &["rev-parse", &format!("{commit}^{{tree}}")]).is_err() {
        return None;
    }
    let mut h = Sha256::new();
    // `<mode> <type> <id>\t<path>`, one a file.
    for entry in listing.split('\0').filter(|e| !e.is_empty()) {
        let path = entry.split_once('\t').map_or("", |(_, p)| p);
        if !is_bookkeeping(path) {
            h.update(entry.as_bytes());
            h.update([0]);
        }
    }
    Some(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
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

/// A new file's length in lines, or `None` for one that is not counted:
/// anything but a regular file, one longer than `cap` bytes, and one that
/// is not text.
///
/// What git lists as untracked is whatever is in the folder. A link is not
/// followed, and nothing is read past `cap`: a link to `/dev/zero` reports
/// a size of nothing and never ends, and a pipe waits for a writer. This
/// runs where the screen is drawn.
fn count_lines(root: &Path, path: &str, cap: u64) -> Option<u32> {
    use std::io::Read;
    let file = crate::project_file::open(root, Path::new(path)).ok()?;
    let mut bytes = Vec::new();
    file.take(cap.saturating_add(1))
        .read_to_end(&mut bytes)
        .ok()?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > cap || bytes.contains(&0) {
        return None;
    }
    let text = std::str::from_utf8(&bytes).ok()?;
    Some(u32::try_from(text.lines().count()).unwrap_or(u32::MAX))
}

/// What differs from the last commit, the work only, read without writing
/// anything: [`changes`] snapshots the files into the repository first,
/// which is more than a glance at the screen's side should do.
pub fn uncommitted(dir: &Path) -> Result<Vec<FileChange>> {
    if !git::is_repo(dir) {
        return Err(Error::Io("not a git repository".into()));
    }
    let dir = &root(dir)?;
    let mut files = Vec::new();
    // Before a first commit there is no `HEAD` to compare with, and every
    // file is new.
    if git::head(dir).is_ok() {
        let numstat = git(dir, &["diff", "--no-renames", "--numstat", "-z", "HEAD"])?;
        let mut counts = std::collections::HashMap::new();
        for rec in numstat.split('\0').filter(|r| !r.is_empty()) {
            let mut parts = rec.splitn(3, '\t');
            if let (Some(a), Some(r), Some(path)) = (parts.next(), parts.next(), parts.next()) {
                counts.insert(path, (a.parse::<u32>().ok(), r.parse::<u32>().ok()));
            }
        }
        let status = git(
            dir,
            &["diff", "--no-renames", "--name-status", "-z", "HEAD"],
        )?;
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
    }
    // Files git has never seen: the diff above leaves them out.
    let seen = if files.is_empty() && git::head(dir).is_err() {
        git(
            dir,
            &[
                "ls-files",
                "-z",
                "--cached",
                "--others",
                "--exclude-standard",
            ],
        )?
    } else {
        git(dir, &["ls-files", "-z", "--others", "--exclude-standard"])?
    };
    // A new file's length in lines, for the first few only: a folder of
    // installed packages nobody has ignored yet is thousands of files, and
    // this runs between turns.
    const COUNTED_BYTES: u64 = 1 << 20;
    const COUNTED_FILES: usize = 50;
    let mut counted = 0;
    for path in seen.split('\0').filter(|p| !p.is_empty()) {
        if files.iter().any(|f| f.path == path) {
            continue;
        }
        let lines = (counted < COUNTED_FILES)
            .then(|| count_lines(dir, path, COUNTED_BYTES))
            .flatten();
        counted += 1;
        files.push(FileChange {
            path: path.to_string(),
            status: Status::Added,
            added: lines.unwrap_or(0),
            removed: 0,
            binary: lines.is_none(),
        });
    }
    files.retain(|f| !is_bookkeeping(&f.path));
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
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
    let scratch = git::Scratch::new(&top)?;
    let msg_file = scratch.path().join("message");
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
    fn uncommitted_lists_the_work_and_writes_nothing() {
        let d = repo();
        let p = d.path();
        assert!(uncommitted(p).unwrap().is_empty());
        fs::write(p.join("keep.txt"), "one\n2\nthree\n").unwrap();
        fs::remove_file(p.join("gone.txt")).unwrap();
        fs::write(p.join("new.txt"), "a\nb\n").unwrap();
        fs::create_dir_all(p.join(".ryter/plans")).unwrap();
        fs::write(p.join(".ryter/plans/a.md"), "plan\n").unwrap();
        let objects = || git(p, &["count-objects"]).unwrap();
        let before = objects();
        let got: Vec<(char, String, u32, u32)> = uncommitted(p)
            .unwrap()
            .iter()
            .map(|f| (f.status.letter(), f.path.clone(), f.added, f.removed))
            .collect();
        assert_eq!(
            got,
            vec![
                ('D', "gone.txt".to_string(), 0, 1),
                ('M', "keep.txt".to_string(), 2, 1),
                ('A', "new.txt".to_string(), 2, 0)
            ]
        );
        assert_eq!(objects(), before, "a glance wrote to the repository");
        assert_eq!(git(p, &["diff", "--cached", "--name-only"]).unwrap(), "");
        // Before a first commit every file is new.
        let fresh = tempfile::TempDir::new().unwrap();
        git(fresh.path(), &["init", "-q"]).unwrap();
        fs::write(fresh.path().join("a.txt"), "x\n").unwrap();
        let got = uncommitted(fresh.path()).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].status, got[0].added), (Status::Added, 1));
        // Not a repository: nothing to compare with.
        let plain = tempfile::TempDir::new().unwrap();
        assert!(uncommitted(plain.path()).is_err());
    }

    /// What git lists as untracked is whatever is in the folder. None of
    /// these may be read to its end: two of them have none.
    #[cfg(unix)]
    #[test]
    fn uncommitted_counts_only_regular_text_files_and_never_waits() {
        use std::os::unix::fs::symlink;
        let d = repo();
        let p = d.path();
        assert_eq!(count_lines(p, "nothing-here", 100), None);
        symlink("/dev/zero", p.join("endless")).unwrap();
        symlink("keep.txt", p.join("alias")).unwrap();
        // A pipe, where the platform lets the test make one.
        #[cfg(target_os = "linux")]
        {
            rustix::fs::mknodat(
                rustix::fs::CWD,
                p.join("pipe"),
                rustix::fs::FileType::Fifo,
                rustix::fs::Mode::from_raw_mode(0o600),
                0,
            )
            .unwrap();
        }
        // Asked for by name: refused, not waited on.
        assert_eq!(count_lines(p, "pipe", 100), None);
        assert_eq!(count_lines(p, "endless", 100), None);
        fs::write(p.join("big.txt"), "x\n".repeat(600_000)).unwrap();
        fs::write(p.join("edge.txt"), "y\n".repeat(1 << 19)).unwrap();
        fs::write(p.join("blob.bin"), [0u8, 159, 146, 150]).unwrap();
        fs::write(p.join("latin.txt"), [0xe9u8, b'\n']).unwrap();
        fs::write(p.join("plain.txt"), "a\nb\nc\n").unwrap();
        // On another thread: if it waits on the pipe or the endless file,
        // the test fails instead of hanging the suite.
        let (tx, rx) = std::sync::mpsc::channel();
        let dir = p.to_path_buf();
        std::thread::spawn(move || {
            let _ = tx.send(uncommitted(&dir));
        });
        let files = rx
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("uncommitted() did not return")
            .unwrap();
        let got: Vec<(&str, u32, bool)> = files
            .iter()
            .map(|f| (f.path.as_str(), f.added, f.binary))
            .collect();
        assert_eq!(
            got,
            vec![
                ("alias", 0, true),
                ("big.txt", 0, true),
                ("blob.bin", 0, true),
                // Exactly the limit is still counted.
                ("edge.txt", 1 << 19, false),
                ("endless", 0, true),
                ("latin.txt", 0, true),
                // Git does not list the pipe at all; had it, it would not
                // be opened for reading either.
                ("plain.txt", 3, false),
            ]
        );
        assert!(files.iter().all(|f| f.status == Status::Added));
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
        let legacy = p.join(".git/RYTER_COMMIT_MSG");
        fs::write(&legacy, "another operation's message").unwrap();
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
        assert_eq!(
            fs::read_to_string(&legacy).unwrap(),
            "another operation's message"
        );
        assert!(commit(p, &["missing-file".into()], "fails").is_err());
        assert!(
            !fs::read_dir(p.join(".git"))
                .unwrap()
                .flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with("ryter-tmp-"))
        );
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

    /// What identifies the work doesn't change when Ryter writes its own
    /// record of it into the project (a plan, a decision, the run file, a
    /// test report), and does when the work changes.
    #[test]
    fn ryters_own_files_do_not_change_what_was_reviewed() {
        let d = tempfile::TempDir::new().unwrap();
        let p = d.path();
        crate::git::init_repo(p).unwrap();
        fs::write(p.join("a.txt"), "one\n").unwrap();
        let tree = |p: &Path| {
            let c = changes(p, &head_base(p)).unwrap();
            tree_of(p, &c.now).expect("a tree")
        };
        let before = tree(p);
        fs::create_dir_all(p.join(".ryter/plans")).unwrap();
        fs::create_dir_all(p.join(".ryter/tests")).unwrap();
        fs::write(p.join(".ryter/plans/2026-10-01-cms.md"), "# plan\n").unwrap();
        fs::write(p.join(".ryter/tests/2026-10-01-cms.md"), "# report\n").unwrap();
        fs::write(p.join(".ryter/decisions.md"), "# Decisions\n").unwrap();
        fs::write(p.join(".ryter/run.toml"), "start = \"x\"\n").unwrap();
        assert_eq!(tree(p), before, "bookkeeping changed the work's identity");
        // The reviewer isn't given them as changes either.
        let work = changes(p, &head_base(p)).unwrap().work();
        let paths: Vec<&str> = work.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["a.txt"]);
        // The work itself, and the project's own skills, are the work.
        fs::write(p.join("a.txt"), "two\n").unwrap();
        let changed = tree(p);
        assert_ne!(changed, before);
        fs::create_dir_all(p.join(".ryter/skills/x")).unwrap();
        fs::write(p.join(".ryter/skills/x/SKILL.md"), "skill\n").unwrap();
        assert_ne!(tree(p), changed);
        assert!(!is_bookkeeping(".ryter/skills/x/SKILL.md"));
        assert!(!is_bookkeeping("src/my.ryter/run.toml"));
        // A project in a subfolder of its repository: the same work has
        // the same identity from the top and from the project's folder, and
        // its own `.ryter/` is bookkeeping there too.
        fs::create_dir_all(p.join("app/.ryter/tests")).unwrap();
        fs::write(p.join("app/main.py"), "x\n").unwrap();
        let from_top = tree(p);
        assert_eq!(tree(&p.join("app")), from_top);
        fs::write(p.join("app/.ryter/tests/2026-10-01-x.md"), "# report\n").unwrap();
        assert_eq!(tree(p), from_top);
        assert!(is_bookkeeping("app/.ryter/tests/2026-10-01-x.md"));
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
        assert!(r.line().ends_with("· review ✓ grok-4.7"), "{}", r.line());
        r.review = Reviewed::of(Some(&mark), Some("t2"));
        assert!(
            r.line().ends_with("· not reviewed after the last change"),
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
