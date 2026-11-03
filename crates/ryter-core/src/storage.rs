//! What Ryter keeps for a project, how big and how old it is, and a cleanup
//! that is shown in full before anything is removed.
//!
//! Three things grow: the sessions under `~/.ryter/sessions/`, the recovery
//! copies a resume leaves beside a repaired log, and the checkpoints under
//! `refs/ryter/undo/` in the project's repository. Nothing here runs on its
//! own: [`report`] reads, [`plan`] says what one cleanup would remove and
//! what that costs, and [`apply`] removes it after the user said yes, after
//! reading everything again.
//!
//! A checkpoint is the user's way back. One is kept while any session in
//! this Ryter home names its commit (for `/undo`, `/redo`, "last turn", an
//! audit's rollback), while it is less than a day old (another Ryter may be
//! in the middle of a turn whose checkpoint only its process knows), and
//! for the two working names `review` and `now`, which are rewritten on
//! every use. Only the rest can go. Refs outside `refs/ryter/undo/` are
//! never touched, and the objects are left for git to collect: Ryter does
//! not run `git gc`, which would also prune what the user left dangling.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::{Error, Result};

/// One day, in milliseconds.
pub const DAY_MS: u64 = 86_400_000;

/// A checkpoint younger than this is kept, whatever names it.
pub const RECENT_MS: u64 = DAY_MS;

/// The ages the session cleanup can be set to, in days.
pub const AGES: [u32; 4] = [7, 30, 90, 180];

/// The age the session cleanup starts at.
pub const DEFAULT_AGE: u32 = 30;

/// Where checkpoints live. Nothing outside it is ever removed.
const PREFIX: &str = "refs/ryter/undo/";

/// Checkpoint names rewritten on every use: `/changes` and a review measure
/// against `review`, `/redo` against `now`.
const WORKING: [&str; 2] = ["review", "now"];

/// How many checkpoints a session keeps for `/undo`.
pub const UNDO_KEPT: usize = 50;

/// One saved session of the project.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionUse {
    /// Full id.
    pub id: String,
    /// Title, or `untitled`.
    pub title: String,
    /// Its folder.
    pub dir: PathBuf,
    /// Bytes in its folder and its pages.
    pub bytes: u64,
    /// Last write, unix ms, as its index says it.
    pub updated_ms: Option<u64>,
    /// What it cost, as far as it was priced.
    pub spend_usd: f64,
}

impl SessionUse {
    /// The first eight characters of the id.
    pub fn short(&self) -> String {
        self.id.chars().take(8).collect()
    }
}

/// Why a checkpoint stays, or that it can go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hold {
    /// A session names its commit: `/undo`, `/redo` or an audit can use it.
    InUse,
    /// Made less than a day ago.
    Recent,
    /// `review` or `now`, rewritten on every use.
    Working,
    /// No session can reach it.
    Free,
}

/// One ref under `refs/ryter/undo/`.
#[derive(Debug, Clone, PartialEq)]
pub struct Checkpoint {
    /// The full ref name.
    pub name: String,
    /// The commit it points at.
    pub sha: String,
    /// When the commit was made, unix ms.
    pub made_ms: Option<u64>,
    /// The session its name carries, if it carries one.
    pub owner: Option<String>,
    /// Whether it stays.
    pub hold: Hold,
}

/// A recovery copy left beside a repaired log.
#[derive(Debug, Clone, PartialEq)]
pub struct Copy {
    /// The file.
    pub path: PathBuf,
    /// Its size.
    pub bytes: u64,
}

