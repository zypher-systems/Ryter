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
    /// What the orchestrator is told: status, the builder's handback, check
    /// results, and the audit. It is the only way results reach the one role
    /// that writes project memory.
    pub report: String,
}

/// Parse the auditor's verdict.
///
/// The contract (`prompts/auditor.md`) is a `VERDICT: PASS` or `VERDICT: FAIL`
/// line; the last one wins, since reviewers often restate the criteria before
/// deciding. A bare `PASS`/`FAIL` first line is still accepted. Anything else
/// is a fail: an unparseable review must not merge.
pub fn parse_verdict(text: &str) -> bool {
    let clean = |l: &str| {
        l.trim()
            .trim_matches(|c: char| matches!(c, '*' | '#' | '`' | '_' | '>' | ' '))
            .to_ascii_uppercase()
    };
    let mut verdict = None;
    for line in text.lines() {
        let l = clean(line);
        if let Some(rest) = l.strip_prefix("VERDICT") {
            let rest = rest.trim_start_matches([':', ' ', '*', '-']);
            if rest.starts_with("PASS") {
                verdict = Some(true);
            } else if rest.starts_with("FAIL") {
                verdict = Some(false);
            }
        }
    }
    if let Some(v) = verdict {
        return v;
    }
    text.lines()
        .map(clean)
        .find(|l| !l.is_empty())
        .is_some_and(|l| l.starts_with("PASS"))
}

/// Everything a build task needs besides the task itself.
pub struct BuildJob<'a> {
    /// Builder inference.
    pub provider: Arc<dyn Provider>,
    /// Builder model.
    pub model: &'a str,
    /// Auditor inference. Ideally a different model from the builder.
    pub auditor_provider: Arc<dyn Provider>,
    /// Auditor model.
    pub auditor_model: &'a str,
    /// The user's repository.
    pub repo: &'a Path,
    /// `~/.ryter`.
    pub home: &'a Path,
    /// Session id (worktree paths and branch names).
    pub session_id: &'a str,
    /// Project root for instructions and memory.
    pub project_root: Option<&'a Path>,
    /// Trusted project.
    pub trusted: bool,
    /// Treat Ask as Allow for the builder.
    pub always_approve: bool,
    /// Web tools on.
    pub web: bool,
    /// When false nothing lands: the branch is left for the user.
    pub auditor_enabled: bool,
    /// Commands the harness runs before the auditor.
    pub checks: &'a [String],
    /// Wall clock per check.
    pub check_timeout: std::time::Duration,
    /// Builder retries after a failed gate.
    pub max_retries: u32,
    /// Lifecycle hooks.
    pub hooks: Option<Arc<crate::hooks::HookSet>>,
    /// This task's cancel.
    pub cancel: Arc<Cancel>,
}

/// Serializes landing across parallel builders. A merge is a handful of
/// synchronous git calls, but it must see a stable target branch.
static LAND: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Times the target may move under a task before it gives up.
const MAX_INTEGRATIONS: usize = 4;

/// Run one build task: builder in a worktree, integrate the target branch into
/// that worktree, gate (checks, then auditor sign-off), land.
pub async fn run_build_task(job: &BuildJob<'_>, task: &Task) -> Result<TaskOutcome> {
    if job.cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    if !git::is_repo(job.repo) {
        return Ok(outcome(
            task,
            TaskStatus::Blocked,
            "workspace is not a git repo",
            "",
            "",
        ));
    }
    let onto = git::branch(job.repo)?;
    if onto == "HEAD" {
        return Ok(outcome(
            task,
            TaskStatus::Blocked,
            "your checkout is a detached HEAD; check out a branch for work to land on",
            "",
            "",
        ));
    }
    let slug: String = task
        .id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(32)
        .collect();
    let sid: String = job
        .session_id
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(8)
        .collect();
    let branch = format!("ryter-{sid}-{slug}");
    let root = job.home.join("worktrees").join(job.session_id);
    let wt = root.join(&task.id);
    // Scratch lives beside the worktree, never inside it: anything inside is
    // committed and merged into the user's repository.
    let scratch = root.join(format!("{}.scratch", task.id));
    let _ = std::fs::create_dir_all(&scratch);
    git::add_worktree(job.repo, &wt, &branch)?;
    let result = build_inner(job, task, &onto, &wt, &branch, &scratch).await;
    let _ = std::fs::remove_dir_all(&scratch);
    match result {
        Ok(o) => Ok(o),
        Err(e) => {
            let _ = git::remove_worktree(job.repo, &wt, &branch);
            Err(e)
        }
    }
}

