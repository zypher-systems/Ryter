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

/// Run git with a private index, so building a snapshot never touches the
/// user's staging area.
fn git_with_index(dir: &Path, index: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(["-C", &dir.to_string_lossy()])
        .args(args)
        .env("GIT_INDEX_FILE", index)
        .output()
        .map_err(|e| Error::Io(format!("git: {e}")))?;
    if !out.status.success() {
        return Err(Error::Io(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Snapshot the working files (tracked and untracked, not ignored) as a
/// commit object kept under `refs/ryter/undo/`, without touching the
/// branch, the index, or the files. `None` outside a repository.
pub fn checkpoint(dir: &Path, name: &str) -> Result<Option<String>> {
    if !is_repo(dir) {
        return Ok(None);
    }
    let index = std::path::PathBuf::from(
        git(dir, &["rev-parse", "--git-path", "ryter-undo-index"])?.trim(),
    );
    let index = if index.is_absolute() {
        index
    } else {
        dir.join(index)
    };
    let _ = std::fs::remove_file(&index);
    let has_head = head(dir).is_ok();
    if has_head {
        git_with_index(dir, &index, &["read-tree", "HEAD"])?;
    }
    git_with_index(dir, &index, &["add", "-A"])?;
    let tree = git_with_index(dir, &index, &["write-tree"])?
        .trim()
        .to_string();
    let _ = std::fs::remove_file(&index);
    let mut args = vec!["commit-tree", tree.as_str(), "-m", "ryter undo checkpoint"];
    if has_head {
        args.extend(["-p", "HEAD"]);
    }
    let sha = git_as(dir, &args)?.trim().to_string();
    git(
        dir,
        &["update-ref", &format!("refs/ryter/undo/{name}"), &sha],
    )?;
    Ok(Some(sha))
}

/// The tree a checkpoint recorded, to compare against the files now.
pub fn checkpoint_tree(dir: &Path, sha: &str) -> Result<String> {
    Ok(git(dir, &["rev-parse", &format!("{sha}^{{tree}}")])?
        .trim()
        .to_string())
}

/// Put the working files back as `sha` recorded them: restore what changed or
/// was deleted, and remove files created since. Ignored files, the index,
/// and the branch are left alone.
pub fn restore_checkpoint(dir: &Path, sha: &str) -> Result<usize> {
    let then: std::collections::HashSet<String> = git(dir, &["ls-tree", "-r", "--name-only", sha])?
        .lines()
        .map(str::to_string)
        .collect();
    let now = git(dir, &["ls-files", "-co", "--exclude-standard"])?;
    let mut touched = 0;
    for f in now.lines().filter(|f| !then.contains(*f)) {
        if std::fs::remove_file(dir.join(f)).is_ok() {
            touched += 1;
        }
    }
    if !then.is_empty() {
        let missing = then.iter().filter(|f| !dir.join(f).exists()).count();
        let changed = git(dir, &["diff", "--name-only", sha, "--", "."])?
            .lines()
            .filter(|f| dir.join(f).exists())
            .count();
        git(dir, &["restore", "--source", sha, "--worktree", "--", "."])?;
        touched += missing + changed;
    }
    Ok(touched)
}

/// Whether `dir` is inside a git work tree.
pub fn is_repo(dir: &Path) -> bool {
    git(dir, &["rev-parse", "--is-inside-work-tree"])
        .map(|s| s.trim() == "true")
        .unwrap_or(false)
}

/// Kept out of the first commit Ryter makes in a folder with no repository:
/// secrets first, then the caches and dependency trees tools create.
const FIRST_GITIGNORE: &str = "\
# Written by Ryter when it set up git here. Edit freely.
.env
.env.*
*.pem
*.key
node_modules/
__pycache__/
*.pyc
.venv/
venv/
.pytest_cache/
.mypy_cache/
.ruff_cache/
target/
.DS_Store
";

/// What [`ensure_repo`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoSetup {
    /// A repository was created (so removing `.git` undoes it).
    pub created: bool,
    /// What was done, for the user.
    pub summary: String,
}

/// Make `dir` a repository with at least one commit, so a crew has a branch
/// to build from and a patch has somewhere to land. `None` when there was
/// nothing to do.
///
/// A folder with no repository gets `git init` (the user's
/// `init.defaultBranch`, else `main`), a `.gitignore` for secrets and caches
/// unless one exists, and a first commit of what is there. A repository with
/// no commits gets the first commit.
pub fn ensure_repo(dir: &Path) -> Result<Option<RepoSetup>> {
    let mut did = Vec::new();
    let created = !is_repo(dir);
    // Started from the home folder or the root: a repository there would
    // sweep up everything the user owns. Ask them to pick a project folder.
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    if created
        && (dir.parent().is_none() || home.as_deref().is_some_and(|h| canon(h) == canon(dir)))
    {
        return Err(Error::Config(
            "Ryter won't create a git repository in your home folder or at the root. Start it \
             in a project folder (mkdir myapp && cd myapp && ryter)."
                .into(),
        ));
    }
    if created {
        let branch = git(dir, &["config", "--get", "init.defaultBranch"])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "main".into());
        git(dir, &["init", "-q", "-b", &branch])?;
        did.push(format!("initialized a git repository on `{branch}`"));
    }
    if head(dir).is_ok() {
        return Ok((!did.is_empty()).then(|| RepoSetup {
            created,
            summary: did.join(", "),
        }));
    }
    let ignore = dir.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(&ignore, FIRST_GITIGNORE).map_err(|e| Error::Io(e.to_string()))?;
        did.push("added a .gitignore for secrets and caches".into());
    }
    git(dir, &["add", "-A"])?;
    let files = git(dir, &["diff", "--cached", "--name-only"])?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count();
    git_as(
        dir,
        &[
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "Initial commit (made by Ryter)",
        ],
    )?;
    did.push(match files {
        0 => "made an empty first commit".into(),
        1 => "committed the 1 file already here as the starting point".into(),
        n => format!("committed the {n} files already here as the starting point"),
    });
    Ok(Some(RepoSetup {
        created,
        summary: did.join(", "),
    }))
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

    fn tracked(dir: &Path) -> Vec<String> {
        git(dir, &["ls-files"])
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// An empty folder becomes a repository with a first commit.
    #[test]
    fn an_empty_folder_gets_a_repository_and_a_first_commit() {
        let dir = TempDir::new().unwrap();
        let setup = ensure_repo(dir.path())
            .unwrap()
            .expect("something was done");
        assert!(setup.created);
        assert!(is_repo(dir.path()) && head(dir.path()).is_ok());
        assert!(setup.summary.contains("initialized"), "{}", setup.summary);
        // The .gitignore it wrote is the first commit.
        assert_eq!(tracked(dir.path()), vec![".gitignore".to_string()]);
        // Nothing left to do the second time.
        assert_eq!(ensure_repo(dir.path()).unwrap(), None);
    }

    /// Existing files are the starting point; secrets and caches are not.
    #[test]
    fn existing_files_are_committed_but_secrets_and_caches_are_not() {
        let dir = TempDir::new().unwrap();
        let d = dir.path();
        std::fs::write(d.join("app.py"), "print(1)\n").unwrap();
        std::fs::write(d.join(".env"), "API_KEY=secret\n").unwrap();
        std::fs::create_dir_all(d.join("node_modules/x")).unwrap();
        std::fs::write(d.join("node_modules/x/i.js"), "").unwrap();
        let setup = ensure_repo(d).unwrap().unwrap();
        let files = tracked(d);
        assert!(files.contains(&"app.py".to_string()), "{files:?}");
        assert!(
            !files
                .iter()
                .any(|f| f.contains(".env") && f != ".gitignore"),
            "{files:?}"
        );
        assert!(
            !files.iter().any(|f| f.starts_with("node_modules")),
            "{files:?}"
        );
        assert!(setup.summary.contains("2 files"), "{}", setup.summary);
        assert!(
            porcelain(d).unwrap().is_empty(),
            "a clean tree to build from"
        );
    }

    /// A repository someone made but never committed to gets its first
    /// commit, and is not reported as created (so no "delete .git" advice).
    #[test]
    fn an_unborn_repository_gets_a_first_commit_only() {
        let dir = TempDir::new().unwrap();
        let d = dir.path();
        git(d, &["init", "-q", "-b", "trunk"]).unwrap();
        std::fs::write(d.join(".gitignore"), "secret.txt\n").unwrap();
        std::fs::write(d.join("secret.txt"), "x").unwrap();
        let setup = ensure_repo(d).unwrap().unwrap();
        assert!(!setup.created);
        assert_eq!(branch(d).unwrap(), "trunk", "their branch name is kept");
        // Their own .gitignore is used, not replaced.
        assert_eq!(
            std::fs::read_to_string(d.join(".gitignore")).unwrap(),
            "secret.txt\n"
        );
        assert!(!tracked(d).contains(&"secret.txt".to_string()));
    }

    /// A checkpoint puts back edits, deletions, and new files, and never
    /// touches the branch or the staging area.
    #[test]
    fn a_checkpoint_restores_files_without_touching_git_state() {
        let dir = TempDir::new().unwrap();
        let d = dir.path();
        init_repo(d).unwrap();
        std::fs::write(d.join("keep.txt"), "draft\n").unwrap(); // untracked, pre-existing
        git(d, &["add", "README.md"]).unwrap();
        let head_before = head(d).unwrap();
        let sha = checkpoint(d, "t1").unwrap().unwrap();
        assert_eq!(
            porcelain(d).unwrap(),
            "?? keep.txt\n",
            "files and index untouched"
        );
        // A turn edits, deletes, and creates.
        std::fs::write(d.join("README.md"), "changed\n").unwrap();
        std::fs::remove_file(d.join("keep.txt")).unwrap();
        std::fs::write(d.join("new.rs"), "fn x() {}\n").unwrap();
        restore_checkpoint(d, &sha).unwrap();
        assert_eq!(
            std::fs::read_to_string(d.join("README.md")).unwrap(),
            "repo\n"
        );
        assert_eq!(
            std::fs::read_to_string(d.join("keep.txt")).unwrap(),
            "draft\n"
        );
        assert!(!d.join("new.rs").exists());
        assert_eq!(head(d).unwrap(), head_before, "no commit on the branch");
        assert_eq!(porcelain(d).unwrap(), "?? keep.txt\n");
    }

    /// Never a repository in the home folder.
    #[test]
    fn no_repository_in_the_home_folder() {
        let home = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap();
        if is_repo(&home) {
            return; // a dotfiles repo: nothing to create, nothing to test
        }
        assert!(ensure_repo(&home).is_err());
        assert!(!home.join(".git").exists());
    }

    /// A repository with history is left alone.
    #[test]
    fn a_repository_with_history_is_left_alone() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path()).unwrap();
        let before = head(dir.path()).unwrap();
        assert_eq!(ensure_repo(dir.path()).unwrap(), None);
        assert_eq!(head(dir.path()).unwrap(), before);
        assert!(!dir.path().join(".gitignore").exists());
    }
}