/// What Ryter keeps for one project.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// The project: its repository's top folder, or the folder.
    pub root: PathBuf,
    /// When this was read, unix ms.
    pub now_ms: u64,
    /// The project's sessions (its folder and those under it), newest
    /// first.
    pub sessions: Vec<SessionUse>,
    /// Recovery copies in those sessions.
    pub copies: Vec<Copy>,
    /// The checkpoints, oldest first. `None` outside a repository.
    pub checkpoints: Option<Vec<Checkpoint>>,
    /// Why there are none when `checkpoints` is `None`: no repository, or
    /// no git to read one with.
    pub repo: crate::git::RepoState,
    /// Bytes only the checkpoints hold, as git counts them on disk. `None`
    /// when this git can't say (before 2.38) or there is no repository.
    pub checkpoint_bytes: Option<u64>,
    /// Every session in this Ryter home, any project.
    pub all_sessions: usize,
    /// Their bytes, pages included.
    pub all_bytes: u64,
    /// The commits each session names, by session id.
    names: BTreeMap<String, BTreeSet<String>>,
    /// A session whose index could not be read, or was written by a newer
    /// Ryter: what it names is unknown, so every checkpoint without an
    /// owner, or owned by it, is held.
    unread: BTreeSet<String>,
}

impl Report {
    /// The checkpoints, or none.
    pub fn refs(&self) -> &[Checkpoint] {
        self.checkpoints.as_deref().unwrap_or(&[])
    }

    /// How many checkpoints are held for `hold`.
    pub fn count(&self, hold: Hold) -> usize {
        self.refs().iter().filter(|c| c.hold == hold).count()
    }

    /// Bytes in the project's sessions.
    pub fn session_bytes(&self) -> u64 {
        self.sessions.iter().map(|s| s.bytes).sum()
    }

    /// The oldest last use among the project's sessions.
    pub fn oldest_ms(&self) -> Option<u64> {
        self.sessions.iter().filter_map(|s| s.updated_ms).min()
    }

    /// The figures, as `(what, how much)` rows: the same in `/storage` and
    /// `ryter storage`.
    pub fn rows(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut sessions = format!(
            "{} · {}",
            self.sessions.len(),
            format_bytes(self.session_bytes())
        );
        if let Some(t) = self.oldest_ms() {
            sessions.push_str(&format!(" · oldest last used {}", ago(self.now_ms, t)));
        }
        out.push(("sessions".into(), sessions));
        if !self.copies.is_empty() {
            out.push((
                "recovery copies".into(),
                format!(
                    "{} · {}",
                    self.copies.len(),
                    format_bytes(self.copies.iter().map(|c| c.bytes).sum())
                ),
            ));
        }
        match &self.checkpoints {
            None => out.push((
                "checkpoints".into(),
                format!("none: {}", self.repo.reason()),
            )),
            Some(refs) => {
                let mut line = refs.len().to_string();
                if !refs.is_empty() {
                    line.push_str(&match self.checkpoint_bytes {
                        Some(b) => format!(" · about {} in git of their own", format_bytes(b)),
                        None => " · this git can't say their size".into(),
                    });
                }
                out.push(("checkpoints".into(), line));
                let parts: Vec<String> = [
                    (Hold::InUse, "a session reaches"),
                    (Hold::Recent, "made in the last day"),
                    (Hold::Working, "working"),
                    (Hold::Free, "no session reaches"),
                ]
                .iter()
                .filter_map(|(h, what)| {
                    let n = self.count(*h);
                    (n > 0).then(|| format!("{n} {what}"))
                })
                .collect();
                if !parts.is_empty() {
                    out.push((String::new(), parts.join(" · ")));
                }
            }
        }
        out.push((
            "all projects".into(),
            format!(
                "{} · {}",
                count(self.all_sessions, "session"),
                format_bytes(self.all_bytes)
            ),
        ));
        out
    }

    /// Why `c` stays with the sessions in `gone` removed.
    fn hold_without(&self, c: &Checkpoint, gone: &BTreeSet<String>) -> Hold {
        let short = c.name.strip_prefix(PREFIX).unwrap_or(&c.name);
        if WORKING.contains(&short) {
            return Hold::Working;
        }
        if c.made_ms
            .is_none_or(|t| self.now_ms.saturating_sub(t) < RECENT_MS)
        {
            return Hold::Recent;
        }
        let named = self
            .names
            .iter()
            .any(|(id, shas)| !gone.contains(id) && shas.contains(&c.sha));
        let unread = !self.unread.is_empty()
            && c.owner
                .as_ref()
                .is_none_or(|o| self.unread.contains(o) && !gone.contains(o));
        if named || unread {
            Hold::InUse
        } else {
            Hold::Free
        }
    }
}

