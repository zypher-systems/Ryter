//! FIFO task queue. `todo_write` is the orchestrator's feed.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

use crate::error::{Error, Result};
use crate::ids::SubagentId;

/// Lifecycle of one queue item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    /// Designed but held for the user's go-ahead (an architect task with
    /// `hold`). The lead releases it by setting it to pending.
    Proposed,
    /// Waiting for a specialist.
    Pending,
    /// A specialist is running.
    Running,
    /// Auditor rejected; retries exhausted.
    Blocked,
    /// Merged or otherwise finished.
    Done,
}

/// One unit of work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Stable id.
    pub id: String,
    /// One-line title, shown in the tasks card.
    pub title: String,
    /// What the specialist needs to do the work: intent, constraints, how to
    /// know it is done. The title alone used to be a builder's entire brief.
    #[serde(default)]
    pub brief: String,
    /// Paths (files or directories) this task owns. Declared scopes let tasks
    /// run in parallel; overlapping or undeclared ones run one at a time.
    #[serde(default)]
    pub files: Vec<String>,
    /// Who created it (`orchestrator`, `architect`).
    #[serde(default)]
    pub by: String,
    /// Who does it: `architect` (design; writes builder tasks) or `builder`.
    /// The lead routes each task; there are no phases to switch.
    #[serde(default = "default_role")]
    pub role: String,
    /// On an architect task: hold the builder tasks it writes as proposed,
    /// for the user to approve ("design it, don't build yet").
    #[serde(default)]
    pub hold: bool,
    /// Tasks that must be done (for a builder task: landed) before this one
    /// starts. A plan that said "the scaffold builds first" in prose was not
    /// enforced: once the scaffold was blocked, the tasks built on it ran
    /// without it and were rejected for its absence, three times each.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<String>,
    /// Status.
    pub status: TaskStatus,
    /// Auditor retries used.
    #[serde(default)]
    pub retries: u32,
    /// Last auditor findings (if any).
    #[serde(default)]
    pub findings: String,
    /// What the task has spent on every run so far. Its caps count all of it.
    #[serde(default, skip_serializing_if = "crate::meter::Tally::is_empty")]
    pub spent: crate::meter::Tally,
}

/// One task as the crew board shows it: everything drawn comes from here,
/// so the picture is the queue, not a description of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskView {
    /// Queue id.
    pub id: String,
    /// One line for the user.
    pub title: String,
    /// `architect` or `builder`.
    pub role: String,
    /// `proposed` / `pending` / `running` / `blocked` / `done`.
    pub status: String,
    /// Who wrote it (`architect` for a design's tasks).
    #[serde(default)]
    pub by: String,
    /// What it waits on now: its `after`, and the task laying the project's
    /// foundation when there is one.
    #[serde(default)]
    pub waits_on: Vec<String>,
    /// Why it is blocked, or what it waits on: one line.
    #[serde(default)]
    pub reason: String,
    /// Audit rejections so far.
    #[serde(default)]
    pub retries: u32,
}

/// The open patch, as the crew board shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchView {
    /// `ryter/patch-…`.
    pub branch: String,
    /// The user's branch it lands on.
    pub target: String,
    /// Task ids taken into it.
    pub tasks: Vec<String>,
    /// Task ids whose work is on it.
    pub landed: Vec<String>,
}

/// Persisted FIFO queue (`tasks.json` in the session dir).
#[derive(Debug, Clone)]
pub struct TaskQueue {
    /// On-disk path.
    pub path: PathBuf,
    /// Items, oldest first.
    pub tasks: Vec<Task>,
}

impl TaskQueue {
    /// Load or create empty.
    pub fn open(path: PathBuf) -> Self {
        let mut tasks: Vec<Task> = fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        // Nothing is running when a queue is opened: a task saved as
        // running was interrupted (a quit or a kill mid-crew). Left as it
        // was, nothing would ever take it, drop it, or land its patch.
        for t in &mut tasks {
            if t.status == TaskStatus::Running {
                t.status = TaskStatus::Pending;
            }
        }
        Self { path, tasks }
    }