/// Gate state carried across integrations.
#[derive(Default)]
struct Gate {
    /// Worktree HEAD at which the checks last passed.
    checked_at: Option<String>,
    /// The auditor has signed off on the builder's code as it stands.
    signed_off: bool,
    /// Check results, for the report.
    checks: String,
    /// Auditor text, for the report.
    audit: String,
}

async fn build_inner(
    job: &BuildJob<'_>,
    task: &Task,
    onto: &str,
    wt: &Path,
    branch: &str,
    scratch: &Path,
) -> Result<TaskOutcome> {
    let ctx = ToolContext {
        workspace: wt.to_path_buf(),
        notes_dir: scratch.to_path_buf(),
        role: Role::Builder,
        always_approve: job.always_approve,
        queue: Arc::new(std::sync::Mutex::new(crate::queue::TaskQueue::open(
            scratch.join("tasks.json"),
        ))),
        mcp: None,
        hooks: job.hooks.clone(),
        cancel: job.cancel.clone(),
        user_io: None,
        sticky_approve: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        web: job.web,
    };

    let msgs = specialist_messages(
        job.home,
        job.project_root,
        job.trusted,
        Role::Builder,
        "",
        &builder_brief(task),
    );
    let handback =
        run_specialist(job.provider.as_ref(), job.model, Role::Builder, msgs, &ctx).await?;
    git::commit_all(wt, &format!("ryter: {}", task.title))?;

    let mut gate = Gate::default();
    for _ in 0..MAX_INTEGRATIONS {
        // 1. Pull the target into the worktree. Conflicts land here, in the
        //    builder's copy, and a builder resolves them — never the user's.
        let target = git::rev(job.repo, onto)?;
        if let git::Integration::Conflict(files) = git::integrate(wt, onto)? {
            let prompt = format!(
                "Merging `{onto}` into your branch conflicted in:\n{}\n\n\
                 Resolve every conflict marker, keeping both the upstream change \
                 and the intent of your task:\n\n{}",
                files.join("\n"),
                builder_brief(task),
            );
            let msgs = specialist_messages(
                job.home,
                job.project_root,
                job.trusted,
                Role::Builder,
                "",
                &prompt,
            );
            let _ =
                run_specialist(job.provider.as_ref(), job.model, Role::Builder, msgs, &ctx).await?;
            // A path stays "unmerged" until it is staged, so stage first, then
            // refuse if anything is still unmerged or still carries markers.
            let _ = git::git(wt, &["add", "-A"]);
            let unresolved = !git::unmerged(wt).is_empty()
                || files.iter().any(|f| has_conflict_markers(&wt.join(f)));
            if unresolved || git::commit_all(wt, &format!("ryter: integrate {onto}")).is_err() {
                git::merge_abort(wt);
                return Ok(keep_branch(
                    job,
                    task,
                    wt,
                    branch,
                    format!(
                        "could not resolve conflicts with `{onto}` in {}",
                        files.join(", ")
                    ),
                    &handback,
                    &gate,
                ));
            }
            // A resolver may have changed any code: it must be signed off again.
            gate.signed_off = false;
        }

        // 2. Gate. Checks rerun whenever the tree changed; the auditor reruns
        //    only when the builder's own code changed.
        let head = git::head(wt)?;
        if gate.checked_at.as_deref() != Some(head.as_str()) {
            match run_checks(job, wt, &ctx) {
                Ok(out) => {
                    gate.checks = out;
                    gate.checked_at = Some(head.clone());
                }
                Err(out) => {
                    gate.checks = out.clone();
                    return Ok(failed(
                        job,
                        task,
                        wt,
                        branch,
                        &format!("checks failed:\n{out}"),
                        &handback,
                        &gate,
                    ));
                }
            }
        }
        if !job.auditor_enabled {
            return Ok(keep_branch(
                job,
                task,
                wt,
                branch,
                "the auditor is off, so nothing merges without review".into(),
                &handback,
                &gate,
            ));
        }
        if !gate.signed_off {
            let audit = audit(job, task, wt, &ctx, &target, &handback, &gate).await?;
            gate.signed_off = parse_verdict(&audit);
            gate.audit = audit;
            if !gate.signed_off {
                let findings = gate.audit.clone();
                return Ok(failed(job, task, wt, branch, &findings, &handback, &gate));
            }
        }

        // 3. Land, holding the lock so the target cannot move underneath.
        let _held = LAND.lock().await;
        if git::branch(job.repo)? != onto {
            return Ok(keep_branch(
                job,
                task,
                wt,
                branch,
                format!("you switched away from `{onto}` during the build"),
                &handback,
                &gate,
            ));
        }
        if git::rev(job.repo, onto)? != target {
            // Someone else landed first. Integrate again and re-gate.
            continue;
        }
        let touched = git::changed_paths(wt, &target, "HEAD");
        let dirty = git::dirty_paths(job.repo);
        let clash: Vec<&String> = touched.iter().filter(|p| dirty.contains(p)).collect();
        if !clash.is_empty() {
            // Git would refuse, or the merge would bury the user's edits.
            // Uncommitted work elsewhere in the tree is fine and stays put.
            return Ok(keep_branch(
                job,
                task,
                wt,
                branch,
                format!(
                    "you have uncommitted changes to files this task also changed: {}",
                    clash
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                &handback,
                &gate,
            ));
        }
        if let Err(e) = git::land(job.repo, branch, &format!("ryter: {}", task.title)) {
            return Ok(keep_branch(
                job,
                task,
                wt,
                branch,
                format!("merge failed and was aborted: {e}"),
                &handback,
                &gate,
            ));
        }
        git::remove_worktree(job.repo, wt, branch)?;
        let summary = format!(
            "merged {} onto {onto}  undo: git revert -m 1 HEAD  (or reset to {})",
            task.title,
            &target[..target.len().min(12)]
        );
        return Ok(TaskOutcome {
            id: task.id.clone(),
            status: TaskStatus::Done,
            findings: gate.audit.clone(),
            report: report(task, "merged", &summary, &handback, &gate),
            summary,
        });
    }
    Ok(keep_branch(
        job,
        task,
        wt,
        branch,
        format!("`{onto}` kept moving; gave up after {MAX_INTEGRATIONS} integrations"),
        &handback,
        &gate,
    ))
}

/// True when a file still holds a conflict marker line.
fn has_conflict_markers(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|t| {
        t.lines()
            .any(|l| l.starts_with("<<<<<<< ") || l.starts_with(">>>>>>> ") || l == "=======")
    })
}