/// A cleanup the user can choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cleanup {
    /// Checkpoints no session can reach.
    Unreachable,
    /// Sessions last used more than this many days ago, with the
    /// checkpoints only they reach.
    OlderThan(u32),
    /// Recovery copies.
    Copies,
}

/// What one cleanup would remove, to be shown before it is done.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// Which cleanup.
    pub cleanup: Cleanup,
    /// The project.
    pub root: PathBuf,
    /// Sessions to remove.
    pub sessions: Vec<SessionUse>,
    /// Checkpoints to remove.
    pub checkpoints: Vec<Checkpoint>,
    /// Recovery copies to remove.
    pub copies: Vec<Copy>,
    /// Bytes only these checkpoints hold, when git can say.
    pub checkpoint_bytes: Option<u64>,
    /// Checkpoints that stay, by why.
    pub kept: Vec<(Hold, usize)>,
    /// The session the cleanup leaves alone because it is open.
    pub current: Option<String>,
}

impl Plan {
    /// Nothing to remove.
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty() && self.checkpoints.is_empty() && self.copies.is_empty()
    }

    /// Bytes in the files it removes.
    pub fn file_bytes(&self) -> u64 {
        self.sessions.iter().map(|s| s.bytes).sum::<u64>()
            + self.copies.iter().map(|c| c.bytes).sum::<u64>()
    }

    /// What the sessions it removes cost, which the project's total keeps.
    pub fn spend_usd(&self) -> f64 {
        self.sessions.iter().map(|s| s.spend_usd).sum()
    }

    /// The row's one-line name.
    pub fn label(&self) -> String {
        match self.cleanup {
            Cleanup::Unreachable => "checkpoints no session reaches".into(),
            Cleanup::OlderThan(days) => format!("sessions unused for {days} days"),
            Cleanup::Copies => "recovery copies".into(),
        }
    }

    /// How much: `4 sessions, 22 checkpoints · 1.3 MB + about 2.9 MB`.
    pub fn amount(&self) -> String {
        if self.is_empty() {
            return "nothing".into();
        }
        let mut parts = Vec::new();
        if !self.sessions.is_empty() {
            parts.push(count(self.sessions.len(), "session"));
        }
        if !self.copies.is_empty() {
            parts.push(count(self.copies.len(), "copy"));
        }
        if !self.checkpoints.is_empty() {
            parts.push(count(self.checkpoints.len(), "checkpoint"));
        }
        let mut sizes = Vec::new();
        if self.file_bytes() > 0 {
            sizes.push(format_bytes(self.file_bytes()));
        }
        if !self.checkpoints.is_empty() {
            sizes.push(match self.checkpoint_bytes {
                Some(b) => format!("about {}", format_bytes(b)),
                None => "git's share unknown".into(),
            });
        }
        format!("{} · {}", parts.join(", "), sizes.join(" + "))
    }

    /// What goes, what is lost with it, and what stays: the preview's
    /// sentences, in order.
    pub fn explain(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.is_empty() {
            out.push(
                match self.cleanup {
                    Cleanup::Unreachable => "Every checkpoint is one a session can reach, or new.",
                    Cleanup::OlderThan(_) => "No session of this project is that old.",
                    Cleanup::Copies => "No recovery copies: no log of this project was repaired.",
                }
                .into(),
            );
            return out;
        }
        let refs = self.checkpoints.len();
        match self.cleanup {
            Cleanup::Unreachable => {
                out.push(format!(
                    "Removes {} that no session can reach: left by deleted sessions, by audits \
                     already closed, or older than the {UNDO_KEPT} a session keeps for /undo.",
                    count(refs, "checkpoint")
                ));
                out.push(
                    "You lose nothing /undo, /redo or an audit can use; only a git command run \
                     by hand could still restore from them."
                        .into(),
                );
            }
            Cleanup::OlderThan(days) => {
                let one = self.sessions.len() == 1;
                let (its, it, they) = if one {
                    ("its", "it", "it")
                } else {
                    ("their", "them", "they")
                };
                let bytes = format_bytes(self.sessions.iter().map(|s| s.bytes).sum());
                let n = count(self.sessions.len(), "session");
                out.push(if refs == 0 {
                    format!(
                        "Removes {n} last used over {days} days ago ({bytes}) and {its} pages. \
                         No checkpoint belongs to {it} alone, so none goes with {it}."
                    )
                } else {
                    format!(
                        "Removes {n} last used over {days} days ago ({bytes}), {its} pages, and \
                         the {} only {they} reach.",
                        count(refs, "checkpoint")
                    )
                });
                out.push(format!(
                    "You lose {its} {}, /resume of {it}, and /undo or /redo of what {they} \
                     changed.",
                    if one { "conversation" } else { "conversations" }
                ));
                let usd = self.spend_usd();
                out.push(if usd > 0.0 {
                    format!(
                        "The project's spend total keeps the {} {they} cost.",
                        crate::spend::format_usd_fine(usd)
                    )
                } else {
                    "The project's spend total is not changed.".into()
                });
            }
            Cleanup::Copies => {
                out.push(format!(
                    "Removes {} ({}): each is a log as it was before resume cut a torn last \
                     record off it.",
                    count(self.copies.len(), "recovery copy"),
                    format_bytes(self.file_bytes())
                ));
                out.push(
                    "The repaired logs stay, and the sessions open as they do now; you lose the \
                     record that was cut."
                        .into(),
                );
            }
        }
        let mut kept: Vec<String> = Vec::new();
        if self.current.is_some() && matches!(self.cleanup, Cleanup::OlderThan(_)) {
            kept.push("this session".into());
        }
        for (hold, n) in &self.kept {
            if *n == 0 {
                continue;
            }
            kept.push(match hold {
                Hold::InUse => format!("{} a session reaches", count(*n, "checkpoint")),
                Hold::Recent => format!("{} made in the last day", count(*n, "checkpoint")),
                Hold::Working => format!("{} /changes uses", count(*n, "working checkpoint")),
                Hold::Free => format!("{} no session reaches", count(*n, "checkpoint")),
            });
        }
        if refs > 0 || !self.sessions.is_empty() {
            kept.push("your branches, tags, stashes and commits".into());
        }
        if !kept.is_empty() {
            out.push(format!("Kept: {}.", listed(&kept)));
        }
        if refs > 0 {
            out.push(match self.checkpoint_bytes {
                Some(b) => format!(
                    "Git gives back about {} when it next collects garbage (git gc); Ryter does \
                     not run it.",
                    format_bytes(b)
                ),
                None => "Git gives the space back when it next collects garbage (git gc); this \
                         git can't say how much. Ryter does not run it."
                    .into(),
            });
        }
        out
    }
}

