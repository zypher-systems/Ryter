//! What Ryter has seen in a project, across sessions on this machine: the
//! commands that were run here, how they came out and how long they took,
//! and what the last sessions did. Written by Ryter from what happened,
//! never by the model, and put in front of the model when a session's first
//! turn begins.
//!
//! A model rediscovers these every session: that the tests take forty
//! seconds (and so sets a timeout that cuts them off), that `npm test` is
//! not found here because the project uses pnpm, that the last session left
//! the check failing. Each rediscovery is a round or a wrong turn.
//!
//! Kept in Ryter's home (`projects/<slug>.observed.json`), not in the
//! project: durations and missing tools are facts about this machine.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Commands kept, newest first.
const MAX_COMMANDS: usize = 12;
/// Sessions kept, newest first.
const MAX_SESSIONS: usize = 5;
/// Durations kept per command, newest last.
const MAX_DURATIONS: usize = 10;
/// Files named per session.
const MAX_EDITED: usize = 8;
/// A command this slow is noted whatever it is: its duration is the fact.
const SLOW: Duration = Duration::from_secs(2);
/// A command that starts with one of these is a project verb: noted however
/// fast, since which runner the project uses is the fact.
const TOOLS: &[&str] = &[
    "cargo", "rustc", "npm", "pnpm", "yarn", "bun", "deno", "npx", "node", "tsc", "jest", "vitest",
    "mocha", "eslint", "prettier", "python", "python3", "pytest", "pip", "pip3", "uv", "poetry",
    "ruff", "mypy", "pyright", "go", "make", "cmake", "ninja", "mvn", "gradle", "dotnet", "mix",
    "bundle", "rake", "ruby", "php", "composer", "swift", "zig", "docker", "podman", "just",
];

/// How a command came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    Failed,
    /// The program was not on this machine.
    NotFound,
    TimedOut,
}

impl Outcome {
    /// From a run: whether it passed, whether it timed out, and what it
    /// printed, read for the shell's way of saying the program is missing.
    pub fn of(ok: bool, timed_out: bool, output: &str) -> Self {
        if timed_out {
            return Self::TimedOut;
        }
        if ok {
            return Self::Ok;
        }
        let head: String = output.lines().take(3).collect::<Vec<_>>().join("\n");
        let low = head.to_ascii_lowercase();
        if low.contains("command not found")
            || low.contains("not recognized as an internal or external command")
            || (low.contains("no such file or directory") && low.contains("sh:"))
        {
            Self::NotFound
        } else {
            Self::Failed
        }
    }
}

/// One command, over its runs here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandNote {
    pub runs: u32,
    pub ok: u32,
    pub failed: u32,
    pub not_found: u32,
    pub timed_out: u32,
    /// The newest runs' durations, in milliseconds.
    pub durations_ms: Vec<u64>,
    pub last: Option<Outcome>,
    /// The day of the last run, `YYYY-MM-DD`.
    pub last_at: String,
}

/// One session, as it stood when it last ended a turn.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionNote {
    pub id: String,
    /// `YYYY-MM-DD HH:MM`, when the session's first turn ran.
    pub at: String,
    pub title: String,
    pub turns: u32,
    /// Files the model wrote or edited, in the order first touched.
    pub edited: Vec<String>,
    /// How many more than are named.
    #[serde(default)]
    pub edited_more: usize,
    /// The last check or test run's result, if one ran.
    pub check: Option<bool>,
}

/// Everything seen in one project.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observed {
    pub root: PathBuf,
    /// By the command as the model wrote it, whitespace collapsed.
    pub commands: BTreeMap<String, CommandNote>,
    /// Newest last.
    pub sessions: Vec<SessionNote>,
}

fn path(home: &Path, root: &Path) -> PathBuf {
    home.join("projects")
        .join(format!("{}.observed.json", crate::session::cwd_slug(root)))
}

/// What has been seen in the project at `root`; nothing, for a project
/// never seen.
pub fn load(home: &Path, root: &Path) -> Observed {
    let p = path(home, root);
    let root = root.to_path_buf();
    crate::outside::run(move || {
        std::fs::read_to_string(&p)
            .ok()
            .and_then(|t| serde_json::from_str::<Observed>(&t).ok())
            .filter(|o| o.root == root)
            .unwrap_or(Observed {
                root,
                ..Observed::default()
            })
    })
}

fn save(home: &Path, root: &Path, observed: &Observed) {
    let p = path(home, root);
    let Ok(text) = serde_json::to_string_pretty(observed) else {
        return;
    };
    // Best effort: a note that cannot be written is a note not kept, and
    // the turn goes on.
    crate::outside::run(move || {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&p, text);
    });
}

