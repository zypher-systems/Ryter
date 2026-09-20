//! Spawn specialists from the task queue: worktrees, auditor, auto-merge.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::cancel::Cancel;
use crate::error::{Error, Result};
use crate::event::AgentEvent;
use crate::git;
use crate::ids::SubagentId;
use crate::llm::{CompletionRequest, Provider, StreamDelta};
use crate::prompt::specialist_messages;
use crate::queue::{Task, TaskStatus};
use crate::role::Role;
use crate::tools::{ToolContext, gated_execute};
use futures_util::StreamExt;

/// Result of one queue item.
#[derive(Debug, Clone)]
pub struct TaskOutcome {
    /// Queue id.
    pub id: String,
    /// New status.
    pub status: TaskStatus,
    /// Auditor text / error.
    pub findings: String,
    /// One-line summary.
    pub summary: String,
}

/// Parse auditor text. First line PASS or FAIL.
pub fn parse_verdict(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.to_ascii_uppercase().starts_with("PASS"))
        .unwrap_or(false)
}

/// Run a builder in a worktree, audit, merge.
#[allow(clippy::too_many_arguments)]
pub async fn run_build_task(
    provider: Arc<dyn Provider>,
    repo: &Path,
    home: &Path,
    session_id: &str,
    task: &Task,
    model: &str,
    auditor_provider: Arc<dyn Provider>,
    auditor_model: &str,
    always_approve: bool,
    web: bool,
    auditor_enabled: bool,
    max_retries: u32,
    project_root: Option<&Path>,
    trusted: bool,
    hooks: Option<std::sync::Arc<crate::hooks::HookSet>>,
    cancel: Arc<Cancel>,
) -> Result<TaskOutcome> {
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    if !git::is_repo(repo) {
        return Ok(TaskOutcome {
            id: task.id.clone(),
            status: TaskStatus::Blocked,
            findings: "workspace is not a git repo".into(),
            summary: "blocked".into(),
        });
    }
    let slug: String = task
        .id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(32)
        .collect();
    let sid: String = session_id
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(8)
        .collect();
    let branch = format!("ryter-{sid}-{slug}");
    let wt = home.join("worktrees").join(session_id).join(&task.id);
    git::add_worktree(repo, &wt, &branch)?;
    match run_build_task_inner(
        provider,
        repo,
        home,
        session_id,
        task,
        model,
        auditor_provider,
        auditor_model,
        always_approve,
        web,
        auditor_enabled,
        max_retries,
        project_root,
        trusted,
        hooks,
        cancel,
        &wt,
        &branch,
    )
    .await
    {
        Ok(o) => Ok(o),
        Err(e) => {
            let _ = git::remove_worktree(repo, &wt, &branch);
            Err(e)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_build_task_inner(
    provider: Arc<dyn Provider>,
    repo: &Path,
    home: &Path,
    _session_id: &str,
    task: &Task,
    model: &str,
    auditor_provider: Arc<dyn Provider>,
    auditor_model: &str,
    always_approve: bool,
    web: bool,
    auditor_enabled: bool,
    max_retries: u32,
    project_root: Option<&Path>,
    trusted: bool,
    hooks: Option<std::sync::Arc<crate::hooks::HookSet>>,
    cancel: Arc<Cancel>,
    wt: &Path,
    branch: &str,
) -> Result<TaskOutcome> {
    let notes = wt.join(".ryter-notes");
    let _ = std::fs::create_dir_all(&notes);
    let ctx = ToolContext {
        workspace: wt.to_path_buf(),
        notes_dir: notes,
        role: Role::Builder,
        always_approve,
        queue: std::sync::Arc::new(std::sync::Mutex::new(crate::queue::TaskQueue::open(
            wt.join("tasks.json"),
        ))),
        mcp: None,
        hooks,
        cancel,
        user_io: None,
        sticky_approve: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        web,
    };

    let pass_note = String::new();
    let mut task_prompt = task.title.clone();
    if !task.findings.is_empty() {
        task_prompt.push_str("\n\nPrevious audit findings:\n");
        task_prompt.push_str(&task.findings);
    }
    let msgs = specialist_messages(
        home,
        project_root,
        trusted,
        Role::Builder,
        &pass_note,
        &task_prompt,
    );
    let builder_text = run_specialist(provider.as_ref(), model, Role::Builder, msgs, &ctx).await?;

    let diff = git::diff(wt).unwrap_or_default();
    // Stage everything so merge sees it.
    let _ = git::git(wt, &["add", "-A"]);
    if !git::porcelain(wt).unwrap_or_default().is_empty() {
        let _ = git::git(wt, &["commit", "-m", &format!("ryter: {}", task.title)]);
    }

    let mut findings = String::new();
    let mut pass = !auditor_enabled;
    if auditor_enabled {
        let audit_ctx = ToolContext {
            role: Role::Auditor,
            always_approve: true,
            ..ctx.clone()
        };
        let audit_msgs = specialist_messages(
            home,
            project_root,
            trusted,
            Role::Auditor,
            &diff,
            &format!(
                "Task: {}\nBuilder said: {builder_text}\nReply PASS or FAIL.",
                task.title
            ),
        );
        let audit_text = run_specialist(
            auditor_provider.as_ref(),
            auditor_model,
            Role::Auditor,
            audit_msgs,
            &audit_ctx,
        )
        .await?;
        findings = audit_text.clone();
        pass = parse_verdict(&audit_text);
    }

    if !pass {
        git::remove_worktree(repo, wt, branch)?;
        let retries = task.retries + 1;
        let status = if retries > max_retries {
            TaskStatus::Blocked
        } else {
            TaskStatus::Pending
        };
        return Ok(TaskOutcome {
            id: task.id.clone(),
            status,
            findings,
            summary: if status == TaskStatus::Blocked {
                "audit failed".into()
            } else {
                "retry".into()
            },
        });
    }

    let onto = git::branch(repo)?;
    if let Err(e) = git::merge_branch(repo, branch) {
        if git::rebase(wt, &onto).is_ok() && git::merge_branch(repo, branch).is_ok() {
            // merged after rebase
        } else {
            git::rebase_abort(wt);
            let status = git::git(wt, &["status"]).unwrap_or_default();
            let diff = git::diff(wt).unwrap_or_default();
            let prompt = format!(
                "Merge conflict merging `{branch}` into `{onto}`:\n{e}\n\nStatus:\n{status}\n\nDiff:\n{diff}\n\nResolve the conflict in this worktree and commit. Do not push."
            );
            let msgs = specialist_messages(
                home,
                project_root,
                trusted,
                Role::Builder,
                &pass_note,
                &prompt,
            );
            let _ = run_specialist(provider.as_ref(), model, Role::Builder, msgs, &ctx).await;
            let _ = git::git(wt, &["add", "-A"]);
            if !git::porcelain(wt).unwrap_or_default().is_empty() {
                let _ = git::git(
                    wt,
                    &[
                        "commit",
                        "-m",
                        &format!("ryter: resolve conflict {}", task.title),
                    ],
                );
            }
            if git::merge_branch(repo, branch).is_err() {
                git::remove_worktree(repo, wt, branch)?;
                return Ok(TaskOutcome {
                    id: task.id.clone(),
                    status: TaskStatus::Blocked,
                    findings: format!("merge conflict: {e}"),
                    summary: "blocked".into(),
                });
            }
        }
    }
    git::remove_worktree(repo, wt, branch)?;
    Ok(TaskOutcome {
        id: task.id.clone(),
        status: TaskStatus::Done,
        findings,
        summary: format!("merged {}", task.title),
    })
}

const HANDBACK_CAP: usize = 12_000;

/// Planner / architect / extra auditor: no worktree. Writes memory files via tools
/// plus a short handback in `findings`.
#[allow(clippy::too_many_arguments)]
pub async fn run_note_task(
    provider: Arc<dyn Provider>,
    workspace: &Path,
    home: &Path,
    task: &Task,
    model: &str,
    role: Role,
    always_approve: bool,
    web: bool,
    project_root: Option<&Path>,
    trusted: bool,
    hooks: Option<std::sync::Arc<crate::hooks::HookSet>>,
    notes_dir: PathBuf,
    pass_note: &str,
    cancel: Arc<Cancel>,
) -> Result<TaskOutcome> {
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let ctx = ToolContext {
        workspace: workspace.to_path_buf(),
        notes_dir,
        role,
        always_approve,
        queue: std::sync::Arc::new(std::sync::Mutex::new(crate::queue::TaskQueue::open(
            workspace.join("tasks.json"),
        ))),
        mcp: None,
        hooks,
        cancel,
        user_io: None,
        sticky_approve: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        web,
    };
    let msgs = specialist_messages(home, project_root, trusted, role, pass_note, &task.title);
    let text = run_specialist(provider.as_ref(), model, role, msgs, &ctx).await?;
    let body = clip_handback(&text);
    persist_workspace_note(workspace, role, &body);
    Ok(TaskOutcome {
        id: task.id.clone(),
        status: TaskStatus::Done,
        findings: body.clone(),
        summary: first_line(&body).unwrap_or_else(|| format!("{} {}", role, task.title)),
    })
}

fn persist_workspace_note(workspace: &Path, role: Role, body: &str) {
    let name = match role {
        Role::Planner => "plan.md",
        Role::Architect => "architect.md",
        Role::Auditor => "audit.md",
        Role::Builder => "build.md",
        Role::Orchestrator => return,
    };
    let dir = workspace.join("notes");
    let _ = std::fs::create_dir_all(&dir);
    if body.trim().is_empty() {
        return;
    }
    let path = dir.join(name);
    if let Ok(prev) = std::fs::read_to_string(&path) {
        if !prev.trim().is_empty() {
            let _ = std::fs::write(&path, format!("{}\n\n---\n\n{body}", prev.trim_end()));
            return;
        }
    }
    let _ = std::fs::write(path, body);
}

fn clip_handback(text: &str) -> String {
    let t = text.trim();
    if t.len() <= HANDBACK_CAP {
        return t.to_string();
    }
    let mut cut = HANDBACK_CAP;
    while cut > 0 && !t.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}\n\n…(handback truncated)\n", &t[..cut])
}

fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.chars().take(120).collect())
}