/// What a cleanup did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Done {
    /// Sessions removed.
    pub sessions: usize,
    /// Checkpoints removed.
    pub checkpoints: usize,
    /// Recovery copies removed.
    pub copies: usize,
    /// Bytes of files removed.
    pub bytes: u64,
    /// What the preview listed that was left, and why.
    pub skipped: Vec<String>,
}

impl Done {
    /// One line for the chat.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.sessions > 0 {
            parts.push(count(self.sessions, "session"));
        }
        if self.copies > 0 {
            parts.push(count(self.copies, "recovery copy"));
        }
        if self.checkpoints > 0 {
            parts.push(count(self.checkpoints, "checkpoint"));
        }
        let mut line = if parts.is_empty() {
            "nothing removed".to_string()
        } else {
            format!("removed {}", listed(&parts))
        };
        if self.bytes > 0 {
            line.push_str(&format!(" · {} freed", format_bytes(self.bytes)));
        }
        if self.checkpoints > 0 {
            line.push_str(" · git frees the checkpoints' space at its next gc");
        }
        if !self.skipped.is_empty() {
            line.push_str(&format!(" · left: {}", self.skipped.join("; ")));
        }
        line
    }
}

/// Read what Ryter keeps for the project `cwd` belongs to.
pub fn report(home: &Path, cwd: &Path, now_ms: u64) -> Result<Report> {
    let root = crate::project::project_root(cwd);
    let mut out = Report {
        root: root.clone(),
        now_ms,
        sessions: Vec::new(),
        copies: Vec::new(),
        checkpoints: None,
        repo: crate::git::RepoState::NotRepo,
        checkpoint_bytes: None,
        all_sessions: 0,
        all_bytes: 0,
        names: BTreeMap::new(),
        unread: BTreeSet::new(),
    };
    for dir in session_dirs(home) {
        let id = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let bytes = dir_bytes(&dir) + dir_bytes(&crate::page::dir(home, &id));
        out.all_sessions += 1;
        out.all_bytes += bytes;
        let meta = fs::read_to_string(dir.join("meta.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
        let Some(meta) = meta else {
            out.unread.insert(id);
            continue;
        };
        let schema = meta.get("schema").and_then(|v| v.as_u64()).unwrap_or(1);
        if schema > u64::from(crate::schema::SCHEMA) {
            out.unread.insert(id.clone());
        }
        let mut shas = BTreeSet::new();
        object_ids(&meta, &mut shas);
        out.names.insert(id.clone(), shas);
        let cwd = meta
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .map(|p| p.canonicalize().unwrap_or(p));
        if !cwd.is_some_and(|c| c.starts_with(&root)) {
            continue;
        }
        let title = meta
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .unwrap_or("untitled")
            .to_string();
        out.sessions.push(SessionUse {
            id,
            title,
            bytes,
            updated_ms: meta
                .get("updated_at")
                .and_then(|v| v.as_str())
                .and_then(|t| t.trim().parse().ok()),
            spend_usd: meta
                .get("spend_usd_total")
                .and_then(serde_json::Value::as_f64)
                .filter(|v| v.is_finite() && *v > 0.0)
                .unwrap_or(0.0),
            dir: dir.clone(),
        });
        out.copies.extend(copies_in(&dir));
    }
    out.sessions
        .sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms).then(a.id.cmp(&b.id)));
    out.repo = crate::git::repo_state(&root);
    if out.repo == crate::git::RepoState::Repo {
        let mut refs = list_refs(&root)?;
        let none = BTreeSet::new();
        for c in &mut refs {
            c.hold = out.hold_without(c, &none);
        }
        let all: Vec<&Checkpoint> = refs.iter().collect();
        out.checkpoint_bytes = own_bytes(&root, &all);
        out.checkpoints = Some(refs);
    }
    Ok(out)
}

