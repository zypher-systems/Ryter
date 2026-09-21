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
    /// Status.
    pub status: TaskStatus,
    /// Auditor retries used.
    #[serde(default)]
    pub retries: u32,
    /// Last auditor findings (if any).
    #[serde(default)]
    pub findings: String,
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
        let tasks = fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
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
                    status: u.status.unwrap_or(TaskStatus::Pending),
                    retries: 0,
                    findings: String::new(),
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
        // Parallel builders merge into one branch, so two tasks editing the
        // same files would race to conflict. Take pending tasks in order and
        // skip any whose scope collides with one already in this batch; they
        // run in a later batch.
        let mut out: Vec<Task> = Vec::new();
        for t in &mut self.tasks {
            if out.len() >= max as usize {
                break;
            }
            if t.status != TaskStatus::Pending || !eligible(t) {
                continue;
            }
            // An empty batch accepts anything, so an unscoped task at the head
            // of the queue still runs — alone.
            if !out.iter().all(|o| can_run_together(o, t)) {
                continue;
            }
            t.status = TaskStatus::Running;
            out.push(t.clone());
        }
        let _ = self.save();
        out
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

/// One `todo_write` item. Every field is optional so an update can name
/// only what changes.
struct Update {
    id: Option<String>,
    title: Option<String>,
    brief: Option<String>,
    files: Option<Vec<String>>,
    role: Option<String>,
    hold: Option<bool>,
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
}