    /// Replace items from a `todo_write` payload.
    pub fn apply_todo(&mut self, args: &Value) -> Result<()> {
        self.apply_todo_as(args, "")
    }

    /// Replace the list on behalf of `by`; new tasks remember their creator.
    pub fn apply_todo_as(&mut self, args: &Value, by: &str) -> Result<()> {
        let items = args
            .get("items")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::Config("todo_write: missing items array".into()))?;
        // Merge by id. The lead and the architect both write this queue, so
        // replacing it wholesale let one erase the other's tasks unless it
        // re-listed every one. Removing a task is explicit: status "dropped".
        for item in items {
            let u = parse_update(item);
            let id = u.id.clone().unwrap_or_else(|| self.next_id());
            if u.dropped {
                self.tasks
                    .retain(|t| t.id != id || t.status == TaskStatus::Running);
                continue;
            }
            match self.tasks.iter_mut().find(|t| t.id == id) {
                Some(t) => {
                    if let Some(v) = u.title {
                        t.title = v;
                    }
                    if let Some(v) = u.brief {
                        t.brief = v;
                    }
                    if let Some(v) = u.files {
                        t.files = v;
                    }
                    if let Some(v) = u.role {
                        t.role = v;
                    }
                    if let Some(v) = u.hold {
                        t.hold = v;
                    }
                    if let Some(v) = u.after {
                        t.after = v;
                    }
                    // A running task keeps running whatever the list says.
                    if let Some(v) = u.status {
                        if t.status != TaskStatus::Running {
                            t.status = v;
                        }
                    }
                }
                None => self.tasks.push(Task {
                    id,
                    title: u.title.unwrap_or_else(|| "task".into()),
                    brief: u.brief.unwrap_or_default(),
                    files: u.files.unwrap_or_default(),
                    by: by.to_string(),
                    // An architect writes build work; it never queues designers.
                    role: if by == "architect" {
                        default_role()
                    } else {
                        u.role.unwrap_or_else(default_role)
                    },
                    hold: u.hold.unwrap_or(false),
                    after: u.after.unwrap_or_default(),
                    status: u.status.unwrap_or(TaskStatus::Pending),
                    retries: 0,
                    findings: String::new(),
                    spent: Default::default(),
                }),
            }
        }
        self.save()
    }

    /// Pending items, up to `max`, marked running.
    pub fn take_pending(&mut self, max: u32) -> Vec<Task> {
        self.take_pending_where(max, |_| true)
    }

    /// Persist after an in-place edit of `tasks`.
    pub fn set_all_saved(&mut self) {
        let _ = self.save();
    }

    /// The first `t<n>` id not in use.
    fn next_id(&self) -> String {
        (1..)
            .map(|n| format!("t{n}"))
            .find(|id| !self.tasks.iter().any(|t| &t.id == id))
            .unwrap_or_default()
    }

    /// Like [`Self::take_pending`], but only tasks `eligible` accepts.
    pub fn take_pending_where(&mut self, max: u32, eligible: impl Fn(&Task) -> bool) -> Vec<Task> {
        self.take_ready(max, eligible, |_| true)
    }

    /// Pending tasks `eligible` accepts that may start now: everything in
    /// their `after` is done, and no unfinished task is laying the project's
    /// foundation (see [`Self::foundation`]). `exists` says whether a path is
    /// in the tree the tasks branch from.
    pub fn take_ready(
        &mut self,
        max: u32,
        eligible: impl Fn(&Task) -> bool,
        exists: impl Fn(&str) -> bool,
    ) -> Vec<Task> {
        let foundation = self.foundation(&exists).map(|t| t.id.clone());
        let ready: Vec<bool> = self
            .tasks
            .iter()
            .map(|t| {
                self.unmet(t).is_empty()
                    && (t.role != "builder" || foundation.as_ref().is_none_or(|f| *f == t.id))
            })
            .collect();
        // Parallel builders merge into one branch, so two tasks editing the
        // same files would race to conflict. Take pending tasks in order and
        // skip any whose scope collides with one already in this batch; they
        // run in a later batch.
        let mut out: Vec<Task> = Vec::new();
        for (t, ready) in self.tasks.iter_mut().zip(ready) {
            if out.len() >= max as usize {
                break;
            }
            if t.status != TaskStatus::Pending || !eligible(t) || !ready {
                continue;
            }
            // The foundation runs alone.
            if foundation.as_deref() == Some(t.id.as_str()) && !out.is_empty() {
                continue;
            }
            // An empty batch accepts anything, so an unscoped task at the head
            // of the queue still runs — alone.
            if !out.iter().all(|o| can_run_together(o, t)) {
                continue;
            }
            t.status = TaskStatus::Running;
            out.push(t.clone());
            if foundation.as_deref() == Some(t.id.as_str()) {
                break;
            }
        }
        let _ = self.save();
        out
    }

    /// Whether any pending task could start now. Tasks that only wait on
    /// something blocked don't count: the crew has nothing to do for them,
    /// and running it again just repeats what they wait on.
    pub fn startable(&self, exists: impl Fn(&str) -> bool) -> bool {
        let foundation = self.foundation(&exists).map(|t| t.id.clone());
        self.tasks.iter().any(|t| {
            t.status == TaskStatus::Pending
                && (t.role == "architect" || t.role == "builder")
                && self.unmet(t).is_empty()
                && (t.role != "builder" || foundation.as_ref().is_none_or(|f| *f == t.id))
        })
    }

    /// The ids in `t.after` that are not done yet. An id that names no task
    /// is unmet: a dropped prerequisite must not be silently skipped.
    pub fn unmet<'a>(&self, t: &'a Task) -> Vec<&'a str> {
        t.after
            .iter()
            .filter(|dep| {
                !self
                    .tasks
                    .iter()
                    .any(|d| &d.id == *dep && d.status == TaskStatus::Done)
            })
            .map(String::as_str)
            .collect()
    }

    /// The unfinished builder task that creates the project's build manifest
    /// (`Cargo.toml`, `package.json`, …) when the tree has none at its root.
    /// Until it lands nothing else builds: other tasks would each invent the
    /// manifest, or be rejected because the crate cannot compile. This holds
    /// even when the plan forgot to say so in `after`.
    pub fn foundation(&self, exists: impl Fn(&str) -> bool) -> Option<&Task> {
        if MANIFESTS.iter().any(|m| exists(m)) {
            return None;
        }
        self.tasks.iter().find(|t| {
            t.role == "builder"
                && t.status != TaskStatus::Done
                && t.files.iter().any(|f| MANIFESTS.contains(&f.as_str()))
        })
    }

    /// Every task for the crew board. `exists` is as for [`Self::take_ready`].
    pub fn views(&self, exists: impl Fn(&str) -> bool) -> Vec<TaskView> {
        let foundation = self.foundation(&exists).map(|t| t.id.clone());
        self.tasks
            .iter()
            .map(|t| {
                let mut waits_on: Vec<String> = t.after.clone();
                if let Some(f) = &foundation {
                    if t.role == "builder" && *f != t.id && !waits_on.contains(f) {
                        waits_on.push(f.clone());
                    }
                }
                let status = match t.status {
                    TaskStatus::Proposed => "proposed",
                    TaskStatus::Pending => "pending",
                    TaskStatus::Running => "running",
                    TaskStatus::Blocked => "blocked",
                    TaskStatus::Done => "done",
                };
                let reason = match t.status {
                    TaskStatus::Blocked => blocked_reason(&t.findings),
                    TaskStatus::Pending => {
                        let unmet = self.unmet(t);
                        if !unmet.is_empty() {
                            format!("waits on {}", unmet.join(", "))
                        } else if foundation
                            .as_ref()
                            .is_some_and(|f| *f != t.id && t.role == "builder")
                        {
                            format!("waits on {}", foundation.clone().unwrap_or_default())
                        } else {
                            String::new()
                        }
                    }
                    _ => String::new(),
                };
                TaskView {
                    id: t.id.clone(),
                    title: t.title.clone(),
                    role: t.role.clone(),
                    status: status.into(),
                    by: t.by.clone(),
                    waits_on,
                    reason,
                    retries: t.retries,
                }
            })
            .collect()
    }

    /// Why each pending task that can't start is waiting, one line each, for
    /// the lead: it is the one who can unblock them.
    pub fn waiting(&self, exists: impl Fn(&str) -> bool) -> Vec<String> {
        let foundation = self.foundation(&exists);
        let why = |id: &str| -> String {
            match self.tasks.iter().find(|d| d.id == id) {
                None => format!("`{id}`, which is not in the task list"),
                Some(d) => {
                    let state = match d.status {
                        TaskStatus::Blocked => {
                            let reason = d.findings.lines().find(|l| !l.trim().is_empty());
                            match reason {
                                Some(r) => format!("blocked: {}", r.trim()),
                                None => "blocked".into(),
                            }
                        }
                        TaskStatus::Proposed => "proposed, waiting for approval".into(),
                        TaskStatus::Running => "running".into(),
                        TaskStatus::Pending => "pending".into(),
                        TaskStatus::Done => "done".into(),
                    };
                    format!("`{id}` ({state})")
                }
            }
        };
        self.tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Pending)
            .filter_map(|t| {
                let unmet = self.unmet(t);
                if !unmet.is_empty() {
                    let on: Vec<String> = unmet.iter().map(|d| why(d)).collect();
                    return Some(format!("- `{}` waits on {}", t.id, on.join(", ")));
                }
                match foundation {
                    Some(f) if f.id != t.id => Some(format!(
                        "- `{}` waits on {}, which creates the project's build manifest",
                        t.id,
                        why(&f.id)
                    )),
                    _ => None,
                }
            })
            .collect()
    }

    /// Update one task.
    pub fn set(&mut self, id: &str, status: TaskStatus, findings: impl Into<String>) {
        if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
            t.status = status;
            t.findings = findings.into();
        }
        let _ = self.save();
    }

    /// True if anything is still waiting or running.
    pub fn has_active(&self) -> bool {
        self.tasks
            .iter()
            .any(|t| matches!(t.status, TaskStatus::Pending | TaskStatus::Running))
    }

    fn save(&self) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(|e| Error::Io(e.to_string()))?;
        }
        let body = serde_json::to_vec_pretty(&self.tasks).map_err(|e| Error::Io(e.to_string()))?;
        fs::write(&self.path, body).map_err(|e| Error::Io(e.to_string()))
    }
}

