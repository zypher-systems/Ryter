//! Git helpers for worktrees and merge.

use std::path::Path;
use std::process::Command;

use crate::error::{Error, Result};

/// Run `git -C dir …`.
pub fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(["-C", &dir.to_string_lossy()])
        .args(args)
        .output()
        .map_err(|e| Error::Io(format!("git: {e}")))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        return Err(Error::Io(format!(
            "git {} failed: {stderr}{stdout}",
            args.join(" ")
        )));
    }
    Ok(stdout)
}

/// Whether `dir` is inside a git work tree.
pub fn is_repo(dir: &Path) -> bool {
    git(dir, &["rev-parse", "--is-inside-work-tree"])
        .map(|s| s.trim() == "true")
        .unwrap_or(false)
}

/// Current branch name.
pub fn branch(dir: &Path) -> Result<String> {
    Ok(git(dir, &["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_string())
}

/// `git worktree add -b branch path`.
pub fn add_worktree(repo: &Path, path: &Path, branch: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::Io(e.to_string()))?;
    }
    git(
        repo,
        &["worktree", "add", "-b", branch, &path.to_string_lossy()],
    )?;
    Ok(())
}

/// Open the worktree for `branch` at `path`, creating whichever part is
/// missing. Returns true when earlier work was already there (a retry).
pub fn open_worktree(repo: &Path, path: &Path, branch: &str) -> Result<bool> {
    if path.join(".git").exists() {
        return Ok(true);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::Io(e.to_string()))?;
    }
    let exists = git(
        repo,
        &["rev-parse", "--verify", &format!("refs/heads/{branch}")],
    )
    .is_ok();
    if exists {
        let _ = git(repo, &["worktree", "prune"]);
        git(repo, &["worktree", "add", &path.to_string_lossy(), branch])?;
        Ok(true)
    } else {
        git(
            repo,
            &["worktree", "add", "-b", branch, &path.to_string_lossy()],
        )?;
        Ok(false)
    }
}

/// Remove a worktree and its branch.
pub fn remove_worktree(repo: &Path, path: &Path, branch: &str) -> Result<()> {
    let _ = git(
        repo,
        &["worktree", "remove", "--force", &path.to_string_lossy()],
    );
    let _ = git(repo, &["branch", "-D", branch]);
    let _ = std::fs::remove_dir_all(path);
    Ok(())
}

/// Remove a worktree's directory but keep its branch, so work that passed
/// (or needs a human) is still there to merge by hand.
pub fn remove_worktree_keep_branch(repo: &Path, path: &Path) {
    let _ = git(
        repo,
        &["worktree", "remove", "--force", &path.to_string_lossy()],
    );
    let _ = std::fs::remove_dir_all(path);
}

/// Resolve a revision to a full sha.
pub fn rev(dir: &Path, rev: &str) -> Result<String> {
    Ok(git(dir, &["rev-parse", "--verify", rev])?
        .trim()
        .to_string())
}

/// Current commit, for the undo point recorded before an auto-merge.
pub fn head(dir: &Path) -> Result<String> {
    rev(dir, "HEAD")
}

/// `git status --porcelain`.
pub fn porcelain(dir: &Path) -> Result<String> {
    git(dir, &["status", "--porcelain"])
}

/// Paths with staged or unstaged changes, including untracked files. Both
/// sides of a rename are reported.
pub fn dirty_paths(dir: &Path) -> Vec<String> {
    let out = git(
        dir,
        &["status", "--porcelain", "-z", "--untracked-files=all"],
    )
    .unwrap_or_default();
    let mut paths = Vec::new();
    let mut fields = out.split('\0').filter(|f| !f.is_empty());
    while let Some(entry) = fields.next() {
        if entry.len() < 4 {
            continue;
        }
        let (code, path) = entry.split_at(3);
        paths.push(path.to_string());
        // With `-z`, a rename's source path is the next field.
        if code.starts_with('R') || code.starts_with('C') {
            if let Some(src) = fields.next() {
                paths.push(src.to_string());
            }
        }
    }
    paths
}