/// The builder's whole brief. It used to be the title alone.
fn builder_brief(task: &Task) -> String {
    let mut s = format!("Task: {}\n", task.title);
    if !task.brief.trim().is_empty() {
        s.push_str("\nBrief:\n");
        s.push_str(task.brief.trim());
        s.push('\n');
    }
    if !task.files.is_empty() {
        s.push_str("\nYou own these paths; other builders are working elsewhere in parallel. Stay inside them unless the task cannot be done otherwise, and say so in your handback if you had to:\n");
        for f in &task.files {
            s.push_str("- ");
            s.push_str(f);
            s.push('\n');
        }
    }
    if !task.findings.trim().is_empty() {
        s.push_str("\nA previous attempt was rejected. Address this first:\n");
        s.push_str(task.findings.trim());
        s.push('\n');
    }
    s
}

/// Run the configured checks in the worktree. `Err` carries the failing output.
fn run_checks(
    job: &BuildJob<'_>,
    wt: &Path,
    ctx: &ToolContext,
) -> std::result::Result<String, String> {
    use crate::tools::shell::{Run, run_command};
    let mut out = String::new();
    for cmd in job.checks {
        let run = run_command(cmd, wt, job.check_timeout, &ctx.cancel)
            .map_err(|e| format!("$ {cmd}\n{e}"))?;
        let (ok, text) = match run {
            Run::Ok(t) => (true, t),
            Run::Failed(t) => (false, t),
            Run::Cancelled => (false, "cancelled".into()),
            Run::TimedOut => (
                false,
                format!("timed out after {}s", job.check_timeout.as_secs()),
            ),
        };
        let block = format!(
            "$ {cmd}  → {}\n{}\n",
            if ok { "ok" } else { "FAILED" },
            crate::tools::cap_output(text)
        );
        out.push_str(&block);
        if !ok {
            return Err(out);
        }
    }
    Ok(out)
}