async fn run_specialist(
    provider: &dyn Provider,
    model: &str,
    role: Role,
    mut messages: Vec<crate::llm::Message>,
    ctx: &ToolContext,
) -> Result<String> {
    let mut last = String::new();
    for _ in 0..12 {
        if ctx.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let req = CompletionRequest {
            model: model.to_string(),
            system: None,
            messages: messages.clone(),
            tools: crate::tools::specs_for_opts(role, ctx.web),
            max_tokens: Some(4096),
        };
        let mut stream = tokio::select! {
            biased;
            () = ctx.cancel.cancelled() => return Err(Error::Cancelled),
            s = provider.stream(req) => s?,
        };
        let mut text = String::new();
        let mut name = String::new();
        let mut args = String::new();
        let mut id = String::new();
        loop {
            if ctx.cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let d = tokio::select! {
                biased;
                () = ctx.cancel.cancelled() => return Err(Error::Cancelled),
                d = stream.next() => d,
            };
            let Some(d) = d else {
                break;
            };
            match d? {
                StreamDelta::Text(t) => text.push_str(&t),
                StreamDelta::ToolCall {
                    id: i,
                    name: n,
                    arguments,
                } => {
                    if !i.is_empty() {
                        id = i;
                    }
                    if !n.is_empty() {
                        name = n;
                    }
                    args.push_str(&arguments);
                }
                _ => {}
            }
        }
        last = text.clone();
        if name.is_empty() {
            return Ok(last);
        }
        let parsed: serde_json::Value =
            serde_json::from_str(&args).unwrap_or(serde_json::Value::Null);
        let out = gated_execute(&name, &parsed, ctx)?;
        messages.push(crate::llm::Message {
            role: "assistant".into(),
            content: text,
            tool_call_id: None,
            tool_calls: Some(vec![crate::llm::AssistantToolCall {
                id: id.clone(),
                name: name.clone(),
                arguments: args,
            }]),
        });
        messages.push(crate::llm::Message {
            role: "tool".into(),
            content: out.text,
            tool_call_id: Some(id),
            tool_calls: None,
        });
        name.clear();
    }
    Ok(last)
}