/// The command as a note names it: one line, whitespace collapsed, cut.
fn normalize(cmd: &str) -> String {
    let one: String = cmd.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= 120 {
        one
    } else {
        let mut cut: String = one.chars().take(119).collect();
        cut.push('…');
        cut
    }
}

/// Whether a command's run is a fact worth keeping: a project verb, or a
/// slow command; `always` for the project's own commands from the run file.
fn worth_noting(cmd: &str, took: Duration, always: bool) -> bool {
    if always || took >= SLOW {
        return true;
    }
    let first = cmd
        .split_whitespace()
        .find(|w| !w.contains('='))
        .unwrap_or("");
    let first = first.rsplit('/').next().unwrap_or(first);
    TOOLS.contains(&first)
}

/// Note a command's run. `always` for the run file's own commands, which are
/// the project's verbs whatever they are.
pub fn note_command(
    home: &Path,
    root: &Path,
    cmd: &str,
    outcome: Outcome,
    took: Duration,
    always: bool,
) {
    if !worth_noting(cmd, took, always) {
        return;
    }
    let mut observed = load(home, root);
    let key = normalize(cmd);
    let note = observed.commands.entry(key.clone()).or_default();
    note.runs += 1;
    match outcome {
        Outcome::Ok => note.ok += 1,
        Outcome::Failed => note.failed += 1,
        Outcome::NotFound => note.not_found += 1,
        Outcome::TimedOut => note.timed_out += 1,
    }
    if outcome != Outcome::NotFound {
        note.durations_ms
            .push(u64::try_from(took.as_millis()).unwrap_or(u64::MAX));
        if note.durations_ms.len() > MAX_DURATIONS {
            let drop = note.durations_ms.len() - MAX_DURATIONS;
            note.durations_ms.drain(..drop);
        }
    }
    note.last = Some(outcome);
    note.last_at = crate::clock::stamp().chars().take(10).collect();
    // The commands kept are the newest run: an old one-off goes.
    while observed.commands.len() > MAX_COMMANDS {
        let oldest = observed
            .commands
            .iter()
            .min_by(|a, b| a.1.last_at.cmp(&b.1.last_at).then(a.1.runs.cmp(&b.1.runs)))
            .map(|(k, _)| k.clone());
        match oldest {
            Some(k) if k != key => observed.commands.remove(&k),
            _ => break,
        };
    }
    save(home, root, &observed);
}

/// Note a session as it stands: the entry with its id is replaced.
pub fn note_session(home: &Path, root: &Path, mut note: SessionNote) {
    let mut observed = load(home, root);
    if note.edited.len() > MAX_EDITED {
        note.edited_more = note.edited.len() - MAX_EDITED;
        note.edited.truncate(MAX_EDITED);
    }
    if let Some(existing) = observed.sessions.iter_mut().find(|s| s.id == note.id) {
        note.at = existing.at.clone();
        *existing = note;
    } else {
        observed.sessions.push(note);
    }
    if observed.sessions.len() > MAX_SESSIONS {
        let drop = observed.sessions.len() - MAX_SESSIONS;
        observed.sessions.drain(..drop);
    }
    save(home, root, &observed);
}

/// Seconds, to the second, or tenths under ten.
fn secs(ms: u64) -> String {
    if ms < 1_000 {
        "under a second".to_string()
    } else if ms < 10_000 {
        format!("{:.1} s", ms as f64 / 1000.0)
    } else if ms < 600_000 {
        format!("{} s", ms / 1000)
    } else {
        format!("{} min", ms / 60_000)
    }
}

fn describe(cmd: &str, n: &CommandNote) -> String {
    let runs = format!("{} run{}", n.runs, if n.runs == 1 { "" } else { "s" });
    if n.not_found == n.runs {
        return format!(
            "- `{cmd}` · {runs} · command not found {}: not on this machine",
            if n.runs == 1 { "" } else { "every time" }
        )
        .replace("  ", " ")
        .replace(" :", ":");
    }
    let last = match n.last {
        Some(Outcome::Ok) => "passed",
        Some(Outcome::Failed) => "failed",
        Some(Outcome::NotFound) => "was not found",
        Some(Outcome::TimedOut) => "timed out",
        None => "ran",
    };
    let mut s = format!("- `{cmd}` · {runs} · last {last} {}", n.last_at);
    if n.failed + n.timed_out > 0 && n.ok > 0 {
        s.push_str(&format!(" · passed {} of {}", n.ok, n.runs));
    }
    if !n.durations_ms.is_empty() {
        let mut d = n.durations_ms.clone();
        d.sort_unstable();
        let median = d[d.len() / 2];
        if median < 1_000 && d[d.len() - 1] < 1_000 {
            s.push_str(" · under a second");
        } else if d.len() >= 3 && d[d.len() - 1] > d[0] * 2 {
            s.push_str(&format!(
                " · about {} ({}–{})",
                secs(median),
                secs(d[0]),
                secs(d[d.len() - 1])
            ));
        } else {
            s.push_str(&format!(" · about {}", secs(median)));
        }
    }
    s
}