/// What `cleanup` would remove, leaving `current` (the open session) alone.
pub fn plan(report: &Report, cleanup: Cleanup, current: Option<&str>) -> Plan {
    let mut gone: BTreeSet<String> = BTreeSet::new();
    let mut sessions = Vec::new();
    let mut copies = Vec::new();
    match cleanup {
        Cleanup::OlderThan(days) => {
            let cut = report
                .now_ms
                .saturating_sub(u64::from(days.max(1)) * DAY_MS);
            for s in &report.sessions {
                if Some(s.id.as_str()) == current {
                    continue;
                }
                if s.updated_ms.is_some_and(|t| t < cut) {
                    gone.insert(s.id.clone());
                    sessions.push(s.clone());
                }
            }
        }
        Cleanup::Copies => copies = report.copies.clone(),
        Cleanup::Unreachable => {}
    }
    let mut checkpoints = Vec::new();
    let mut kept: BTreeMap<u8, usize> = BTreeMap::new();
    for c in report.refs() {
        let hold = report.hold_without(c, &gone);
        let takes = match cleanup {
            Cleanup::Unreachable => hold == Hold::Free,
            // Only what these sessions leave behind: one named for them,
            // or one only they name.
            Cleanup::OlderThan(_) => {
                hold == Hold::Free
                    && (c.hold == Hold::InUse || c.owner.as_ref().is_some_and(|o| gone.contains(o)))
            }
            Cleanup::Copies => false,
        };
        if takes {
            checkpoints.push(c.clone());
        } else if !matches!(cleanup, Cleanup::Copies) {
            *kept.entry(hold_rank(hold)).or_insert(0) += 1;
        }
    }
    let checkpoint_bytes = if checkpoints.is_empty() {
        Some(0)
    } else {
        own_bytes(&report.root, &checkpoints.iter().collect::<Vec<_>>())
    };
    Plan {
        cleanup,
        root: report.root.clone(),
        sessions,
        checkpoints,
        copies,
        checkpoint_bytes,
        kept: kept
            .into_iter()
            .map(|(rank, n)| (HOLDS[usize::from(rank)], n))
            .collect(),
        // Said as kept only where it is one of the project's.
        current: current
            .filter(|c| report.sessions.iter().any(|s| s.id == *c))
            .map(str::to_string),
    }
}