fn default_role() -> String {
    "builder".into()
}

/// One line saying why a task is blocked. An audit's findings open with the
/// seat (`[z-ai/glm-5.3-prime · review]`) and often a line of preamble; the
/// board showed that header as the reason. The first finding, or the verdict
/// when there is none, says it.
fn blocked_reason(findings: &str) -> String {
    let lines: Vec<&str> = findings
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter(|l| !(l.starts_with('[') && l.ends_with(']')))
        .collect();
    let finding = lines.iter().find(|l| {
        let low = l.to_ascii_lowercase();
        (l.starts_with("- ")
            || l.starts_with("* ")
            || l.starts_with("1.")
            || low.contains("blocking"))
            && !low.starts_with("verdict")
    });
    let rejected = findings.contains("VERDICT: FAIL") || findings.contains("VERDICT:FAIL");
    let line = finding
        .or(lines
            .iter()
            .find(|l| !l.to_ascii_uppercase().starts_with("VERDICT")))
        .map(|l| {
            l.trim_start_matches(['-', '*', ' '])
                .trim_start_matches("1.")
                .trim()
                .to_string()
        })
        .unwrap_or_default();
    let line = line.replace("**", "");
    if rejected && !line.is_empty() {
        format!("the audit failed it: {line}")
    } else {
        line
    }
}