async fn audit(
    job: &BuildJob<'_>,
    task: &Task,
    wt: &Path,
    ctx: &ToolContext,
    target: &str,
    handback: &str,
    gate: &Gate,
) -> Result<String> {
    // Everything landing would change, committed or not, new files included.
    // The old diff read the worktree before staging, so new files were
    // invisible and a builder's own commits dropped out.
    let diff = crate::tools::cap_output(git::diff_range(wt, target, "HEAD"));
    let checks = if job.checks.is_empty() {
        "No checks are configured for this project. Run its tests yourself before you pass the work.".to_string()
    } else {
        format!(
            "The harness ran these checks on this exact tree; all passed:\n{}",
            gate.checks
        )
    };
    let body = format!(
        "{}\nBuilder's handback:\n{handback}\n\n{checks}\n\n\
         The full change is below (`git diff {short} HEAD` in your workspace \
         shows it again, per path if it was truncated here).\n\n{diff}",
        builder_brief(task),
        short = &target[..target.len().min(12)],
    );
    let audit_ctx = ToolContext {
        role: Role::Auditor,
        ..ctx.clone()
    };
    let msgs = specialist_messages(
        job.home,
        job.project_root,
        job.trusted,
        Role::Auditor,
        "",
        &body,
    );
    run_specialist(
        job.auditor_provider.as_ref(),
        job.auditor_model,
        Role::Auditor,
        msgs,
        &audit_ctx,
    )
    .await
}

/// The gate refused: drop the attempt so a retry starts from a clean branch.
fn failed(
    job: &BuildJob<'_>,
    task: &Task,
    wt: &Path,
    branch: &str,
    findings: &str,
    handback: &str,
    gate: &Gate,
) -> TaskOutcome {
    let _ = git::remove_worktree(job.repo, wt, branch);
    let retries = task.retries + 1;
    let status = if retries > job.max_retries {
        TaskStatus::Blocked
    } else {
        TaskStatus::Pending
    };
    let summary = if status == TaskStatus::Blocked {
        format!("rejected {retries} times; blocked")
    } else {
        "rejected; retrying".into()
    };
    TaskOutcome {
        id: task.id.clone(),
        status,
        findings: findings.to_string(),
        report: report(task, &summary, findings, handback, gate),
        summary,
    }
}

/// Work that must not land automatically: keep the branch for the user.
fn keep_branch(
    job: &BuildJob<'_>,
    task: &Task,
    wt: &Path,
    branch: &str,
    why: String,
    handback: &str,
    gate: &Gate,
) -> TaskOutcome {
    git::remove_worktree_keep_branch(job.repo, wt);
    let summary = format!("not merged: {why}. Branch `{branch}` has the work.");
    TaskOutcome {
        id: task.id.clone(),
        status: TaskStatus::Blocked,
        findings: why.clone(),
        report: report(task, "not merged", &summary, handback, gate),
        summary,
    }
}

fn outcome(task: &Task, status: TaskStatus, why: &str, handback: &str, audit: &str) -> TaskOutcome {
    TaskOutcome {
        id: task.id.clone(),
        status,
        findings: why.to_string(),
        summary: why.to_string(),
        report: format!("### {} — {}\n{why}\n{handback}{audit}", task.id, task.title),
    }
}

fn report(task: &Task, status: &str, detail: &str, handback: &str, gate: &Gate) -> String {
    let mut s = format!("### {} — {} ({status})\n{detail}\n", task.id, task.title);
    if !handback.trim().is_empty() {
        s.push_str("\nBuilder handback:\n");
        s.push_str(handback.trim());
        s.push('\n');
    }
    if !gate.checks.trim().is_empty() {
        s.push_str("\nChecks:\n");
        s.push_str(&crate::tools::cap_output(gate.checks.clone()));
    }
    if !gate.audit.trim().is_empty() {
        s.push_str("\nAudit:\n");
        s.push_str(gate.audit.trim());
        s.push('\n');
    }
    s
}

const HANDBACK_CAP: usize = 12_000;