const HOLDS: [Hold; 4] = [Hold::InUse, Hold::Recent, Hold::Working, Hold::Free];

fn hold_rank(h: Hold) -> u8 {
    HOLDS.iter().position(|x| *x == h).unwrap_or(0) as u8
}

/// Remove what `plan` lists, after reading everything again: a session
/// used since the preview, a checkpoint a session has come to name or that
/// moved, and a copy that changed are left and said.
pub fn apply(home: &Path, plan: &Plan, now_ms: u64) -> Result<Done> {
    let mut done = Done::default();
    if !plan.sessions.is_empty() {
        // The project's total reads each log as it grows. Bring it up to
        // date first, so what these sessions spent is in it before their
        // logs go: the preview promised the total keeps it.
        crate::project::project_spend(home, &plan.root)?;
    }
    let fresh = report(home, &plan.root, now_ms)?;
    for s in &plan.sessions {
        let now = fresh.sessions.iter().find(|f| f.id == s.id);
        match now {
            None => {}
            Some(_) if plan.current.as_deref() == Some(s.id.as_str()) => {}
            Some(f) if f.updated_ms != s.updated_ms => {
                done.skipped
                    .push(format!("session {} (used since the preview)", s.short()));
            }
            Some(f) => match crate::session::Session::remove(&f.dir) {
                Ok(()) => {
                    done.sessions += 1;
                    done.bytes += f.bytes;
                }
                Err(e) => done.skipped.push(format!("session {} ({e})", s.short())),
            },
        }
    }
    let sessions_dir = home.join("sessions");
    for c in &plan.copies {
        let same = fs::metadata(&c.path).is_ok_and(|m| m.is_file() && m.len() == c.bytes);
        if !c.path.starts_with(&sessions_dir) || !is_copy_name(&c.path) {
            continue;
        }
        if !same {
            if c.path.exists() {
                done.skipped
                    .push(format!("{} (changed since the preview)", c.path.display()));
            }
            continue;
        }
        match fs::remove_file(&c.path) {
            Ok(()) => {
                done.copies += 1;
                done.bytes += c.bytes;
            }
            Err(e) => done.skipped.push(format!("{} ({e})", c.path.display())),
        }
    }
    if !plan.checkpoints.is_empty() {
        // Read again with the sessions gone: what is free now is what goes.
        // Sessions may already be removed, so a failed read is said in
        // what was done, not returned in place of it.
        let after = match report(home, &plan.root, now_ms) {
            Ok(after) => after,
            Err(e) => {
                done.skipped.push(format!(
                    "{} (the repository could not be read again: {e})",
                    count(plan.checkpoints.len(), "checkpoint")
                ));
                crate::trace::log(home, &format!("storage cleanup: {}", done.summary()));
                return Ok(done);
            }
        };
        let free: BTreeMap<&str, &str> = after
            .refs()
            .iter()
            .filter(|c| c.hold == Hold::Free)
            .map(|c| (c.name.as_str(), c.sha.as_str()))
            .collect();
        let mut go = Vec::new();
        let mut held = 0;
        for c in &plan.checkpoints {
            match free.get(c.name.as_str()) {
                Some(sha) if *sha == c.sha && c.name.starts_with(PREFIX) => go.push(c),
                Some(_) => held += 1,
                None if after.refs().iter().any(|a| a.name == c.name) => held += 1,
                None => {}
            }
        }
        if held > 0 {
            done.skipped.push(format!(
                "{} (a session came to use them, or they moved)",
                count(held, "checkpoint")
            ));
        }
        done.checkpoints += delete_refs(&plan.root, &go)?;
    }
    crate::trace::log(home, &format!("storage cleanup: {}", done.summary()));
    Ok(done)
}