/// Build manifests at a project's root. A tree with none of them can't be
/// built, whatever else is in it.
pub const MANIFESTS: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "setup.py",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "CMakeLists.txt",
    "Makefile",
    "Gemfile",
    "composer.json",
    "mix.exs",
    "deno.json",
];

/// One `todo_write` item. Every field is optional so an update can name
/// only what changes.
struct Update {
    id: Option<String>,
    title: Option<String>,
    brief: Option<String>,
    files: Option<Vec<String>>,
    role: Option<String>,
    hold: Option<bool>,
    after: Option<Vec<String>>,
    status: Option<TaskStatus>,
    dropped: bool,
}

fn parse_update(item: &Value) -> Update {
    if let Some(s) = item.as_str() {
        return Update {
            id: None,
            title: Some(s.to_string()),
            brief: None,
            files: None,
            role: None,
            hold: None,
            after: None,
            status: None,
            dropped: false,
        };
    }
    let text = |k: &str| item.get(k).and_then(Value::as_str).map(str::to_string);
    let status_word = text("status").unwrap_or_default().to_ascii_lowercase();
    Update {
        id: text("id"),
        title: text("title").or_else(|| text("content")),
        brief: text("brief").or_else(|| text("description")),
        files: item.get("files").and_then(Value::as_array).map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(|f| {
                    f.trim()
                        .trim_start_matches("./")
                        .trim_end_matches('/')
                        .to_string()
                })
                .filter(|f| !f.is_empty())
                .collect()
        }),
        role: text("role").map(|r| match r.to_ascii_lowercase().as_str() {
            "architect" | "design" | "designer" | "plan" | "planner" => "architect".to_string(),
            _ => default_role(),
        }),
        hold: item.get("hold").and_then(Value::as_bool),
        after: item.get("after").and_then(Value::as_array).map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty())
                .collect()
        }),
        status: match status_word.as_str() {
            "" => None,
            "done" | "completed" => Some(TaskStatus::Done),
            "blocked" => Some(TaskStatus::Blocked),
            "running" => Some(TaskStatus::Running),
            "proposed" => Some(TaskStatus::Proposed),
            _ => Some(TaskStatus::Pending),
        },
        dropped: matches!(
            status_word.as_str(),
            "dropped" | "drop" | "removed" | "cancelled"
        ),
    }
}