/// Paths that differ between two commits.
pub fn changed_paths(dir: &Path, from: &str, to: &str) -> Vec<String> {
    git(dir, &["diff", "--name-only", from, to])
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// What landing `to` onto `from` would change: a `--stat` summary, then the
/// patch. The summary survives truncation, so a reviewer always sees scope.
pub fn diff_range(dir: &Path, from: &str, to: &str) -> String {
    let stat = git(dir, &["diff", "--stat", from, to]).unwrap_or_default();
    let patch = git(dir, &["diff", from, to]).unwrap_or_default();
    format!("{stat}\n{patch}")
}

/// Commit identity flags when the repository has none configured, so a
/// builder's work is not silently left uncommitted on a fresh machine.
fn identity(dir: &Path) -> Vec<String> {
    let has = git(dir, &["config", "user.email"]).is_ok_and(|s| !s.trim().is_empty());
    if has {
        Vec::new()
    } else {
        vec![
            "-c".into(),
            "user.name=ryter".into(),
            "-c".into(),
            "user.email=ryter@localhost".into(),
        ]
    }
}

fn git_as(dir: &Path, args: &[&str]) -> Result<String> {
    let id = identity(dir);
    let mut all: Vec<&str> = id.iter().map(String::as_str).collect();
    all.extend_from_slice(args);
    git(dir, &all)
}

/// Build and tool caches that are never source. Running a project's tests
/// creates them; a fresh repository has no .gitignore for them yet, and
/// `add -A` committed `__pycache__/*.pyc` on the first live crew run.
const NEVER_COMMIT: &[&str] = &[
    ":(exclude)*__pycache__*",
    ":(exclude)*.pyc",
    ":(exclude)*.pytest_cache*",
    ":(exclude)*.mypy_cache*",
    ":(exclude)*.ruff_cache*",
    ":(exclude)*node_modules*",
    ":(exclude)*.DS_Store",
];

/// Stage everything except caches and commit it. Returns whether a commit
/// was made.
pub fn commit_all(dir: &Path, message: &str) -> Result<bool> {
    let mut args = vec!["add", "-A", "--", "."];
    args.extend_from_slice(NEVER_COMMIT);
    git(dir, &args)?;
    // Only what is staged matters: changed caches alone are not a commit.
    if git(dir, &["diff", "--cached", "--quiet"]).is_ok() {
        return Ok(false);
    }
    git_as(dir, &["commit", "--no-verify", "-m", message])?;
    Ok(true)
}

/// Throw away everything in `dir` that is not committed: edits, new files,
/// scratch. Used after a review, whose probes must never reach a commit.
pub fn discard_uncommitted(dir: &Path) {
    let _ = git(dir, &["reset", "-q", "--hard", "HEAD"]);
    let _ = git(dir, &["clean", "-q", "-fd"]);
}

/// Result of pulling the target branch into a builder's worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Integration {
    /// Merged, or already up to date.
    Clean,
    /// Conflicted; markers are in these files, in the worktree only.
    Conflict(Vec<String>),
}

/// Merge `onto` into the worktree's branch.
///
/// Conflicts are resolved here, in the builder's worktree, never in the user's
/// checkout. Afterwards landing the branch onto `onto` cannot conflict.
pub fn integrate(worktree: &Path, onto: &str) -> Result<Integration> {
    match git_as(
        worktree,
        &[
            "merge",
            "--no-edit",
            "-m",
            &format!("ryter: integrate {onto}"),
            onto,
        ],
    ) {
        Ok(_) => Ok(Integration::Clean),
        Err(e) => {
            let files = unmerged(worktree);
            if files.is_empty() {
                let _ = git(worktree, &["merge", "--abort"]);
                Err(e)
            } else {
                Ok(Integration::Conflict(files))
            }
        }
    }
}

/// Files still carrying unresolved conflicts.
pub fn unmerged(dir: &Path) -> Vec<String> {
    git(dir, &["diff", "--name-only", "--diff-filter=U"])
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Abandon an in-progress merge.
pub fn merge_abort(dir: &Path) {
    let _ = git(dir, &["merge", "--abort"]);
}

/// Land `branch` onto `repo` HEAD as one revertable commit.
///
/// `--no-ff` keeps a task as a single merge commit rather than fast-forwarding
/// it into the user's history, so `git revert -m 1` undoes the whole task. A
/// failure is always aborted: a half-finished merge must never be left in the
/// user's checkout.
pub fn land(repo: &Path, branch: &str, message: &str) -> Result<()> {
    match git_as(
        repo,
        &["merge", "--no-ff", "--no-edit", "-m", message, branch],
    ) {
        Ok(_) => Ok(()),
        Err(e) => {
            merge_abort(repo);
            Err(e)
        }
    }
}

#[cfg(test)]
pub fn init_repo(dir: &Path) -> Result<()> {
    Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(dir)
        .output()
        .map_err(|e| Error::Io(e.to_string()))?;
    git(dir, &["config", "user.email", "ryter@test"])?;
    git(dir, &["config", "user.name", "ryter"])?;
    git(dir, &["config", "commit.gpgsign", "false"])?;
    std::fs::write(dir.join("README.md"), "repo\n").map_err(|e| Error::Io(e.to_string()))?;
    git(dir, &["add", "-A"])?;
    git(dir, &["commit", "-m", "init"])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn worktree_add_and_remove() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path()).unwrap();
        let wt = dir.path().join("wt");
        add_worktree(dir.path(), &wt, "ryter-test-wt").unwrap();
        assert!(wt.join("README.md").exists());
        remove_worktree(dir.path(), &wt, "ryter-test-wt").unwrap();
        assert!(!wt.exists());
    }
}