/// `n thing(s)`.
fn count(n: usize, thing: &str) -> String {
    if n == 1 {
        format!("1 {thing}")
    } else if let Some(stem) = thing.strip_suffix('y') {
        format!("{n} {stem}ies")
    } else {
        format!("{n} {thing}s")
    }
}

/// `a`, `a and b`, `a, b and c`.
fn listed(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// `today`, `yesterday`, `12 days ago`.
pub fn ago(now_ms: u64, then_ms: u64) -> String {
    match now_ms.saturating_sub(then_ms) / DAY_MS {
        0 => "today".into(),
        1 => "yesterday".into(),
        d => format!("{d} days ago"),
    }
}

/// `512 B`, `3.4 KB`, `1.2 MB`, `2.0 GB`, in steps of 1024.
pub fn format_bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut v = n as f64 / 1024.0;
    let mut unit = 0;
    while v >= 1024.0 && unit + 1 < UNITS.len() {
        v /= 1024.0;
        unit += 1;
    }
    format!("{v:.1} {}", UNITS[unit])
}

/// Every session folder in the home: `sessions/<project>/<id>/` holding a
/// `meta.json`.
fn session_dirs(home: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(projects) = fs::read_dir(home.join("sessions")) else {
        return out;
    };
    for project in projects.flatten() {
        if !project.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Ok(sessions) = fs::read_dir(project.path()) else {
            continue;
        };
        for s in sessions.flatten() {
            let dir = s.path();
            if s.file_type().is_ok_and(|t| t.is_dir()) && dir.join("meta.json").is_file() {
                out.push(dir);
            }
        }
    }
    out.sort();
    out
}

/// Bytes in the files under `dir`; links are not followed.
fn dir_bytes(dir: &Path) -> u64 {
    let Ok(rd) = fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0;
    for ent in rd.flatten() {
        let Ok(t) = ent.file_type() else { continue };
        if t.is_dir() {
            total += dir_bytes(&ent.path());
        } else if t.is_file() {
            total += ent.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    total
}

/// `transcript.jsonl.recovery-<id>.bak` and its kind.
fn is_copy_name(path: &Path) -> bool {
    path.file_name()
        .map(|n| n.to_string_lossy())
        .is_some_and(|n| n.contains(".jsonl.recovery-") && n.ends_with(".bak"))
}

fn copies_in(dir: &Path) -> Vec<Copy> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<Copy> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()) && is_copy_name(&e.path()))
        .map(|e| Copy {
            bytes: e.metadata().map(|m| m.len()).unwrap_or(0),
            path: e.path(),
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Every git object id a session's index names, whatever field it is in:
/// a field a later Ryter adds is still read.
fn object_ids(v: &serde_json::Value, out: &mut BTreeSet<String>) {
    match v {
        serde_json::Value::String(s) if is_object_id(s) => {
            out.insert(s.clone());
        }
        serde_json::Value::Array(items) => items.iter().for_each(|i| object_ids(i, out)),
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                // `turn_records` is keyed by the checkpoint.
                if is_object_id(k) {
                    out.insert(k.clone());
                }
                object_ids(v, out);
            }
        }
        _ => {}
    }
}

