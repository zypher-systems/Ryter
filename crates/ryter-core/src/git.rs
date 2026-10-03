//! Git helpers: the repository a turn works in, and the snapshots that
//! `/undo`, `/changes` and a review are measured against.

mod scratch;
pub(crate) use scratch::Scratch;

use std::path::{Path, PathBuf};
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
    let scratch = Scratch::new(dir)?;
    let index = scratch.path().join("index");
    let has_head = head(dir).is_ok();
    if has_head {
        git_with_index(dir, &index, &["read-tree", "HEAD"])?;
    }
    git_with_index(dir, &index, &["add", "-A"])?;
    let tree = git_with_index(dir, &index, &["write-tree"])?
        .trim()
        .to_string();
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

/// Paths that differ between two snapshots: changed, added, or deleted
/// (renames as both paths). Errors are errors: undo must not guess.
pub fn paths_between(dir: &Path, from: &str, to: &str) -> Result<Vec<String>> {
    Ok(git(
        dir,
        &[
            "diff",
            "--name-only",
            "--no-renames",
            "--no-relative",
            "-z",
            from,
            to,
            "--",
            ".",
        ],
    )?
    .split('\0')
    .filter(|p| !p.is_empty())
    .map(str::to_string)
    .collect())
}

/// Put `paths` back as snapshot `sha` has them: restore the ones it has,
/// delete the ones it doesn't. Nothing else is touched. Returns how many.
/// The repository's top folder, canonical.
pub fn toplevel(dir: &Path) -> Result<PathBuf> {
    let top = PathBuf::from(git(dir, &["rev-parse", "--show-toplevel"])?.trim());
    Ok(top.canonicalize().unwrap_or(top))
}

pub fn restore_paths(dir: &Path, sha: &str, paths: &[String]) -> Result<usize> {
    // Diff paths are repository-relative, even when the session started
    // inside a subdirectory. Both git and filesystem operations must use
    // that same root; otherwise app/file becomes app/app/file.
    let root = PathBuf::from(git(dir, &["rev-parse", "--show-toplevel"])?.trim());
    let dir = root.as_path();
    // Do not turn an invalid snapshot or a corrupt saved path into a
    // deletion. Validate the complete request before changing any file.
    git(dir, &["cat-file", "-e", &format!("{sha}^{{tree}}")])?;
    for path in paths {
        if path.is_empty()
            || Path::new(path)
                .components()
                .any(|p| !matches!(p, std::path::Component::Normal(_)))
        {
            return Err(Error::Io(format!(
                "invalid repository-relative restore path: {path}"
            )));
        }
    }
    let mut keep = Vec::new();
    let mut n = 0;
    for p in paths {
        if git(dir, &["cat-file", "-e", &format!("{sha}:{p}")]).is_ok() {
            keep.push(p.as_str());
        } else if std::fs::remove_file(dir.join(p)).is_ok() {
            n += 1;
        }
    }
    for chunk in keep.chunks(200) {
        let mut args = vec![
            "--literal-pathspecs",
            "restore",
            "--source",
            sha,
            "--worktree",
            "--",
        ];
        args.extend(chunk.iter().copied());
        git(dir, &args)?;
        n += chunk.len();
    }
    Ok(n)
}

/// Whether git ignores `path` (relative to `dir`), so snapshots skip it.
pub fn is_ignored(dir: &Path, path: &str) -> bool {
    Command::new("git")
        .args([
            "-C",
            &dir.to_string_lossy(),
            "check-ignore",
            "-q",
            "--",
            path,
        ])
        .status()
        .is_ok_and(|s| s.success())
}

/// Store the file's content as a git object, for putting it back later;
/// `None` when there is no file.
pub fn save_blob(dir: &Path, path: &str) -> Result<Option<String>> {
    if !dir.join(path).is_file() {
        return Ok(None);
    }
    Ok(Some(
        git(dir, &["hash-object", "-w", "--", path])?
            .trim()
            .to_string(),
    ))
}

