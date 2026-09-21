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

/// Combined diff (staged + unstaged) in `dir`.
pub fn diff(dir: &Path) -> Result<String> {
    let unstaged = git(dir, &["diff"])?;
    let staged = git(dir, &["diff", "--cached"])?;
    Ok(format!("{unstaged}{staged}"))
}

/// Merge `branch` into `repo` HEAD as one revertable commit.
///
/// `--no-ff` keeps the builder's work as a single merge commit rather than
/// fast-forwarding it into the user's history, so `git revert -m 1` undoes the
/// whole task.
pub fn merge_branch(repo: &Path, branch: &str) -> Result<()> {
    git(repo, &["merge", "--no-ff", "--no-edit", branch]).map(|_| ())
}

/// Current commit, for the undo point recorded before an auto-merge.
pub fn head(dir: &Path) -> Result<String> {
    Ok(git(dir, &["rev-parse", "HEAD"])?.trim().to_string())
}

/// True when the tree has staged or unstaged changes.
pub fn is_dirty(dir: &Path) -> bool {
    !porcelain(dir).unwrap_or_default().trim().is_empty()
}

/// Rebase `worktree` onto `onto` (a ref in that repo).
pub fn rebase(worktree: &Path, onto: &str) -> Result<()> {
    git(worktree, &["rebase", onto]).map(|_| ())
}

/// Abort an in-progress rebase.
pub fn rebase_abort(worktree: &Path) {
    let _ = git(worktree, &["rebase", "--abort"]);
}

/// `git status --porcelain`.
pub fn porcelain(dir: &Path) -> Result<String> {
    git(dir, &["status", "--porcelain"])
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
