//! Project cost: every session's spend for one project, across sessions.
//!
//! A project is its git repository root, so sessions started in a subfolder
//! count toward it; outside a repository it is the folder. Nothing new is
//! recorded: every model call is already appended to its session's
//! `spend.jsonl` as it is charged. This adds those logs up.
//!
//! Adding up every log at every launch would slow down as sessions pile up,
//! so a running total is kept in `~/.ryter/projects/<project>.json` with the
//! byte offset read in each log. Each call reads only what was appended since.
//! The logs stay the truth: a missing, corrupt, or outdated total (a log that
//! shrank) is rebuilt from them.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::role::Role;
use crate::session::SpendRecord;

/// What one project has cost, across sessions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectSpend {
    /// The project root this total belongs to.
    pub root: PathBuf,
    /// Known USD.
    pub total_usd: f64,
    /// Model calls counted.
    pub calls: u64,
    /// Calls with no known price: the total leaves them out, and says so.
    pub unpriced_calls: u64,
    /// Sessions with any spend.
    pub sessions: u64,
    /// USD by role (`build`, `plan`, `review`, `orchestrator`, `architect`, …).
    pub by_role: BTreeMap<String, f64>,
    /// USD by model.
    pub by_model: BTreeMap<String, f64>,
    /// USD by UTC month, `YYYY-MM`.
    pub by_month: BTreeMap<String, f64>,
    /// Bytes read so far in each session's log.
    #[serde(default)]
    offsets: BTreeMap<String, u64>,
}

impl ProjectSpend {
    /// Solo mode's share (the build, plan, and review hats).
    pub fn solo_usd(&self) -> f64 {
        self.by_role
            .iter()
            .filter(|(r, _)| matches!(r.as_str(), "build" | "plan" | "review"))
            .map(|(_, v)| v)
            .sum()
    }

    /// Crew mode's share: the lead and every specialist.
    pub fn crew_usd(&self) -> f64 {
        self.total_usd - self.solo_usd()
    }

    /// This UTC month so far.
    pub fn this_month(&self) -> f64 {
        self.by_month.get(&month_of_now()).copied().unwrap_or(0.0)
    }

    /// Count one call: the live path, for a spend that happens after the
    /// total was read. The next [`project_spend`] reads it from the log and
    /// replaces this copy, so it is never counted twice on disk.
    pub fn add(&mut self, role: Role, model: &str, usd: Option<f64>) {
        self.calls += 1;
        match usd {
            Some(v) => {
                self.total_usd += v;
                *self.by_role.entry(role.to_string()).or_insert(0.0) += v;
                *self.by_model.entry(model.to_string()).or_insert(0.0) += v;
                *self.by_month.entry(month_of_now()).or_insert(0.0) += v;
            }
            None => self.unpriced_calls += 1,
        }
    }

    fn count(&mut self, r: &SpendRecord) {
        self.calls += 1;
        let Some(v) = r.total_usd else {
            self.unpriced_calls += 1;
            return;
        };
        self.total_usd += v;
        *self.by_role.entry(r.role.to_string()).or_insert(0.0) += v;
        *self.by_model.entry(r.model.clone()).or_insert(0.0) += v;
        let month =
            r.ts.parse::<u64>()
                .map(month_of_millis)
                .unwrap_or_else(|_| month_of_now());
        *self.by_month.entry(month).or_insert(0.0) += v;
    }
}

/// The project a folder belongs to: its git repository's root, or the folder
/// itself outside a repository.
pub fn project_root(cwd: &Path) -> PathBuf {
    let canon = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    crate::git::git(&canon, &["rev-parse", "--show-toplevel"])
        .ok()
        .map(|s| PathBuf::from(s.trim()))
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.canonicalize().unwrap_or(p))
        .unwrap_or(canon)
}

/// What the project containing `cwd` has cost, across every session run in
/// it (or in any folder under it). Updates the running total on disk.
pub fn project_spend(home: &Path, cwd: &Path) -> Result<ProjectSpend> {
    let root = project_root(cwd);
    let cache = home
        .join("projects")
        .join(format!("{}.json", crate::session::cwd_slug(&root)));
    let mut total: ProjectSpend = fs::read_to_string(&cache)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .filter(|t: &ProjectSpend| t.root == root)
        .unwrap_or_else(|| ProjectSpend {
            root: root.clone(),
            ..ProjectSpend::default()
        });
    let logs = session_logs(home, &root);
    // A log shorter than what was read means it was rewritten: start over.
    let shrank = logs.iter().any(|p| {
        let key = p.to_string_lossy().to_string();
        let len = fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        total.offsets.get(&key).is_some_and(|&off| off > len)
    });
    if shrank {
        total = ProjectSpend {
            root: root.clone(),
            ..ProjectSpend::default()
        };
    }
    for log in &logs {
        read_new(&mut total, log)?;
    }
    total.sessions = logs
        .iter()
        .filter(|p| fs::metadata(p).is_ok_and(|m| m.len() > 0))
        .count() as u64;
    if let Some(dir) = cache.parent() {
        fs::create_dir_all(dir).map_err(|e| Error::Io(e.to_string()))?;
    }
    let body = serde_json::to_string(&total).map_err(|e| Error::Io(e.to_string()))?;
    fs::write(&cache, body).map_err(|e| Error::Io(e.to_string()))?;
    Ok(total)
}