/// Write a saved object back as the file, byte for byte; `None` removes it.
pub fn put_blob(dir: &Path, path: &str, blob: Option<&str>) -> Result<()> {
    let target = dir.join(path);
    let Some(blob) = blob else {
        let _ = std::fs::remove_file(&target);
        return Ok(());
    };
    let out = Command::new("git")
        .args(["-C", &dir.to_string_lossy(), "cat-file", "blob", blob])
        .output()
        .map_err(|e| Error::Io(format!("git: {e}")))?;
    if !out.status.success() {
        return Err(Error::Io(format!("git cat-file blob {blob} failed")));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::Io(e.to_string()))?;
    }
    std::fs::write(&target, out.stdout).map_err(|e| Error::Io(e.to_string()))
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

/// Make `dir` a repository with at least one commit, so a build turn has
/// something to snapshot and undo against. `None` when there was nothing
/// to do.
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
    // A folder that holds other projects, like `~/workspace`: a first commit
    // here swept 29 of them into one repository. Refuse before touching it.
    let repos = holds_repos(dir);
    if !repos.is_empty() {
        return Err(swept_in(dir, created, &repos));
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
    let wrote_ignore = !ignore.exists();
    if wrote_ignore {
        std::fs::write(&ignore, FIRST_GITIGNORE).map_err(|e| Error::Io(e.to_string()))?;
        did.push("added a .gitignore for secrets and caches".into());
    }
    // Git has the last word. `holds_repos` looks two levels down and
    // follows git's ignore rules as best it can; git itself, staging into a
    // private index, sees every repository the commit would hold, at any
    // depth and under every rule. On a refusal, undo only what this call did.
    // A check that couldn't run (git can't stage a repository with no
    // commit, say) refuses too, and undoes the same way.
    let checked = repos_to_add(dir);
    if checked.as_ref().map_or(true, |repos| !repos.is_empty()) {
        if wrote_ignore {
            let _ = std::fs::remove_file(&ignore);
        }
        // The repository made above, still without a commit.
        if created && head(dir).is_err() {
            let _ = std::fs::remove_dir_all(dir.join(".git"));
        }
        return Err(match checked {
            Ok(repos) => swept_in(dir, created, &repos),
            Err(e) => Error::Config(format!(
                "Ryter didn't {} {}: git couldn't stage it ({e}). Nothing was left behind.",
                if created {
                    "set up git in"
                } else {
                    "make the first commit in"
                },
                dir.display()
            )),
        });
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

/// Why Ryter won't make the first commit in `dir`: `repos` would be swept in.
fn swept_in(dir: &Path, created: bool, repos: &[String]) -> Error {
    let what = if created {
        "set up git in"
    } else {
        "make the first commit in"
    };
    Error::Config(format!(
        "Ryter won't {what} {}: it holds other repositories ({}), and they'd be swept into \
         it. If this is a folder of projects, start Ryter in the project's own folder (mkdir \
         myapp && cd myapp && ryter). If they're this project's dependencies, add their folder \
         to .gitignore. To make one repository here anyway, run git init and make the first \
         commit yourself.",
        dir.display(),
        name_repos(repos)
    ))
}

/// The other repositories `git add -A` would put in `dir`'s first commit
/// (gitlinks). Staged into a private copy of the index, removed after, so
/// the user's staging area isn't touched. A copy, not an empty index: in an
/// unborn repository the user may already have staged one, and ignore rules
/// don't take a staged entry out.
fn repos_to_add(dir: &Path) -> Result<Vec<String>> {
    let git_dir = PathBuf::from(git(dir, &["rev-parse", "--absolute-git-dir"])?.trim());
    let scratch = Scratch::new(dir)?;
    let index = scratch.path().join("index");
    let real = git_dir.join("index");
    if real.is_file() {
        std::fs::copy(&real, &index).map_err(|e| Error::Io(e.to_string()))?;
    }
    let staged = git_with_index(dir, &index, &["add", "-A"])
        .and_then(|_| git_with_index(dir, &index, &["ls-files", "-s", "-z"]));
    Ok(gitlinks_in(&staged?))
}

/// The gitlinks (mode 160000) in `git ls-files -s -z` output.
fn gitlinks_in(listing: &str) -> Vec<String> {
    listing
        .split('\0')
        .filter_map(|e| e.strip_prefix("160000 "))
        .filter_map(|e| e.split_once('\t'))
        .map(|(_, path)| path.to_string())
        .collect()
}

/// The repositories inside `dir` when it is a folder of projects rather
/// than a project: not a repository with commits itself, but holding some
/// (`~/workspace`). Empty for a project folder.
pub fn holds_repos(dir: &Path) -> Vec<String> {
    if !is_repo(dir) {
        return repos_inside(dir);
    }
    if head(dir).is_ok() {
        return Vec::new();
    }
    // Unborn: a repository staged by hand is in the first commit whatever
    // the ignore rules say.
    let mut found = repos_inside(dir);
    let staged = git(dir, &["ls-files", "-s", "-z"]).unwrap_or_default();
    for repo in gitlinks_in(&staged) {
        if !found.contains(&repo) {
            found.push(repo);
        }
    }
    found
}

/// `alpha, beta, gamma, and 2 more`.
pub fn name_repos(repos: &[String]) -> String {
    let mut names: Vec<String> = repos.iter().take(3).cloned().collect();
    if repos.len() > 3 {
        names.push(format!("and {} more", repos.len() - 3));
    }
    names.join(", ")
}

/// Repositories in `dir`'s folders, or one level further down
/// (`org/app`): a folder of projects rather than a project. A repository
/// isn't searched.
///
/// Folders git will ignore are skipped, as `git add -A` leaves their
/// repositories out, judged as git does: the nearest `.gitignore` first,
/// then the folder's own `.gitignore` (or Ryter's first one, where it will
/// be written), then `.git/info/exclude`, and nothing inside an ignored
/// folder comes back. The user's global excludes aren't read, so a doubt
/// refuses setup. Git itself checks again before the first commit
/// ([`ensure_repo`]), at any depth.
fn repos_inside(dir: &Path) -> Vec<String> {
    use ignore::Match;
    use ignore::gitignore::{Gitignore, GitignoreBuilder};
    fn rules(base: &Path, file: &Path) -> Option<Gitignore> {
        if !file.is_file() {
            return None;
        }
        let mut b = GitignoreBuilder::new(base);
        let _ = b.add(file);
        b.build().ok()
    }
    fn folders(dir: &Path) -> Vec<(String, PathBuf)> {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut out: Vec<_> = rd
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && e.file_name() != ".git")
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
            .collect();
        out.sort();
        out
    }
    fn ignored(layers: &[Option<&Gitignore>], path: &Path) -> bool {
        for rules in layers.iter().flatten() {
            match rules.matched(path, true) {
                Match::Ignore(_) => return true,
                Match::Whitelist(_) => return false,
                Match::None => {}
            }
        }
        false
    }
    let top = rules(dir, &dir.join(".gitignore")).or_else(|| {
        let mut b = GitignoreBuilder::new(dir);
        for line in FIRST_GITIGNORE.lines() {
            let _ = b.add_line(None, line);
        }
        b.build().ok()
    });
    let exclude = rules(dir, &dir.join(".git/info/exclude"));
    let mut found = Vec::new();
    for (name, path) in folders(dir) {
        if ignored(&[top.as_ref(), exclude.as_ref()], &path) {
            continue;
        }
        if path.join(".git").exists() {
            found.push(name);
            continue;
        }
        let near = rules(&path, &path.join(".gitignore"));
        for (inner, path) in folders(&path) {
            if !ignored(&[near.as_ref(), top.as_ref(), exclude.as_ref()], &path)
                && path.join(".git").exists()
            {
                found.push(format!("{name}/{inner}"));
            }
        }
    }
    found
}

/// Current branch name.
pub fn branch(dir: &Path) -> Result<String> {
    Ok(git(dir, &["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_string())
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
#[cfg(test)]
pub fn porcelain(dir: &Path) -> Result<String> {
    git(dir, &["status", "--porcelain"])
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
    fn simultaneous_snapshots_own_their_indices_and_preserve_user_staging() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        init_repo(root).unwrap();
        std::fs::write(root.join("README.md"), "staged\n").unwrap();
        git(root, &["add", "README.md"]).unwrap();
        std::fs::write(root.join("README.md"), "working\n").unwrap();
        let index = std::fs::read(root.join(".git/index")).unwrap();
        let legacy = root.join(".git/ryter-undo-index");
        std::fs::write(&legacy, "belongs to somebody else").unwrap();
        let gate = std::sync::Barrier::new(8);
        let shas = std::thread::scope(|scope| {
            let jobs: Vec<_> = (0..8)
                .map(|i| {
                    let gate = &gate;
                    scope.spawn(move || {
                        gate.wait();
                        checkpoint(root, &format!("parallel-{i}")).unwrap().unwrap()
                    })
                })
                .collect();
            jobs.into_iter()
                .map(|j| j.join().unwrap())
                .collect::<Vec<_>>()
        });
        for sha in shas {
            assert_eq!(
                git(root, &["show", &format!("{sha}:README.md")]).unwrap(),
                "working\n"
            );
        }
        assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
        assert_eq!(
            std::fs::read_to_string(legacy).unwrap(),
            "belongs to somebody else"
        );
        // An invalid ref fails after creating the snapshot and must clean up too.
        assert!(checkpoint(root, "invalid name").is_err());
        assert!(
            !std::fs::read_dir(root.join(".git"))
                .unwrap()
                .flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with("ryter-tmp-"))
        );
    }

    #[test]
    fn nested_restore_uses_root_paths_and_preserves_unrelated_work() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        init_repo(root).unwrap();
        let app = root.join("app");
        std::fs::create_dir(&app).unwrap();
        for file in [
            "changed.txt",
            "deleted.txt",
            "literal[1].txt",
            "line\nbreak.txt",
        ] {
            std::fs::write(app.join(file), "before").unwrap();
        }
        let before = checkpoint(&app, "before").unwrap().unwrap();
        std::fs::write(app.join("changed.txt"), "after").unwrap();
        std::fs::write(app.join("literal[1].txt"), "after").unwrap();
        std::fs::write(app.join("line\nbreak.txt"), "after").unwrap();
        std::fs::remove_file(app.join("deleted.txt")).unwrap();
        std::fs::write(app.join("added.txt"), "added").unwrap();
        std::fs::write(root.join("sibling.txt"), "user work during turn").unwrap();
        let after = checkpoint(&app, "after").unwrap().unwrap();
        std::fs::write(app.join("unrelated.txt"), "later user work").unwrap();
        std::fs::write(root.join("sibling.txt"), "later sibling work").unwrap();
        git(root, &["add", "sibling.txt"]).unwrap();
        let index = git(root, &["ls-files", "--stage"]).unwrap();
        let branch_head = head(root).unwrap();
        // User configuration must not change the coordinate system.
        git(root, &["config", "diff.relative", "true"]).unwrap();
        let paths = paths_between(&app, &before, &after).unwrap();
        assert_eq!(paths.len(), 5, "{paths:?}");
        assert!(paths.iter().all(|p| p.starts_with("app/")));
        assert_eq!(restore_paths(&app, &before, &paths).unwrap(), 5);
        for file in [
            "changed.txt",
            "deleted.txt",
            "literal[1].txt",
            "line\nbreak.txt",
        ] {
            assert_eq!(std::fs::read_to_string(app.join(file)).unwrap(), "before");
        }
        assert!(!app.join("added.txt").exists());
        assert_eq!(restore_paths(&app, &after, &paths).unwrap(), 5);
        assert_eq!(
            std::fs::read_to_string(app.join("changed.txt")).unwrap(),
            "after"
        );
        assert!(!app.join("deleted.txt").exists());
        assert_eq!(
            std::fs::read_to_string(app.join("added.txt")).unwrap(),
            "added"
        );
        assert_eq!(
            std::fs::read_to_string(app.join("unrelated.txt")).unwrap(),
            "later user work"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("sibling.txt")).unwrap(),
            "later sibling work"
        );
        assert_eq!(git(root, &["ls-files", "--stage"]).unwrap(), index);
        assert_eq!(head(root).unwrap(), branch_head);
        assert!(restore_paths(&app, "invalid-snapshot", &["app/added.txt".into()]).is_err());
        assert!(
            restore_paths(
                &app,
                &before,
                &["app/added.txt".into(), "../outside".into()]
            )
            .is_err()
        );
        assert!(app.join("added.txt").exists());
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
    /// A folder of projects is never made into one repository: Ryter once
    /// did that to `~/workspace` and committed 29 repositories into it.
    #[test]
    fn a_folder_of_repositories_is_left_alone() {
        let top = TempDir::new().unwrap();
        let d = top.path();
        for app in ["alpha", "beta", "gamma", "delta"] {
            std::fs::create_dir_all(d.join(app)).unwrap();
            init_repo(&d.join(app)).unwrap();
        }
        std::fs::write(d.join("notes.txt"), "mine").unwrap();
        let err = ensure_repo(d).unwrap_err().to_string();
        assert!(
            err.contains("won't set up git in")
                && err.contains("alpha, beta, delta, and 1 more")
                && err.contains("project's own folder")
                && err.contains("add their folder to .gitignore"),
            "{err}"
        );
        assert!(!d.join(".git").exists(), "nothing was created");
        assert!(!d.join(".gitignore").exists());

        // One level further down counts too: `clients/acme` is a project.
        let top = TempDir::new().unwrap();
        let d = top.path();
        std::fs::create_dir_all(d.join("clients/acme")).unwrap();
        init_repo(&d.join("clients/acme")).unwrap();
        assert!(
            ensure_repo(d)
                .unwrap_err()
                .to_string()
                .contains("clients/acme")
        );

        // A repository with no commits yet gets no first commit either.
        git(d, &["init", "-q"]).unwrap();
        let err = ensure_repo(d).unwrap_err().to_string();
        assert!(err.contains("won't make the first commit in"), "{err}");
        assert!(head(d).is_err(), "no commit was made");

        // A hidden folder is searched: its repository would be swept in.
        let top = TempDir::new().unwrap();
        let d = top.path();
        std::fs::create_dir_all(d.join(".cache/tool")).unwrap();
        init_repo(&d.join(".cache/tool")).unwrap();
        assert!(
            ensure_repo(d)
                .unwrap_err()
                .to_string()
                .contains(".cache/tool")
        );

        // Dependency trees the first `.gitignore` leaves out are not projects,
        // when Ryter writes it: there was no `.gitignore`.
        let top = TempDir::new().unwrap();
        let d = top.path();
        for dep in ["node_modules/pkg", "target/x", ".venv/src"] {
            std::fs::create_dir_all(d.join(dep)).unwrap();
            init_repo(&d.join(dep)).unwrap();
        }
        std::fs::write(d.join("main.py"), "print(1)").unwrap();
        assert!(ensure_repo(d).unwrap().unwrap().created);
        assert!(!gitlinks(d), "no repository was committed");
    }

    /// Whether HEAD's tree holds another repository (a gitlink).
    fn gitlinks(d: &Path) -> bool {
        git(d, &["ls-tree", "-r", "HEAD"])
            .unwrap()
            .lines()
            .any(|l| l.starts_with("160000"))
    }

    /// The folder's own ignore rules decide what counts, not folder names.
    /// A reviewer's two cases: with a `.gitignore` of only `*.log`, a
    /// repository in `node_modules` or `target` was committed into the new
    /// parent, because those names were skipped whatever the rules said.
    #[test]
    fn the_folders_own_ignore_rules_decide_what_counts() {
        // A new parent: its `.gitignore` stays, so the template doesn't apply.
        let top = TempDir::new().unwrap();
        let d = top.path();
        std::fs::write(d.join(".gitignore"), "*.log\n").unwrap();
        std::fs::create_dir_all(d.join("node_modules/pkg")).unwrap();
        init_repo(&d.join("node_modules/pkg")).unwrap();
        assert_eq!(holds_repos(d), ["node_modules/pkg"]);
        assert!(ensure_repo(d).is_err());
        assert!(!d.join(".git").exists());

        // An unborn parent, the same `.gitignore`, a repository in `target`.
        let top = TempDir::new().unwrap();
        let d = top.path();
        git(d, &["init", "-q"]).unwrap();
        std::fs::write(d.join(".gitignore"), "*.log\n").unwrap();
        std::fs::create_dir_all(d.join("target/pkg")).unwrap();
        init_repo(&d.join("target/pkg")).unwrap();
        assert_eq!(holds_repos(d), ["target/pkg"]);
        assert!(ensure_repo(d).is_err());
        assert!(head(d).is_err(), "no first commit");

        // Folders the folder's rules ignore are left out, at either level
        // and from `.git/info/exclude`, and the commit holds no repository.
        let top = TempDir::new().unwrap();
        let d = top.path();
        git(d, &["init", "-q"]).unwrap();
        std::fs::write(d.join(".gitignore"), "node_modules/\n").unwrap();
        std::fs::write(d.join(".git/info/exclude"), "vendored/\n").unwrap();
        std::fs::create_dir_all(d.join("web")).unwrap();
        std::fs::write(d.join("web/.gitignore"), "cache/\n").unwrap();
        for dep in ["node_modules/pkg", "vendored/lib", "web/cache"] {
            std::fs::create_dir_all(d.join(dep)).unwrap();
            init_repo(&d.join(dep)).unwrap();
        }
        std::fs::write(d.join("web/app.js"), "1").unwrap();
        assert!(holds_repos(d).is_empty(), "{:?}", holds_repos(d));
        assert!(ensure_repo(d).unwrap().is_some());
        assert!(!gitlinks(d), "no repository was committed");
    }

    /// A nested `.gitignore` outranks the root's in git, Ryter's template
    /// included: the second review's case. `web/.gitignore` with
    /// `!target/` put `web/target`, a repository, in the first commit.
    #[test]
    fn a_nested_negation_outranks_the_template() {
        let top = TempDir::new().unwrap();
        let d = top.path();
        std::fs::create_dir_all(d.join("web/target")).unwrap();
        std::fs::write(d.join("web/.gitignore"), "!target/\n").unwrap();
        init_repo(&d.join("web/target")).unwrap();
        assert_eq!(holds_repos(d), ["web/target"]);
        assert!(
            ensure_repo(d)
                .unwrap_err()
                .to_string()
                .contains("web/target")
        );
        assert!(!d.join(".git").exists() && !d.join(".gitignore").exists());
    }

    /// Git has the last word before the first commit: a repository deeper
    /// than the walk looks is still refused, and only what Ryter did is
    /// undone. The user's own repository and staging area are left as they
    /// were.
    #[test]
    fn git_itself_checks_the_first_commit() {
        // A new parent: the repository and `.gitignore` Ryter made go again.
        let top = TempDir::new().unwrap();
        let d = top.path();
        std::fs::create_dir_all(d.join("a/b/c")).unwrap();
        init_repo(&d.join("a/b/c")).unwrap();
        assert!(holds_repos(d).is_empty(), "deeper than the walk");
        let err = ensure_repo(d).unwrap_err().to_string();
        assert!(
            err.contains("won't set up git in") && err.contains("a/b/c"),
            "{err}"
        );
        assert!(!d.join(".git").exists() && !d.join(".gitignore").exists());

        // An unborn repository of the user's, with something staged.
        let top = TempDir::new().unwrap();
        let d = top.path();
        git(d, &["init", "-q"]).unwrap();
        std::fs::write(d.join("mine.txt"), "staged").unwrap();
        git(d, &["add", "mine.txt"]).unwrap();
        std::fs::create_dir_all(d.join("x/y/z")).unwrap();
        init_repo(&d.join("x/y/z")).unwrap();
        let err = ensure_repo(d).unwrap_err().to_string();
        assert!(
            err.contains("won't make the first commit in") && err.contains("x/y/z"),
            "{err}"
        );
        assert!(head(d).is_err(), "no commit");
        assert_eq!(
            git(d, &["diff", "--cached", "--name-only"]).unwrap(),
            "mine.txt\n"
        );
        assert!(!d.join(".gitignore").exists());
        let git_dir = d.join(".git");
        assert!(
            !std::fs::read_dir(&git_dir)
                .unwrap()
                .flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with("ryter-")),
            "the private index is removed"
        );
    }

    /// A repository the user staged by hand in an unborn repository is in
    /// the first commit whatever the ignore rules say: the third review's
    /// case. The check began from an empty index, missed it, and the first
    /// commit held `target/pkg`. Refused now, and the user's index keeps it.
    #[test]
    fn a_repository_staged_by_hand_is_seen() {
        let top = TempDir::new().unwrap();
        let d = top.path();
        git(d, &["init", "-q"]).unwrap();
        std::fs::create_dir_all(d.join("target/pkg")).unwrap();
        init_repo(&d.join("target/pkg")).unwrap();
        git(d, &["add", "target/pkg"]).unwrap();
        std::fs::write(d.join(".gitignore"), "target/\n").unwrap();
        let staged = || git(d, &["ls-files", "-s"]).unwrap();
        let before = staged();
        assert!(before.starts_with("160000") && before.contains("target/pkg"));
        assert_eq!(
            holds_repos(d),
            ["target/pkg"],
            "found before anything is staged"
        );
        // `ensure_repo` refuses on git's check alone, not only the walk.
        assert_eq!(repos_to_add(d).unwrap(), ["target/pkg"]);
        let err = ensure_repo(d).unwrap_err().to_string();
        assert!(
            err.contains("won't make the first commit in") && err.contains("target/pkg"),
            "{err}"
        );
        assert!(head(d).is_err(), "no commit");
        assert_eq!(staged(), before, "the user's index is as it was");
    }

    /// When git's check can't run, setup is refused and undone like any
    /// other refusal. A repository with no commit, deeper than the walk,
    /// makes `git add` fail, and that left the new `.git` and `.gitignore`.
    #[test]
    fn a_check_that_cannot_run_leaves_nothing_behind() {
        let top = TempDir::new().unwrap();
        let d = top.path();
        std::fs::create_dir_all(d.join("a/b/c")).unwrap();
        git(&d.join("a/b/c"), &["init", "-q"]).unwrap();
        std::fs::write(d.join("a/b/c/f.txt"), "x").unwrap();
        std::fs::write(d.join("main.py"), "print(1)").unwrap();
        assert!(holds_repos(d).is_empty(), "deeper than the walk");
        let err = ensure_repo(d).unwrap_err().to_string();
        assert!(err.contains("Nothing was left behind"), "{err}");
        assert!(!d.join(".git").exists() && !d.join(".gitignore").exists());
    }

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