/// True when two path scopes can touch the same file: equal, or one contains
/// the other (`src/tui` and `src/tui/draw.rs`).
fn scopes_overlap(a: &str, b: &str) -> bool {
    let under = |x: &str, y: &str| x == y || x.starts_with(&format!("{y}/"));
    under(a, b) || under(b, a)
}

/// Whether two tasks may run at the same time. A task with no declared files
/// could touch anything, so it runs alone.
pub fn can_run_together(a: &Task, b: &Task) -> bool {
    if a.files.is_empty() || b.files.is_empty() {
        return false;
    }
    !a.files
        .iter()
        .any(|x| b.files.iter().any(|y| scopes_overlap(x, y)))
}

/// New id for a subagent run.
pub fn new_sub_id() -> SubagentId {
    SubagentId::new(uuid::Uuid::now_v7().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn todo_write_round_trip() {
        let dir = TempDir::new().unwrap();
        let mut q = TaskQueue::open(dir.path().join("tasks.json"));
        q.apply_todo(&json!({"items":["one","two"]})).unwrap();
        assert_eq!(q.tasks.len(), 2);
        let batch = q.take_pending(1);
        assert_eq!(batch.len(), 1);
        assert_eq!(q.tasks[0].status, TaskStatus::Running);
        assert_eq!(q.tasks[1].status, TaskStatus::Pending);
    }
}

#[cfg(test)]
mod scope_tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn queue(items: Value) -> (TempDir, TaskQueue) {
        let dir = TempDir::new().unwrap();
        let mut q = TaskQueue::open(dir.path().join("tasks.json"));
        q.apply_todo(&json!({ "items": items })).unwrap();
        (dir, q)
    }

    fn ids(batch: &[Task]) -> Vec<&str> {
        batch.iter().map(|t| t.id.as_str()).collect()
    }

    #[test]
    fn disjoint_scopes_run_together_overlapping_ones_wait() {
        let (_d, mut q) = queue(json!([
            {"id": "a", "title": "parser", "files": ["src/parse"]},
            {"id": "b", "title": "parser tests", "files": ["src/parse/tests.rs"]},
            {"id": "c", "title": "docs", "files": ["docs/guide.md"]},
        ]));
        assert_eq!(ids(&q.take_pending(4)), ["a", "c"]);
        assert_eq!(ids(&q.take_pending(4)), ["b"]);
    }

    /// A task that declares nothing could touch anything, so it runs alone.
    #[test]
    fn unscoped_tasks_run_alone() {
        let (_d, mut q) = queue(json!([
            "refactor everything",
            {"id": "b", "title": "docs", "files": ["docs"]},
        ]));
        assert_eq!(ids(&q.take_pending(4)), ["t1"]);
        assert_eq!(ids(&q.take_pending(4)), ["b"]);
    }

    #[test]
    fn scope_overlap_is_by_path_component() {
        assert!(scopes_overlap("src/tui", "src/tui/draw.rs"));
        assert!(scopes_overlap("src/tui/draw.rs", "src/tui"));
        assert!(!scopes_overlap("src/tui", "src/tuix/a.rs"));
        assert!(!scopes_overlap("src/a.rs", "src/b.rs"));
    }

    /// The lead updating one task must not erase the architect's others.
    #[test]
    fn writes_merge_by_id_instead_of_replacing() {
        let (_d, mut q) = queue(json!([
            {"id": "b1", "title": "a", "files": ["a"]},
            {"id": "b2", "title": "b", "files": ["b"]},
        ]));
        q.apply_todo(&json!({"items": [{"id": "b1", "status": "blocked"}]}))
            .unwrap();
        assert_eq!(q.tasks.len(), 2, "b2 survives an update to b1");
        assert_eq!(q.tasks[0].status, TaskStatus::Blocked);
        assert_eq!(
            q.tasks[0].files,
            vec!["a".to_string()],
            "unnamed fields are kept"
        );
        q.apply_todo(&json!({"items": [{"id": "b2", "status": "dropped"}]}))
            .unwrap();
        assert_eq!(ids(&q.tasks), ["b1"]);
    }

    /// Items without ids never overwrite earlier ones.
    #[test]
    fn new_items_get_fresh_ids() {
        let (_d, mut q) = queue(json!(["one", "two"]));
        q.apply_todo(&json!({"items": ["three"]})).unwrap();
        assert_eq!(ids(&q.tasks), ["t1", "t2", "t3"]);
    }

    /// Rewriting the list must not orphan a task that is running right now.
    #[test]
    fn a_rewrite_keeps_running_tasks() {
        let (_d, mut q) = queue(json!([{"id": "arch", "title": "design"}]));
        q.take_pending(1);
        q.apply_todo(&json!({"items": [{"id": "b1", "title": "build it"}]}))
            .unwrap();
        assert!(
            q.tasks
                .iter()
                .any(|t| t.id == "arch" && t.status == TaskStatus::Running)
        );
        assert!(q.tasks.iter().any(|t| t.id == "b1"));
    }

    fn done(q: &mut TaskQueue, id: &str) {
        q.set(id, TaskStatus::Done, "");
    }

    /// A task starts only once what it is `after` is done; a blocked
    /// prerequisite holds it, and the lead is told why.
    #[test]
    fn a_task_waits_for_what_it_is_after() {
        let (_d, mut q) = queue(json!([
            {"id": "scaffold", "title": "crate", "files": ["src/main.rs"]},
            {"id": "audio", "title": "audio", "files": ["src/audio"], "after": ["scaffold"]},
            {"id": "ui", "title": "ui", "files": ["src/ui"], "after": ["scaffold"]},
        ]));
        assert_eq!(ids(&q.take_pending(4)), ["scaffold"]);
        q.set(
            "scaffold",
            TaskStatus::Blocked,
            "needs alsa-lib-devel installed\nmore",
        );
        assert!(
            q.take_pending(4).is_empty(),
            "nothing builds on a blocked scaffold"
        );
        assert_eq!(
            q.waiting(|_| true),
            [
                "- `audio` waits on `scaffold` (blocked: needs alsa-lib-devel installed)",
                "- `ui` waits on `scaffold` (blocked: needs alsa-lib-devel installed)",
            ]
        );
        q.set("scaffold", TaskStatus::Pending, "");
        assert_eq!(ids(&q.take_pending(4)), ["scaffold"]);
        done(&mut q, "scaffold");
        assert_eq!(
            ids(&q.take_pending(4)),
            ["audio", "ui"],
            "then both, in parallel"
        );
    }

    /// A prerequisite that was dropped, or never existed, is not met.
    #[test]
    fn an_unknown_prerequisite_is_unmet() {
        let (_d, mut q) = queue(json!([
            {"id": "b", "title": "b", "files": ["b"], "after": ["gone"]},
        ]));
        assert!(q.take_pending(4).is_empty());
        assert_eq!(
            q.waiting(|_| true),
            ["- `b` waits on `gone`, which is not in the task list"]
        );
    }

    /// With no build manifest in the tree, the task that creates one runs
    /// alone, and the rest wait for it, even when the plan didn't say so.
    #[test]
    fn the_task_that_creates_the_manifest_runs_first_and_alone() {
        let (_d, mut q) = queue(json!([
            {"id": "lib", "title": "library", "files": ["src/library.rs"]},
            {"id": "scaffold", "title": "crate", "files": ["Cargo.toml", "src/main.rs"]},
            {"id": "audio", "title": "audio", "files": ["src/audio"]},
        ]));
        let empty = |_: &str| false;
        assert_eq!(ids(&q.take_ready(4, |_| true, empty)), ["scaffold"]);
        q.set("scaffold", TaskStatus::Blocked, "cargo build failed");
        assert!(q.take_ready(4, |_| true, empty).is_empty());
        assert_eq!(
            q.waiting(empty),
            [
                "- `lib` waits on `scaffold` (blocked: cargo build failed), which creates the project's build manifest",
                "- `audio` waits on `scaffold` (blocked: cargo build failed), which creates the project's build manifest",
            ]
        );
        // Once the tree has a manifest there is no foundation to wait for.
        q.set("scaffold", TaskStatus::Done, "");
        let built = |p: &str| p == "Cargo.toml";
        assert_eq!(ids(&q.take_ready(4, |_| true, built)), ["lib", "audio"]);
    }

    /// An existing project already has its manifest: nothing waits on a task
    /// that edits it.
    #[test]
    fn an_existing_manifest_is_no_foundation() {
        let (_d, mut q) = queue(json!([
            {"id": "deps", "title": "bump deps", "files": ["Cargo.toml"]},
            {"id": "docs", "title": "docs", "files": ["docs"]},
        ]));
        let built = |p: &str| p == "Cargo.toml";
        assert_eq!(ids(&q.take_ready(4, |_| true, built)), ["deps", "docs"]);
    }

    /// The board's view carries what each task waits on, the foundation
    /// rule included, and why a blocked task is blocked.
    #[test]
    fn views_say_what_waits_on_what() {
        let (_d, mut q) = queue(json!([
            {"id": "scaffold", "title": "crate", "files": ["Cargo.toml"]},
            {"id": "audio", "title": "audio", "files": ["src/audio.rs"], "after": ["scaffold"]},
            {"id": "ui", "title": "ui", "files": ["src/ui.rs"]},
        ]));
        q.set(
            "scaffold",
            TaskStatus::Blocked,
            "needs alsa-lib-devel\nmore",
        );
        let v = q.views(|_| false);
        assert_eq!(v[0].reason, "needs alsa-lib-devel");
        assert_eq!(v[1].waits_on, ["scaffold"]);
        assert_eq!(v[1].reason, "waits on scaffold");
        assert_eq!(v[2].waits_on, ["scaffold"], "the manifest comes first");
        // Once the manifest exists nothing waits on it by that rule.
        let v = q.views(|p| p == "Cargo.toml");
        assert!(v[2].waits_on.is_empty());
    }

    /// The audit's header is not a reason; its first finding is.
    #[test]
    fn a_blocked_reason_is_the_finding_not_the_header() {
        let findings = "[z-ai/glm-5.3-prime · review]\nThe exact failure repeated.\n\n1. **`src/app.rs` is still absent** — nothing to review.\n2. more\n\nVERDICT: FAIL";
        assert_eq!(
            blocked_reason(findings),
            "the audit failed it: `src/app.rs` is still absent — nothing to review."
        );
        assert_eq!(
            blocked_reason("needs alsa-lib-devel\nmore"),
            "needs alsa-lib-devel"
        );
    }
}