/// The section of the prompt, or `None` when nothing has been seen.
pub fn shown(home: &Path, root: &Path, current_session: &str) -> Option<String> {
    let observed = load(home, root);
    let sessions: Vec<&SessionNote> = observed
        .sessions
        .iter()
        .rev()
        .filter(|s| s.id != current_session && s.turns > 0)
        .collect();
    if observed.commands.is_empty() && sessions.is_empty() {
        return None;
    }
    let mut s = String::from(
        "## What Ryter has seen in this project\nFrom earlier sessions on this machine, \
         written by Ryter from what happened. The durations are what the commands took \
         here: set timeouts by them, and prefer the fast check to the slow suite while \
         iterating. A command not found here is not this project's runner.\n",
    );
    if !observed.commands.is_empty() {
        s.push_str("\nCommands\n");
        let mut cmds: Vec<(&String, &CommandNote)> = observed.commands.iter().collect();
        cmds.sort_by(|a, b| b.1.last_at.cmp(&a.1.last_at).then(b.1.runs.cmp(&a.1.runs)));
        for (cmd, note) in cmds {
            s.push_str(&describe(cmd, note));
            s.push('\n');
        }
    }
    if !sessions.is_empty() {
        s.push_str("\nLast sessions\n");
        for session in sessions {
            let mut line = format!(
                "- {} · \"{}\" · {} turn{}",
                session.at,
                session.title,
                session.turns,
                if session.turns == 1 { "" } else { "s" }
            );
            if session.edited.is_empty() {
                line.push_str(" · nothing edited");
            } else {
                line.push_str(&format!(" · edited {}", session.edited.join(", ")));
                if session.edited_more > 0 {
                    line.push_str(&format!(" and {} more", session.edited_more));
                }
            }
            match session.check {
                Some(true) => line.push_str(" · the check passed at the end"),
                Some(false) => line.push_str(" · the check was failing at the end"),
                None => {}
            }
            s.push_str(&line);
            s.push('\n');
        }
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn project_verbs_and_slow_commands_are_noted_and_one_offs_are_not() {
        let home = TempDir::new().unwrap();
        let root = TempDir::new().unwrap();
        let (h, r) = (home.path(), root.path());
        note_command(h, r, "ls -la", Outcome::Ok, Duration::from_millis(5), false);
        note_command(
            h,
            r,
            "cargo  test",
            Outcome::Failed,
            Duration::from_millis(41_000),
            false,
        );
        note_command(
            h,
            r,
            "cargo test",
            Outcome::Ok,
            Duration::from_millis(28_000),
            false,
        );
        note_command(
            h,
            r,
            "cargo test",
            Outcome::Ok,
            Duration::from_millis(61_000),
            false,
        );
        note_command(
            h,
            r,
            "npm test",
            Outcome::NotFound,
            Duration::from_millis(30),
            false,
        );
        note_command(
            h,
            r,
            "npm test",
            Outcome::NotFound,
            Duration::from_millis(30),
            false,
        );
        note_command(
            h,
            r,
            "sh t.sh",
            Outcome::Ok,
            Duration::from_millis(120),
            true,
        );
        note_command(
            h,
            r,
            "./slow-thing",
            Outcome::Ok,
            Duration::from_secs(3),
            false,
        );
        let o = load(h, r);
        assert!(!o.commands.contains_key("ls -la"));
        let cargo = &o.commands["cargo test"];
        assert_eq!((cargo.runs, cargo.ok, cargo.failed), (3, 2, 1));
        assert_eq!(cargo.durations_ms, vec![41_000, 28_000, 61_000]);
        assert_eq!(o.commands["npm test"].not_found, 2);
        assert!(o.commands["npm test"].durations_ms.is_empty());
        assert!(o.commands.contains_key("sh t.sh"));
        assert!(o.commands.contains_key("./slow-thing"));
        let text = shown(h, r, "this").unwrap();
        assert!(
            text.starts_with("## What Ryter has seen in this project\n"),
            "{text}"
        );
        let day = crate::clock::stamp().chars().take(10).collect::<String>();
        assert!(
            text.contains(&format!("- `cargo test` · 3 runs · last passed {day} · passed 2 of 3 · about 41 s (28 s–61 s)")),
            "{text}"
        );
        assert!(
            text.contains(
                "- `npm test` · 2 runs · command not found every time: not on this machine"
            ),
            "{text}"
        );
        assert!(
            text.contains("- `sh t.sh` · 1 run · last passed") && text.contains("· under a second"),
            "{text}"
        );
        assert!(
            text.contains("- `./slow-thing` · 1 run · last passed"),
            "{text}"
        );
        assert!(!text.contains("Last sessions"), "{text}");
    }

    #[test]
    fn sessions_are_kept_newest_last_replaced_by_id_and_shown_without_the_current_one() {
        let home = TempDir::new().unwrap();
        let root = TempDir::new().unwrap();
        let (h, r) = (home.path(), root.path());
        assert_eq!(shown(h, r, "s1"), None);
        for i in 0..7 {
            note_session(
                h,
                r,
                SessionNote {
                    id: format!("s{i}"),
                    at: format!("2026-10-2{} 10:0{i}", i % 3),
                    title: format!("task {i}"),
                    turns: 1,
                    edited: (0..i).map(|k| format!("src/f{k}.rs")).collect(),
                    edited_more: 0,
                    check: if i % 2 == 0 { Some(true) } else { None },
                },
            );
        }
        let o = load(h, r);
        assert_eq!(o.sessions.len(), MAX_SESSIONS);
        assert_eq!(o.sessions[0].id, "s2");
        // A later note of the same session replaces it and keeps its start.
        note_session(
            h,
            r,
            SessionNote {
                id: "s6".into(),
                at: "later".into(),
                title: "task 6 renamed".into(),
                turns: 4,
                edited: (0..10).map(|k| format!("src/g{k}.rs")).collect(),
                edited_more: 0,
                check: Some(false),
            },
        );
        let o = load(h, r);
        assert_eq!(o.sessions.len(), MAX_SESSIONS);
        let last = o.sessions.last().unwrap();
        assert_eq!(
            (last.at.as_str(), last.title.as_str(), last.turns),
            ("2026-10-20 10:06", "task 6 renamed", 4)
        );
        assert_eq!((last.edited.len(), last.edited_more), (MAX_EDITED, 2));
        let text = shown(h, r, "s6").unwrap();
        assert!(!text.contains("task 6"), "{text}");
        let lines: Vec<&str> = text.lines().filter(|l| l.starts_with("- 2026")).collect();
        assert_eq!(lines.len(), 4, "{text}");
        assert!(
            lines[0].starts_with(
                "- 2026-10-22 10:05 · \"task 5\" · 1 turn · edited src/f0.rs, src/f1.rs"
            ),
            "{}",
            lines[0]
        );
        assert!(
            lines[1].ends_with("· the check passed at the end"),
            "{}",
            lines[1]
        );
        let text = shown(h, r, "none").unwrap();
        assert!(
            text.contains("· \"task 6 renamed\" · 4 turns · edited src/g0.rs")
                && text.contains("and 2 more · the check was failing at the end"),
            "{text}"
        );
    }

    #[test]
    fn an_outcome_is_read_from_the_run() {
        assert_eq!(Outcome::of(true, false, ""), Outcome::Ok);
        assert_eq!(Outcome::of(false, true, ""), Outcome::TimedOut);
        assert_eq!(
            Outcome::of(false, false, "sh: 1: pnpm: not found\n[exit 127]"),
            Outcome::Failed
        );
        assert_eq!(
            Outcome::of(
                false,
                false,
                "bash: line 1: pnpm: command not found\n[exit 127]"
            ),
            Outcome::NotFound
        );
        assert_eq!(
            Outcome::of(false, false, "sh: 1: ./run: No such file or directory"),
            Outcome::NotFound
        );
        assert_eq!(
            Outcome::of(false, false, "error: test failed"),
            Outcome::Failed
        );
    }

    #[test]
    fn another_projects_notes_are_not_this_ones() {
        let home = TempDir::new().unwrap();
        let a = TempDir::new().unwrap();
        let b = TempDir::new().unwrap();
        note_command(
            home.path(),
            a.path(),
            "cargo test",
            Outcome::Ok,
            Duration::from_secs(1),
            false,
        );
        assert!(
            load(home.path(), a.path())
                .commands
                .contains_key("cargo test")
        );
        assert!(load(home.path(), b.path()).commands.is_empty());
    }
}
