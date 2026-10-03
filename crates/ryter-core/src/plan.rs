//! Approved plans: `.ryter/plans/` in the project.
//!
//! A plan the user approved is a file, not a message. The build works from
//! it, a later review can hold the work against it, and the user decides
//! whether it is committed. One file per plan, named for the day and the
//! plan's title; an earlier plan is never written over.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// The folder, under the project's top.
pub const DIR: &str = ".ryter/plans";
/// The approved plan, at a path every hat knows: the latest one, replaced
/// by each approval. The dated copies under [`DIR`] are kept.
pub const FILE: &str = ".ryter/plan.md";

/// The most a plan may be. One the user has to approve is one they can read.
pub const MAX_BYTES: usize = 64 * 1024;

/// `title` as a file name: lowercase words joined by dashes.
fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 48 {
            break;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "plan".into() } else { out }
}

/// The plan as the file holds it: under its title, ending in one newline.
fn document(title: &str, plan: &str) -> String {
    let plan = plan.trim();
    // A plan that brings its own title keeps it.
    if plan.starts_with("# ") {
        format!("{plan}\n")
    } else {
        format!("# {}\n\n{plan}\n", title.trim())
    }
}

/// A folder Ryter keeps its own files in: `rel` under the project's top
/// (`.ryter`, `.ryter/plans`), made if it isn't there. Every folder on the
/// way is the project's own: one that is a link would send what Ryter
/// writes somewhere else, so it is refused.
pub fn own_dir(root: &Path, rel: &str) -> Result<PathBuf> {
    let mut dir = root.to_path_buf();
    for part in Path::new(rel).components() {
        dir.push(part);
        match dir.symlink_metadata() {
            Ok(m) if m.file_type().is_dir() => {}
            Ok(_) => {
                return Err(Error::Io(format!(
                    "{} is not a folder of the project's own (a link, or a file): Ryter \
                     writes its plans, decisions and reports in the project itself",
                    dir.display()
                )));
            }
            Err(_) => std::fs::create_dir(&dir)
                .map_err(|e| Error::Io(format!("{}: {e}", dir.display())))?,
        }
    }
    Ok(dir)
}

/// The same folder, when it is there and every folder on the way is the
/// project's own. Nothing is made.
pub fn own_dir_there(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut dir = root.to_path_buf();
    for part in Path::new(rel).components() {
        dir.push(part);
        if !dir.symlink_metadata().is_ok_and(|m| m.file_type().is_dir()) {
            return None;
        }
    }
    Some(dir)
}

/// `rel` as a folder and a file name under the project's top, when it is
/// made of plain names only: no `..`, nothing absolute.
fn parts(rel: &str) -> Option<(Vec<String>, String)> {
    let mut names = Vec::new();
    for part in Path::new(rel).components() {
        match part {
            std::path::Component::Normal(n) => names.push(n.to_string_lossy().into_owned()),
            _ => return None,
        }
    }
    let name = names.pop()?;
    Some((names, name))
}

/// Write one of Ryter's own files: `rel` under the project's top
/// (`.ryter/run.toml`). Returns the file.
///
/// Nothing is written through a link. Each folder on the way is opened as
/// a folder of the project's own, refusing a link, and the next is opened
/// from that one: a folder swapped for a link while this runs is not
/// followed either. The text goes to a new file there, one that did not
/// exist until this made it, and that file is moved into place: a link at
/// the name is replaced, not followed. A fixed temporary name was
/// followed: a project that came with `.ryter/run.toml.tmp` linked to a
/// file of the user's had that file overwritten.
///
/// Ryter's own files are written where a sandbox profile doesn't reach
/// (see [`crate::outside`]): they are its record of the work, not the work
/// of a command.
pub fn write_own(root: &Path, rel: &str, text: &str) -> Result<PathBuf> {
    let (root, rel, text) = (root.to_path_buf(), rel.to_string(), text.to_string());
    crate::outside::run(move || place(&root, &rel, &text, true).map(|(path, _)| path))
}