fn is_object_id(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The session a checkpoint's name carries: `<id>-3`, `<id>-after-…`,
/// `audit-<id>-2`. An audit's after and undone checkpoints carry none.
fn owner_of(short: &str) -> Option<String> {
    let rest = short.strip_prefix("audit-").unwrap_or(short);
    let id = rest.get(..36)?;
    let shape = id.char_indices().all(|(i, c)| match i {
        8 | 13 | 18 | 23 => c == '-',
        _ => c.is_ascii_hexdigit(),
    });
    shape.then(|| id.to_string())
}

fn list_refs(root: &Path) -> Result<Vec<Checkpoint>> {
    let out = crate::git::git(
        root,
        &[
            "for-each-ref",
            "--format=%(refname)%00%(objectname)%00%(committerdate:unix)%00%(symref)",
            PREFIX,
        ],
    )?;
    let mut refs: Vec<Checkpoint> = out
        .lines()
        .filter_map(|line| {
            let mut f = line.split('\0');
            let name = f.next()?.to_string();
            let sha = f.next()?.to_string();
            let made_ms = f
                .next()
                .and_then(|t| t.trim().parse::<u64>().ok())
                .map(|s| s * 1000);
            // Ryter writes direct refs only. A symbolic one under the prefix
            // is someone else's, and its old value is the branch it points
            // at: deleting through it would delete that branch.
            if f.next().is_some_and(|target| !target.is_empty()) {
                return None;
            }
            let short = name.strip_prefix(PREFIX)?;
            Some(Checkpoint {
                owner: owner_of(short),
                name,
                sha,
                made_ms,
                hold: Hold::InUse,
            })
        })
        .collect();
    refs.sort_by(|a, b| a.made_ms.cmp(&b.made_ms).then(a.name.cmp(&b.name)));
    Ok(refs)
}

/// Bytes on disk of the objects only `refs` reach: not every other ref,
/// nor HEAD. `None` when this git has no `--disk-usage` or can't say.
fn own_bytes(root: &Path, refs: &[&Checkpoint]) -> Option<u64> {
    if refs.is_empty() {
        return Some(0);
    }
    let these: BTreeSet<&str> = refs.iter().map(|c| c.name.as_str()).collect();
    let others = crate::git::git(
        root,
        &["for-each-ref", "--format=%(refname)%00%(objectname)"],
    )
    .ok()?;
    let mut input = String::new();
    for c in refs {
        input.push_str(&c.sha);
        input.push('\n');
    }
    for line in others.lines() {
        if let Some((name, sha)) = line.split_once('\0') {
            if !these.contains(name) {
                input.push('^');
                input.push_str(sha);
                input.push('\n');
            }
        }
    }
    if let Ok(head) = crate::git::head(root) {
        input.push('^');
        input.push_str(head.trim());
        input.push('\n');
    }
    let out = git_stdin(
        root,
        &["rev-list", "--objects", "--disk-usage", "--stdin"],
        &input,
    )
    .ok()?;
    out.trim().parse().ok()
}

/// Delete `refs` in one transaction, each only if it still points where it
/// did. If the transaction fails (one moved), each is tried alone.
fn delete_refs(root: &Path, refs: &[&Checkpoint]) -> Result<usize> {
    if refs.is_empty() {
        return Ok(0);
    }
    // `no-deref` holds for the one command after it: each delete removes
    // the ref named and never what it might point at.
    let mut input = String::new();
    for c in refs {
        if !c.name.starts_with(PREFIX) {
            continue;
        }
        input.push_str(&format!("option no-deref\ndelete {} {}\n", c.name, c.sha));
    }
    if input.is_empty() {
        return Ok(0);
    }
    if git_stdin(root, &["update-ref", "--stdin"], &input).is_ok() {
        return Ok(input.matches("delete ").count());
    }
    let mut n = 0;
    for c in refs.iter().filter(|c| c.name.starts_with(PREFIX)) {
        if crate::git::git(root, &["update-ref", "--no-deref", "-d", &c.name, &c.sha]).is_ok() {
            n += 1;
        }
    }
    Ok(n)
}

fn git_stdin(dir: &Path, args: &[&str], input: &str) -> Result<String> {
    let mut child = Command::new("git")
        .args(["-C", &dir.to_string_lossy()])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::Io(format!("git: {e}")))?;
    let mut stdin = child.stdin.take().expect("piped");
    let body = input.to_string();
    // Written from a thread: a long list must not wait on a full pipe while
    // git waits on us to read its output.
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(body.as_bytes());
    });
    let out = child
        .wait_with_output()
        .map_err(|e| Error::Io(format!("git: {e}")))?;
    let _ = writer.join();
    if !out.status.success() {
        return Err(Error::Io(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests;