/// Every `spend.jsonl` of a session whose folder is `root` or inside it.
fn session_logs(home: &Path, root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(projects) = fs::read_dir(home.join("sessions")) else {
        return out;
    };
    for dir in projects.flatten().map(|e| e.path()) {
        let Ok(sessions) = fs::read_dir(&dir) else {
            continue;
        };
        let sessions: Vec<PathBuf> = sessions.flatten().map(|e| e.path()).collect();
        // Every session in one folder shares its cwd; one meta.json says it.
        let cwd = sessions.iter().find_map(|s| {
            let text = fs::read_to_string(s.join("meta.json")).ok()?;
            let v: serde_json::Value = serde_json::from_str(&text).ok()?;
            v.get("cwd")?.as_str().map(PathBuf::from)
        });
        let Some(cwd) = cwd else {
            continue;
        };
        let cwd = cwd.canonicalize().unwrap_or(cwd);
        if !cwd.starts_with(root) {
            continue;
        }
        for s in sessions {
            let log = s.join("spend.jsonl");
            if log.is_file() {
                out.push(log);
            }
        }
    }
    out.sort();
    out
}

/// Count the lines appended to `log` since the last read. A line still being
/// written (no newline yet) waits for the next read.
fn read_new(total: &mut ProjectSpend, log: &Path) -> Result<()> {
    let key = log.to_string_lossy().to_string();
    let start = total.offsets.get(&key).copied().unwrap_or(0);
    let mut f = fs::File::open(log).map_err(|e| Error::Io(e.to_string()))?;
    f.seek(SeekFrom::Start(start))
        .map_err(|e| Error::Io(e.to_string()))?;
    let mut buf = String::new();
    f.read_to_string(&mut buf)
        .map_err(|e| Error::Io(e.to_string()))?;
    let complete = buf.rfind('\n').map(|i| i + 1).unwrap_or(0);
    for line in buf[..complete].lines() {
        if let Ok(r) = serde_json::from_str::<SpendRecord>(line) {
            total.count(&r);
        }
    }
    total.offsets.insert(key, start + complete as u64);
    Ok(())
}

fn month_of_millis(ms: u64) -> String {
    let date = crate::prompt::civil_date(ms / 86_400_000);
    date[..7].to_string()
}

fn month_of_now() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    month_of_millis(ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase::Phase;
    use crate::session::Session;
    use tempfile::TempDir;

    fn spend(session: &mut Session, role: Role, model: &str, usd: Option<f64>) {
        session
            .record_spend(crate::session::spend_record(
                "c".into(),
                model.into(),
                role,
                crate::spend::Usage::default(),
                usd,
            ))
            .unwrap();
    }

    fn session(home: &Path, cwd: &Path) -> Session {
        Session::create(home, cwd, Phase::Build, "c".into(), "m".into()).unwrap()
    }

    /// Sessions anywhere in the repository count; other projects don't.
    #[test]
    fn a_project_is_its_repository() {
        let home = TempDir::new().unwrap();
        let repo = TempDir::new().unwrap();
        let other = TempDir::new().unwrap();
        crate::git::init_repo(repo.path()).unwrap();
        std::fs::create_dir_all(repo.path().join("web/src")).unwrap();
        let mut a = session(home.path(), repo.path());
        spend(&mut a, Role::SoloBuild, "cheap", Some(0.25));
        let mut b = session(home.path(), &repo.path().join("web/src"));
        spend(&mut b, Role::Architect, "strong", Some(1.0));
        spend(&mut b, Role::Builder, "cheap", None);
        let mut c = session(home.path(), other.path());
        spend(&mut c, Role::SoloBuild, "cheap", Some(9.0));

        // From the subfolder, the whole repository.
        let t = project_spend(home.path(), &repo.path().join("web")).unwrap();
        assert!((t.total_usd - 1.25).abs() < 1e-9, "{t:?}");
        assert_eq!(t.calls, 3);
        assert_eq!(t.unpriced_calls, 1, "unknown is counted, not zero");
        assert_eq!(t.sessions, 2);
        assert!((t.solo_usd() - 0.25).abs() < 1e-9);
        assert!((t.crew_usd() - 1.0).abs() < 1e-9);
        assert!((t.by_model["cheap"] - 0.25).abs() < 1e-9);
        assert!((t.this_month() - 1.25).abs() < 1e-9);
    }

    /// The running total reads only what's new, never counts twice, and
    /// rebuilds when it can't be trusted.
    #[test]
    fn the_running_total_is_incremental_and_self_healing() {
        let home = TempDir::new().unwrap();
        let dir = TempDir::new().unwrap();
        let mut s = session(home.path(), dir.path());
        spend(&mut s, Role::SoloBuild, "m", Some(0.5));
        assert!((project_spend(home.path(), dir.path()).unwrap().total_usd - 0.5).abs() < 1e-9);
        // Reading again adds nothing.
        assert!((project_spend(home.path(), dir.path()).unwrap().total_usd - 0.5).abs() < 1e-9);
        spend(&mut s, Role::SoloPlan, "m", Some(0.25));
        assert!((project_spend(home.path(), dir.path()).unwrap().total_usd - 0.75).abs() < 1e-9);
        // A corrupt total is rebuilt from the logs.
        let cache = home.path().join("projects").join(format!(
            "{}.json",
            crate::session::cwd_slug(&project_root(dir.path()))
        ));
        std::fs::write(&cache, "not json").unwrap();
        assert!((project_spend(home.path(), dir.path()).unwrap().total_usd - 0.75).abs() < 1e-9);
        // A log rewritten shorter starts the count over.
        std::fs::write(s.spend_path(), "").unwrap();
        spend(&mut s, Role::SoloBuild, "m", Some(0.1));
        assert!((project_spend(home.path(), dir.path()).unwrap().total_usd - 0.1).abs() < 1e-9);
    }

    #[test]
    fn months_come_from_the_call_time() {
        assert_eq!(month_of_millis(20_717 * 86_400_000), "2026-09");
        assert_eq!(month_of_millis(0), "1970-01");
    }
}