/// Save text under `rel_dir` as `base.md`, or the next free number
/// (`base-2.md`): an earlier file is never written over, by this or by
/// another session saving at the same moment. Returns the file.
pub fn save_new(root: &Path, rel_dir: &str, base: &str, text: &str) -> Result<PathBuf> {
    let (root, rel_dir, base, text) = (
        root.to_path_buf(),
        rel_dir.to_string(),
        base.to_string(),
        text.to_string(),
    );
    crate::outside::run(move || {
        let mut n = 1;
        loop {
            let name = if n == 1 {
                format!("{base}.md")
            } else {
                format!("{base}-{n}.md")
            };
            match place(&root, &format!("{rel_dir}/{name}"), &text, false)? {
                (path, true) => return Ok(path),
                (_, false) => n += 1,
            }
        }
    })
}

/// Put `text` at `rel` under `root`. With `replace`, whatever has the name
/// is replaced; without, the name has to be free, and `false` comes back
/// when it isn't.
#[cfg(unix)]
fn place(root: &Path, rel: &str, text: &str, replace: bool) -> Result<(PathBuf, bool)> {
    use rustix::fs::{AtFlags, CWD, Mode, OFlags};
    use std::io::Write;
    let path = root.join(rel);
    let io = |e: std::io::Error| Error::Io(format!("{}: {e}", path.display()));
    let Some((folders, name)) = parts(rel) else {
        return Err(Error::Io(format!("{rel}: not a file of Ryter's own")));
    };
    let folder = OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::RDONLY;
    let mut dir = rustix::fs::openat(
        CWD,
        root,
        OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| io(e.into()))?;
    let mut at = root.to_path_buf();
    for part in &folders {
        at.push(part);
        let not_ours = |e: rustix::io::Errno| {
            Error::Io(format!(
                "{} is not a folder of the project's own (a link, or a file): Ryter writes \
                 its plans, decisions and reports in the project itself ({e})",
                at.display()
            ))
        };
        let next = match rustix::fs::openat(&dir, part.as_str(), folder, Mode::empty()) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) => {
                match rustix::fs::mkdirat(&dir, part.as_str(), Mode::from_raw_mode(0o755)) {
                    Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                    Err(e) => return Err(io(e.into())),
                }
                rustix::fs::openat(&dir, part.as_str(), folder, Mode::empty()).map_err(not_ours)?
            }
            Err(e) => return Err(not_ours(e)),
        };
        dir = next;
    }
    // A name nothing has: made new, or not at all.
    let mut made = None;
    for n in 0..64u32 {
        let tmp = format!(".{name}.{}-{n}.new", std::process::id());
        match rustix::fs::openat(
            &dir,
            tmp.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o644),
        ) {
            Ok(fd) => {
                made = Some((std::fs::File::from(fd), tmp));
                break;
            }
            Err(rustix::io::Errno::EXIST) => {}
            Err(e) => return Err(io(e.into())),
        }
    }
    let Some((mut file, tmp)) = made else {
        return Err(Error::Io(format!(
            "{}: no free name to write it under",
            path.display()
        )));
    };
    let written = file
        .write_all(text.as_bytes())
        .and_then(|()| file.sync_all());
    let done = written.and_then(|()| {
        if replace {
            rustix::fs::renameat(&dir, tmp.as_str(), &dir, name.as_str())
                .map(|()| true)
                .map_err(std::io::Error::from)
        } else {
            // A second name for the file, which fails if the name is
            // taken, where moving it there would replace what has it.
            let linked =
                rustix::fs::linkat(&dir, tmp.as_str(), &dir, name.as_str(), AtFlags::empty());
            let _ = rustix::fs::unlinkat(&dir, tmp.as_str(), AtFlags::empty());
            match linked {
                Ok(()) => Ok(true),
                Err(rustix::io::Errno::EXIST) => Ok(false),
                Err(e) => Err(e.into()),
            }
        }
    });
    match done {
        Ok(placed) => Ok((path, placed)),
        Err(e) => {
            let _ = rustix::fs::unlinkat(&dir, tmp.as_str(), AtFlags::empty());
            Err(io(e))
        }
    }
}

