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
        let items = args
            .get("items")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::Config("todo_write: missing items array".into()))?;
        let mut next = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let Item {
                id,
                title,
                brief,
                files,
                status,
            } = parse_item(i, item);
            if let Some(old) = self.tasks.iter().find(|t| t.id == id) {
                next.push(Task {
                    id,
                    title,
                    brief,
                    files,
                    status: if status == TaskStatus::Pending && old.status == TaskStatus::Running {
                        old.status
                    } else {
                        status
                    },
                    retries: old.retries,
                    findings: old.findings.clone(),
                });
            } else {
                next.push(Task {
                    id,
                    title,
                    brief,
                    files,
                    status,
                    retries: 0,
                    findings: String::new(),
                });
            }
        }
        // Keep in-flight tasks the new list omits. An architect running as a
        // queue item rewrites the list; dropping its own entry would orphan
        // the result when it finishes.
        for t in &self.tasks {
            if t.status == TaskStatus::Running && !next.iter().any(|n| n.id == t.id) {
                next.insert(0, t.clone());
            }
        }
        self.tasks = next;
        self.save()
    }

    /// Pending items, up to `max`, marked running.
    pub fn take_pending(&mut self, max: u32) -> Vec<Task> {
        // Parallel builders merge into one branch, so two tasks editing the
        // same files would race to conflict. Take pending tasks in order and
        // skip any whose scope collides with one already in this batch; they
        // run in a later batch.
        let mut out: Vec<Task> = Vec::new();
        for t in &mut self.tasks {
            if out.len() >= max as usize {
                break;
            }
            if t.status != TaskStatus::Pending {
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

/// One `todo_write` item, parsed.
struct Item {
    id: String,
    title: String,
    brief: String,
    files: Vec<String>,
    status: TaskStatus,
}

fn parse_item(i: usize, item: &Value) -> Item {
    if let Some(s) = item.as_str() {
        return Item {
            id: format!("t{}", i + 1),
            title: s.to_string(),
            brief: String::new(),
            files: Vec::new(),
            status: TaskStatus::Pending,
        };
    }
    let title = item
        .get("content")
        .or_else(|| item.get("title"))
        .and_then(Value::as_str)
        .unwrap_or("task")
        .to_string();
    let id = item
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("t{}", i + 1));
    let brief = item
        .get("brief")
        .or_else(|| item.get("description"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let files = item
        .get("files")
        .and_then(Value::as_array)
        .map(|a| {
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
        })
        .unwrap_or_default();
    let status = match item
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("pending")
    {
        "done" | "completed" => TaskStatus::Done,
        "blocked" => TaskStatus::Blocked,
        "running" => TaskStatus::Running,
        _ => TaskStatus::Pending,
    };
    Item {
        id,
        title,
        brief,
        files,
        status,
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