/// Notify the UI that a child started/finished.
pub fn started_event(id: SubagentId, role: Role, task: &Task) -> AgentEvent {
    AgentEvent::SubagentStarted {
        id,
        role,
        description: task.title.clone(),
    }
}

pub fn finished_event(id: SubagentId, role: Role, summary: String, body: String) -> AgentEvent {
    AgentEvent::SubagentFinished {
        id,
        role,
        summary,
        body,
    }
}

/// Worktree parent for a session.
pub fn worktree_root(home: &Path, session_id: &str) -> PathBuf {
    home.join("worktrees").join(session_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_pass_fail() {
        assert!(parse_verdict("PASS\nlooks good"));
        assert!(!parse_verdict("FAIL\nbad"));
        assert!(!parse_verdict(""));
    }

    #[tokio::test]
    async fn planner_writes_workspace_note() {
        use crate::llm::{ReplayProvider, StreamDelta};
        use crate::queue::Task;
        use tempfile::TempDir;

        let ws = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        let notes = ws.path().join(".ryter-notes");
        std::fs::create_dir_all(&notes).unwrap();
        let p = ReplayProvider::scripted(vec![vec![
            StreamDelta::Text("scope the CLI\n".into()),
            StreamDelta::Done,
        ]]);
        let out = run_note_task(
            std::sync::Arc::new(p),
            ws.path(),
            home.path(),
            &Task {
                id: "t1".into(),
                title: "draft plan".into(),
                status: TaskStatus::Pending,
                retries: 0,
                findings: String::new(),
            },
            "grok-4.6",
            Role::Planner,
            true,
            false,
            Some(ws.path()),
            false,
            None,
            notes,
            "",
            crate::cancel::Cancel::new(),
        )
        .await
        .unwrap();
        assert_eq!(out.status, TaskStatus::Done);
        let disk = std::fs::read_to_string(ws.path().join("notes/plan.md")).unwrap();
        assert!(disk.contains("scope the CLI"), "{disk}");
    }

    #[tokio::test]
    async fn builder_audits_and_merges() {
        use crate::git;
        use crate::llm::{ReplayProvider, StreamDelta};
        use crate::queue::Task;
        use tempfile::TempDir;

        let repo = TempDir::new().unwrap();
        git::init_repo(repo.path()).unwrap();
        let home = TempDir::new().unwrap();
        let write_args = serde_json::json!({
            "path": "extra.txt",
            "content": "hello from worker\n"
        })
        .to_string();
        let provider = ReplayProvider::scripted(vec![
            vec![
                StreamDelta::ToolCall {
                    id: "1".into(),
                    name: "write".into(),
                    arguments: write_args,
                },
                StreamDelta::Done,
            ],
            vec![StreamDelta::Text("wrote extra".into()), StreamDelta::Done],
            vec![StreamDelta::Text("PASS".into()), StreamDelta::Done],
        ]);
        let task = Task {
            id: "t1".into(),
            title: "add extra.txt".into(),
            status: crate::queue::TaskStatus::Running,
            retries: 0,
            findings: String::new(),
        };
        let provider = std::sync::Arc::new(provider);
        let out = run_build_task(
            provider.clone(),
            repo.path(),
            home.path(),
            "sess01",
            &task,
            "grok-4.6",
            provider.clone(),
            "grok-4.6",
            true,
            false,
            true,
            2,
            None,
            false,
            None,
            crate::cancel::Cancel::new(),
        )
        .await
        .unwrap();
        assert_eq!(out.status, crate::queue::TaskStatus::Done, "{out:?}");
        let body = std::fs::read_to_string(repo.path().join("extra.txt")).unwrap();
        assert!(body.contains("hello from worker"));
    }

    #[tokio::test]
    async fn auditor_fail_blocks_without_merge() {
        use crate::git;
        use crate::llm::{ReplayProvider, StreamDelta};
        use crate::queue::Task;
        use tempfile::TempDir;

        let repo = TempDir::new().unwrap();
        git::init_repo(repo.path()).unwrap();
        let home = TempDir::new().unwrap();
        let write_args = serde_json::json!({
            "path": "nope.txt",
            "content": "secret\n"
        })
        .to_string();
        let provider = ReplayProvider::scripted(vec![
            vec![
                StreamDelta::ToolCall {
                    id: "1".into(),
                    name: "write".into(),
                    arguments: write_args,
                },
                StreamDelta::Done,
            ],
            vec![StreamDelta::Text("done".into()), StreamDelta::Done],
            vec![StreamDelta::Text("FAIL\nbad".into()), StreamDelta::Done],
        ]);
        let task = Task {
            id: "t2".into(),
            title: "bad file".into(),
            status: crate::queue::TaskStatus::Running,
            retries: 0,
            findings: String::new(),
        };
        let provider = std::sync::Arc::new(provider);
        let out = run_build_task(
            provider.clone(),
            repo.path(),
            home.path(),
            "sess02",
            &task,
            "grok-4.6",
            provider.clone(),
            "grok-4.6",
            true,
            false,
            true,
            0,
            None,
            false,
            None,
            crate::cancel::Cancel::new(),
        )
        .await
        .unwrap();
        assert_eq!(out.status, crate::queue::TaskStatus::Blocked);
        assert!(!repo.path().join("nope.txt").exists());
    }
}