#[cfg(not(unix))]
fn place(root: &Path, rel: &str, text: &str, replace: bool) -> Result<(PathBuf, bool)> {
    use std::io::Write;
    let Some((folders, name)) = parts(rel) else {
        return Err(Error::Io(format!("{rel}: not a file of Ryter's own")));
    };
    let dir = own_dir(root, &folders.join("/"))?;
    let path = dir.join(&name);
    let io = |e: std::io::Error| Error::Io(format!("{}: {e}", path.display()));
    let mut open = std::fs::OpenOptions::new();
    open.write(true);
    if replace {
        open.create(true).truncate(true);
    } else {
        open.create_new(true);
    }
    match open.open(&path) {
        Ok(mut f) => f
            .write_all(text.as_bytes())
            .map(|()| (path.clone(), true))
            .map_err(io),
        Err(e) if !replace && e.kind() == std::io::ErrorKind::AlreadyExists => Ok((path, false)),
        Err(e) => Err(io(e)),
    }
}

/// Read one of Ryter's own files: `rel` under the project's top. Only a
/// file of the project's own is read: a link there (to a key, say) is what
/// somebody else put in the project, and what it points at is not Ryter's
/// to show anyone.
pub fn read_own(root: &Path, rel: &str) -> std::io::Result<String> {
    let rel = Path::new(rel);
    let not_ours = |what: &str| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{}: {what}", root.join(rel).display()),
        )
    };
    if parts(&rel.to_string_lossy()).is_none() {
        return Err(not_ours("not a file of Ryter's own"));
    }
    let (Some(folder), Some(name)) = (rel.parent(), rel.file_name()) else {
        return Err(not_ours("not a file name"));
    };
    let mut dir = root.to_path_buf();
    for part in folder.components() {
        dir.push(part);
        match dir.symlink_metadata() {
            Ok(m) if m.file_type().is_dir() => {}
            Ok(_) => return Err(not_ours("a folder on the way is a link")),
            Err(e) => return Err(e),
        }
    }
    let path = dir.join(name);
    match path.symlink_metadata() {
        Ok(m) if m.file_type().is_file() => std::fs::read_to_string(&path),
        Ok(_) => Err(not_ours("a link, or not a file")),
        Err(e) => Err(e),
    }
}

/// Save an approved plan under `root`, on `day` (`YYYY-MM-DD`). Returns the
/// file. A plan of the same name on the same day gets a number: `-2`, `-3`.
pub fn save_on(root: &Path, day: &str, title: &str, plan: &str) -> Result<PathBuf> {
    let text = document(title, plan);
    let dated = save_new(root, DIR, &format!("{day}-{}", slug(title)), &text)?;
    // And at the fixed path, which every hat reads (`docs/specialists-design.md`
    // R-PLAN-01): the latest approval, replaced by the next.
    write_own(root, FILE, &text)?;
    Ok(dated)
}

/// The plan on record in the project when a session has none: [`FILE`] as
/// a project-relative path, or the dated copy under [`DIR`] with the same
/// text when there is one, so the decisions recorded under it are found.
/// `None` when there is no `plan.md`.
pub fn on_record(root: &Path) -> Option<String> {
    let text = read_own(root, FILE).ok()?;
    let dated = std::fs::read_dir(root.join(DIR)).ok().and_then(|dir| {
        let mut same: Vec<String> = dir
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let rel = format!("{DIR}/{name}");
                (name.ends_with(".md") && read_own(root, &rel).ok()? == text).then_some(rel)
            })
            .collect();
        // The latest of several, by name (they start with the day).
        same.sort();
        same.pop()
    });
    Some(dated.unwrap_or_else(|| FILE.to_string()))
}