/// Architect / extra auditor: no worktree. Writes memory files via tools
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
    queue: Arc<std::sync::Mutex<crate::queue::TaskQueue>>,
) -> Result<TaskOutcome> {
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    // The session queue, so an architect's task list is what builders run.
    // It used to be a throwaway queue at `<repo>/tasks.json`: the tasks never
    // reached a builder, and the file landed in the user's project.
    let ctx = ToolContext {
        workspace: workspace.to_path_buf(),
        notes_dir,
        role,
        always_approve,
        queue,
        mcp: None,
        hooks,
        cancel,
        user_io: None,
        sticky_approve: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        web,
    };
    let msgs = specialist_messages(
        home,
        project_root,
        trusted,
        role,
        pass_note,
        &builder_brief(task),
    );
    let text = run_specialist(provider.as_ref(), model, role, msgs, &ctx).await?;
    let body = clip_handback(&text);
    persist_workspace_note(workspace, role, &body);
    Ok(TaskOutcome {
        id: task.id.clone(),
        status: TaskStatus::Done,
        report: format!("### {} — {} ({role})\n{body}\n", task.id, task.title),
        findings: body.clone(),
        summary: first_line(&body).unwrap_or_else(|| format!("{} {}", role, task.title)),
    })
}

fn persist_workspace_note(workspace: &Path, role: Role, body: &str) {
    let name = match role {
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

/// Tool rounds a specialist may take before it must hand back.
const SPECIALIST_ROUNDS: usize = 40;
/// Output ceiling per specialist response. A builder writing a whole file puts
/// that file in its tool arguments; 4096 truncated them mid-JSON.
const SPECIALIST_MAX_TOKENS: u32 = 16_384;

async fn run_specialist(
    provider: &dyn Provider,
    model: &str,
    role: Role,
    mut messages: Vec<crate::llm::Message>,
    ctx: &ToolContext,
) -> Result<String> {
    let mut last = String::new();
    for _ in 0..SPECIALIST_ROUNDS {
        if ctx.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let req = CompletionRequest {
            model: model.to_string(),
            system: None,
            messages: messages.clone(),
            tools: crate::tools::specs_for_opts(role, ctx.web),
            max_tokens: Some(SPECIALIST_MAX_TOKENS),
        };
        let mut stream = tokio::select! {
            biased;
            () = ctx.cancel.cancelled() => return Err(Error::Cancelled),
            s = provider.stream(req) => s?,
        };
        let mut text = String::new();
        // Several calls may arrive in one response; tracking a single name and
        // argument string glued their fragments into one unparseable call.
        let mut calls = crate::llm::ToolCallAccumulator::default();
        loop {
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
                    id,
                    name,
                    arguments,
                } => calls.push(&id, &name, &arguments),
                _ => {}
            }
        }
        last = text.clone();
        let calls = calls.finish();
        if calls.is_empty() {
            return Ok(last);
        }
        messages.push(crate::llm::Message {
            role: "assistant".into(),
            content: text,
            tool_call_id: None,
            tool_calls: Some(calls.clone()),
        });
        for call in calls {
            if ctx.cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let parsed: serde_json::Value =
                serde_json::from_str(&call.arguments).unwrap_or(serde_json::Value::Null);
            let out = gated_execute(&call.name, &parsed, ctx)?;
            messages.push(crate::llm::Message {
                role: "tool".into(),
                content: out.text,
                tool_call_id: Some(call.id),
                tool_calls: None,
            });
        }
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
    use crate::llm::{CompletionRequest, DeltaStream, ModelInfo, ReplayProvider, StreamDelta};
    use std::sync::Mutex;
    use tempfile::TempDir;

    #[test]
    fn verdict_contract() {
        let cases: &[(&str, bool)] = &[
            ("VERDICT: PASS", true),
            ("VERDICT: FAIL\n- src/a.rs: unwrap on user input", false),
            // The auditor restates criteria, then decides: the last line wins.
            (
                "Criteria: VERDICT: FAIL if tests fail.\n\nAll good.\nVERDICT: PASS",
                true,
            ),
            ("## Review\nLooks fine.\n\n**VERDICT: PASS**", true),
            ("verdict - fail", false),
            // Bare first line still works.
            ("PASS\nlooks good", true),
            ("FAIL\nbad", false),
            // Anything unparseable must not merge.
            ("## Review\nlooks good to me", false),
            ("", false),
        ];
        for (text, want) in cases {
            assert_eq!(parse_verdict(text), *want, "{text:?}");
        }
    }

    fn task(id: &str, title: &str) -> Task {
        Task {
            id: id.into(),
            title: title.into(),
            brief: String::new(),
            files: Vec::new(),
            status: TaskStatus::Running,
            retries: 0,
            findings: String::new(),
        }
    }

    fn write_call(path: &str, content: &str) -> Vec<StreamDelta> {
        vec![
            StreamDelta::ToolCall {
                id: "w".into(),
                name: "write".into(),
                arguments: serde_json::json!({"path": path, "content": content}).to_string(),
            },
            StreamDelta::Done,
        ]
    }

    fn say(text: &str) -> Vec<StreamDelta> {
        vec![StreamDelta::Text(text.into()), StreamDelta::Done]
    }

    /// A hook run before the given turn number.
    type BeforeTurn = Option<(usize, Box<dyn FnOnce() + Send>)>;

    /// Replays scripted turns, records every request, and can run a hook before
    /// a given turn (to move the target branch mid-build).
    struct Scripted {
        inner: ReplayProvider,
        seen: Mutex<Vec<CompletionRequest>>,
        before: Mutex<BeforeTurn>,
    }

    impl Scripted {
        fn new(turns: Vec<Vec<StreamDelta>>) -> Arc<Self> {
            Arc::new(Self {
                inner: ReplayProvider::scripted(turns),
                seen: Mutex::new(Vec::new()),
                before: Mutex::new(None),
            })
        }
        fn before_turn(&self, n: usize, f: impl FnOnce() + Send + 'static) {
            *self.before.lock().unwrap() = Some((n, Box::new(f)));
        }
        fn request(&self, n: usize) -> String {
            let seen = self.seen.lock().unwrap();
            seen[n]
                .messages
                .iter()
                .map(|m| m.content.clone())
                .collect::<Vec<_>>()
                .join("\n")
        }
        fn calls(&self) -> usize {
            self.seen.lock().unwrap().len()
        }
    }

    #[async_trait::async_trait]
    impl Provider for Scripted {
        async fn stream(&self, req: CompletionRequest) -> Result<DeltaStream> {
            let n = {
                let mut seen = self.seen.lock().unwrap();
                seen.push(req.clone());
                seen.len() - 1
            };
            let hook = {
                let mut b = self.before.lock().unwrap();
                match b.take() {
                    Some((at, f)) if at == n => Some(f),
                    other => {
                        *b = other;
                        None
                    }
                }
            };
            if let Some(f) = hook {
                f();
            }
            self.inner.stream(req).await
        }
        async fn list_models(&self) -> Result<Vec<ModelInfo>> {
            Ok(Vec::new())
        }
    }

    struct Fixture {
        repo: TempDir,
        home: TempDir,
    }

    fn fixture() -> Fixture {
        let repo = TempDir::new().unwrap();
        git::init_repo(repo.path()).unwrap();
        Fixture {
            repo,
            home: TempDir::new().unwrap(),
        }
    }

    async fn run(
        f: &Fixture,
        p: &Arc<Scripted>,
        t: &Task,
        auditor_enabled: bool,
        checks: &[String],
    ) -> TaskOutcome {
        let job = BuildJob {
            provider: p.clone(),
            model: "m",
            auditor_provider: p.clone(),
            auditor_model: "m",
            repo: f.repo.path(),
            home: f.home.path(),
            session_id: "sess0001",
            project_root: None,
            trusted: false,
            always_approve: true,
            web: false,
            auditor_enabled,
            checks,
            check_timeout: std::time::Duration::from_secs(30),
            max_retries: 0,
            hooks: None,
            cancel: Cancel::new(),
        };
        run_build_task(&job, t).await.unwrap()
    }

    fn mid_merge(repo: &Path) -> bool {
        repo.join(".git/MERGE_HEAD").exists()
    }

    #[tokio::test]
    async fn architect_writes_workspace_note_and_real_tasks() {
        let ws = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        let notes = home.path().join("notes");
        std::fs::create_dir_all(&notes).unwrap();
        let todo = serde_json::json!({"items": [
            {"id": "b1", "title": "add parser", "brief": "parse x", "files": ["src/parse.rs"]}
        ]})
        .to_string();
        let p = ReplayProvider::scripted(vec![
            vec![
                StreamDelta::ToolCall {
                    id: "t".into(),
                    name: "todo_write".into(),
                    arguments: todo,
                },
                StreamDelta::Done,
            ],
            say("shape: one parser module\n"),
        ]);
        let queue = Arc::new(Mutex::new(crate::queue::TaskQueue::open(
            home.path().join("tasks.json"),
        )));
        let out = run_note_task(
            Arc::new(p),
            ws.path(),
            home.path(),
            &task("a1", "design the parser"),
            "m",
            Role::Architect,
            true,
            false,
            Some(ws.path()),
            false,
            None,
            notes,
            "",
            Cancel::new(),
            queue.clone(),
        )
        .await
        .unwrap();
        assert_eq!(out.status, TaskStatus::Done);
        let disk = std::fs::read_to_string(ws.path().join("notes/architect.md")).unwrap();
        assert!(disk.contains("one parser module"), "{disk}");
        // The architect's tasks reach the queue builders read from…
        let q = queue.lock().unwrap();
        let b1 = q.tasks.iter().find(|t| t.id == "b1").expect("task queued");
        assert_eq!(b1.files, vec!["src/parse.rs".to_string()]);
        assert_eq!(b1.brief, "parse x");
        // …and nothing lands in the user's project root.
        assert!(!ws.path().join("tasks.json").exists());
    }

    #[tokio::test]
    async fn builder_passes_the_gate_and_lands_as_one_commit() {
        let f = fixture();
        let p = Scripted::new(vec![
            write_call("extra.txt", "hello from worker\n"),
            say("STATUS: DONE\nFILES: extra.txt"),
            say("VERDICT: PASS"),
        ]);
        let out = run(&f, &p, &task("t1", "add extra"), true, &[]).await;
        assert_eq!(out.status, TaskStatus::Done, "{out:?}");
        assert_eq!(
            std::fs::read_to_string(f.repo.path().join("extra.txt")).unwrap(),
            "hello from worker\n"
        );
        let log = git::git(f.repo.path(), &["log", "--merges", "--format=%s"]).unwrap();
        assert!(log.contains("ryter: add extra"), "{log}");
        // The orchestrator is told what happened.
        assert!(out.report.contains("STATUS: DONE") && out.report.contains("VERDICT: PASS"));
    }

    /// Bug: the audit diff was taken before staging, so new files were
    /// invisible and the auditor reviewed an empty change.
    #[tokio::test]
    async fn the_auditor_sees_new_files() {
        let f = fixture();
        let p = Scripted::new(vec![
            write_call("src/new_module.rs", "pub fn brand_new() {}\n"),
            say("STATUS: DONE"),
            say("VERDICT: PASS"),
        ]);
        run(&f, &p, &task("t1", "add a module"), true, &[]).await;
        let audit_request = p.request(2);
        assert!(
            audit_request.contains("src/new_module.rs") && audit_request.contains("brand_new"),
            "auditor never saw the new file:\n{audit_request}"
        );
    }

    #[tokio::test]
    async fn a_failed_audit_merges_nothing() {
        let f = fixture();
        let p = Scripted::new(vec![
            write_call("nope.txt", "secret\n"),
            say("done"),
            say("VERDICT: FAIL\n- nope.txt: not asked for"),
        ]);
        let out = run(&f, &p, &task("t2", "bad file"), true, &[]).await;
        assert_eq!(out.status, TaskStatus::Blocked);
        assert!(!f.repo.path().join("nope.txt").exists());
        assert!(out.findings.contains("not asked for"));
    }

    /// The harness runs the checks; a failing check rejects the work before
    /// an auditor is asked (and paid) to review it.
    #[tokio::test]
    async fn a_failing_check_rejects_before_audit() {
        let f = fixture();
        let p = Scripted::new(vec![
            write_call("extra.txt", "x\n"),
            say("done"),
            say("VERDICT: PASS"),
        ]);
        let checks = vec!["echo compiling; echo 'error: boom' >&2; false".to_string()];
        let out = run(&f, &p, &task("t3", "breaks the build"), true, &checks).await;
        assert_eq!(out.status, TaskStatus::Blocked);
        assert!(out.findings.contains("error: boom"), "{out:?}");
        assert_eq!(p.calls(), 2, "the auditor must not be consulted");
        assert!(!f.repo.path().join("extra.txt").exists());
    }

    #[tokio::test]
    async fn passing_checks_are_shown_to_the_auditor() {
        let f = fixture();
        let p = Scripted::new(vec![
            write_call("extra.txt", "x\n"),
            say("done"),
            say("VERDICT: PASS"),
        ]);
        let checks = vec!["echo 'test result: ok. 3 passed'".to_string()];
        let out = run(&f, &p, &task("t4", "ok"), true, &checks).await;
        assert_eq!(out.status, TaskStatus::Done, "{out:?}");
        assert!(p.request(2).contains("3 passed"));
    }

    /// Sign-off is required to merge. With the auditor off, the work waits on
    /// its branch instead of landing unreviewed.
    #[tokio::test]
    async fn without_an_auditor_nothing_lands() {
        let f = fixture();
        let p = Scripted::new(vec![write_call("extra.txt", "x\n"), say("done")]);
        let out = run(&f, &p, &task("t5", "unreviewed"), false, &[]).await;
        assert_eq!(out.status, TaskStatus::Blocked);
        assert!(!f.repo.path().join("extra.txt").exists());
        assert!(out.summary.contains("auditor is off"), "{out:?}");
        let branches = git::git(f.repo.path(), &["branch", "--list", "ryter-*"]).unwrap();
        assert!(
            !branches.trim().is_empty(),
            "the branch must be kept for the user"
        );
    }

    /// Uncommitted edits to a file the task also changed block the merge;
    /// uncommitted edits elsewhere do not, and are left untouched.
    #[tokio::test]
    async fn dirty_files_block_only_when_they_overlap() {
        let f = fixture();
        std::fs::write(f.repo.path().join("README.md"), "mine, uncommitted\n").unwrap();
        let p = Scripted::new(vec![
            write_call("README.md", "builder's readme\n"),
            say("done"),
            say("VERDICT: PASS"),
        ]);
        let out = run(&f, &p, &task("t6", "edit readme"), true, &[]).await;
        assert_eq!(out.status, TaskStatus::Blocked);
        assert!(out.summary.contains("README.md"), "{out:?}");
        assert_eq!(
            std::fs::read_to_string(f.repo.path().join("README.md")).unwrap(),
            "mine, uncommitted\n"
        );
        assert!(!mid_merge(f.repo.path()));

        let f = fixture();
        std::fs::write(f.repo.path().join("README.md"), "mine, uncommitted\n").unwrap();
        let p = Scripted::new(vec![
            write_call("built.txt", "ok\n"),
            say("done"),
            say("VERDICT: PASS"),
        ]);
        let out = run(&f, &p, &task("t7", "add a file"), true, &[]).await;
        assert_eq!(out.status, TaskStatus::Done, "{out:?}");
        assert!(f.repo.path().join("built.txt").exists());
        assert_eq!(
            std::fs::read_to_string(f.repo.path().join("README.md")).unwrap(),
            "mine, uncommitted\n",
            "the user's uncommitted edit must survive the merge"
        );
    }

    /// The target moves mid-build and conflicts with the builder. The conflict
    /// is resolved in the worktree, the resolution is re-audited, and the
    /// user's checkout never holds a conflict marker.
    #[tokio::test]
    async fn conflicts_resolve_in_the_worktree_and_are_re_audited() {
        let f = fixture();
        let repo = f.repo.path().to_path_buf();
        let p = Scripted::new(vec![
            write_call("README.md", "builder line\n"),
            say("done"),
            // Resolver turn, then its handback.
            write_call("README.md", "upstream line\nbuilder line\n"),
            say("resolved"),
            say("VERDICT: PASS"),
        ]);
        // While the builder works, someone lands a conflicting commit.
        p.before_turn(1, move || {
            std::fs::write(repo.join("README.md"), "upstream line\n").unwrap();
            git::commit_all(&repo, "upstream edit").unwrap();
        });
        let out = run(&f, &p, &task("t8", "edit readme"), true, &[]).await;
        assert_eq!(out.status, TaskStatus::Done, "{out:?}");
        assert_eq!(
            std::fs::read_to_string(f.repo.path().join("README.md")).unwrap(),
            "upstream line\nbuilder line\n"
        );
        assert!(!mid_merge(f.repo.path()));
        // The resolver's result reached the auditor.
        assert!(p.request(4).contains("upstream line"), "{}", p.request(4));
    }
}