/// [`save_on`] today.
pub fn save(root: &Path, title: &str, plan: &str) -> Result<PathBuf> {
    // The user's own date: a plan approved in the evening was filed under
    // tomorrow's.
    save_on(root, &crate::clock::today(), title, plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Each approval replaces `plan.md` and adds a dated copy; the plan on
    /// record is the dated copy with the same text, or `plan.md` alone.
    #[test]
    fn the_latest_plan_is_at_a_fixed_path_beside_its_dated_copy() {
        let root = TempDir::new().unwrap();
        assert_eq!(on_record(root.path()), None);
        let first = save_on(root.path(), "2026-10-01", "First", "do a").unwrap();
        let fixed = root.path().join(FILE);
        assert_eq!(
            std::fs::read_to_string(&fixed).unwrap(),
            std::fs::read_to_string(&first).unwrap()
        );
        assert_eq!(
            on_record(root.path()).as_deref(),
            Some(".ryter/plans/2026-10-01-first.md")
        );
        let second = save_on(root.path(), "2026-10-02", "Second", "do b").unwrap();
        assert_eq!(
            std::fs::read_to_string(&fixed).unwrap(),
            std::fs::read_to_string(&second).unwrap()
        );
        assert!(first.exists(), "the dated copy is kept");
        assert_eq!(
            on_record(root.path()).as_deref(),
            Some(".ryter/plans/2026-10-02-second.md")
        );
        // Edited by hand, or copied in: `plan.md` on its own is the plan.
        std::fs::write(&fixed, "# By hand\n\nnothing dated matches\n").unwrap();
        assert_eq!(on_record(root.path()).as_deref(), Some(FILE));
    }

    #[test]
    fn a_plan_is_saved_under_its_day_and_title() {
        let root = TempDir::new().unwrap();
        let path = save_on(
            root.path(),
            "2026-10-01",
            "Add CSV export to reports!",
            "## Goal\nReports download as CSV.\n\n## Steps\n1. Add export_csv()\n",
        )
        .unwrap();
        assert_eq!(
            path,
            root.path()
                .join(".ryter/plans/2026-10-01-add-csv-export-to-reports.md")
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# Add CSV export to reports!\n\n## Goal\nReports download as CSV.\n\n## Steps\n1. Add export_csv()\n"
        );
        // The same title again that day is kept beside it, not over it.
        let again = save_on(
            root.path(),
            "2026-10-01",
            "Add CSV export to reports",
            "second",
        )
        .unwrap();
        assert!(again.ends_with("2026-10-01-add-csv-export-to-reports-2.md"));
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("export_csv")
        );
        // A plan with its own title keeps it; a title of no letters still names a file.
        let own = save_on(root.path(), "2026-10-02", "???", "# Mine\n\nbody").unwrap();
        assert!(own.ends_with("2026-10-02-plan.md"));
        assert_eq!(std::fs::read_to_string(&own).unwrap(), "# Mine\n\nbody\n");
    }

    /// Ryter's own files are written in the project and nowhere else. A
    /// project can arrive with links where those files go: every name Ryter
    /// writes under, the temporary one included, and every folder on the
    /// way. The run file was written to `.ryter/run.toml.tmp` first, and a
    /// link by that name sent the text into the file it pointed at.
    #[cfg(unix)]
    #[test]
    fn own_files_are_never_written_or_read_through_a_link() {
        use std::os::unix::fs::symlink;
        let root = TempDir::new().unwrap();
        let elsewhere = TempDir::new().unwrap();
        let theirs = |name: &str| {
            let f = elsewhere.path().join(name);
            std::fs::write(&f, "not Ryter's\n").unwrap();
            f
        };
        let untouched = |f: &Path| assert_eq!(std::fs::read_to_string(f).unwrap(), "not Ryter's\n");
        let own = root.path().join(".ryter");
        std::fs::create_dir(&own).unwrap();
        // The name the file is saved under, the old temporary name, and the
        // first temporary name this process would try.
        let (a, b, c) = (theirs("a"), theirs("b"), theirs("c"));
        symlink(&a, own.join("run.toml")).unwrap();
        symlink(&b, own.join("run.toml.tmp")).unwrap();
        let first = own.join(format!(".run.toml.{}-0.new", std::process::id()));
        symlink(&c, &first).unwrap();
        let path = write_own(root.path(), ".ryter/run.toml", "start = \"make up\"\n").unwrap();
        for f in [&a, &b, &c] {
            untouched(f);
        }
        assert!(path.symlink_metadata().unwrap().file_type().is_file());
        assert_eq!(
            read_own(root.path(), ".ryter/run.toml").unwrap(),
            "start = \"make up\"\n"
        );
        // Nothing of its own is left beside it.
        let left: Vec<String> = std::fs::read_dir(&own)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".new") && own.join(n) != first)
            .collect();
        assert!(left.is_empty(), "{left:?}");
        // A link is not read: what it points at is not Ryter's to show.
        symlink(&a, own.join("decisions.md")).unwrap();
        assert_eq!(
            read_own(root.path(), ".ryter/decisions.md")
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidInput
        );
        // A folder on the way that is a link: nothing is written there.
        let folder = elsewhere.path().join("folder");
        std::fs::create_dir(&folder).unwrap();
        symlink(&folder, own.join("plans")).unwrap();
        assert!(save_on(root.path(), "2026-10-01", "A plan", "body").is_err());
        assert!(std::fs::read_dir(&folder).unwrap().next().is_none());
        assert!(own_dir_there(root.path(), DIR).is_none());
        // A name taken by a link with nothing behind it is not reused.
        std::fs::remove_file(own.join("plans")).unwrap();
        std::fs::create_dir(own.join("plans")).unwrap();
        symlink(
            elsewhere.path().join("gone"),
            own.join("plans/2026-10-01-a-plan.md"),
        )
        .unwrap();
        let saved = save_on(root.path(), "2026-10-01", "A plan", "body").unwrap();
        assert!(saved.ends_with("2026-10-01-a-plan-2.md"), "{saved:?}");
        assert!(!elsewhere.path().join("gone").exists());
        // Only a name under the project is one of Ryter's own files.
        for rel in ["../theirs", "/etc/hostname", ".ryter/../../x"] {
            assert!(read_own(root.path(), rel).is_err(), "{rel}");
            assert!(write_own(root.path(), rel, "x").is_err(), "{rel}");
        }
        // The project's own folder itself.
        let linked = TempDir::new().unwrap();
        symlink(&folder, linked.path().join(".ryter")).unwrap();
        assert!(write_own(linked.path(), ".ryter/run.toml", "x").is_err());
        assert!(save_on(linked.path(), "2026-10-01", "A plan", "body").is_err());
        assert!(std::fs::read_dir(&folder).unwrap().next().is_none());
    }

    /// An earlier file is never written over, by two sessions saving the
    /// same plan at the same moment either: each gets a name of its own.
    #[test]
    fn two_savers_never_take_the_same_name() {
        let root = TempDir::new().unwrap();
        let savers: Vec<_> = (0..8)
            .map(|i| {
                let root = root.path().to_path_buf();
                std::thread::spawn(move || {
                    (0..25)
                        .map(|j| {
                            save_on(&root, "2026-10-02", "Same plan", &format!("{i}-{j}")).unwrap()
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut saved: Vec<PathBuf> = savers.into_iter().flat_map(|s| s.join().unwrap()).collect();
        saved.sort();
        saved.dedup();
        assert_eq!(saved.len(), 200, "two saves were given one name");
        let kept = std::fs::read_dir(root.path().join(DIR)).unwrap().count();
        assert_eq!(
            kept, 200,
            "a plan was written over, or a temporary file left"
        );
    }
}
