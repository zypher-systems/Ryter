//! Spawn specialists from the task queue: worktrees, auditor, auto-merge.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::cancel::Cancel;
use crate::error::{Error, Result};
use crate::event::AgentEvent;
use crate::git;
use crate::ids::SubagentId;
use crate::llm::{CompletionRequest, Provider, StreamDelta};
use crate::meter::Meter;
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

/// One seat on the auditor panel.
#[derive(Clone)]
pub struct Auditor {
    /// Inference.
    pub provider: Arc<dyn Provider>,
    /// Model id.
    pub model: String,
    /// Connection name.
    pub connection: String,
    /// What this seat weighs most (`"security"`); empty for a general review.
    pub focus: String,
    /// Only reviews changes touching these globs; empty for every change.
    pub paths: Vec<String>,
}

impl Auditor {
    /// Whether this seat reviews a change touching `changed`.
    pub fn applies_to(&self, changed: &[String]) -> bool {
        self.paths.is_empty()
            || self
                .paths
                .iter()
                .any(|g| glob::Pattern::new(g).is_ok_and(|p| changed.iter().any(|c| p.matches(c))))
    }
}

/// Everything a build task needs besides the task itself.
pub struct BuildJob<'a> {
    /// Builder inference.
    pub provider: Arc<dyn Provider>,
    /// Builder model.
    pub model: &'a str,
    /// Builder connection name, for the spend log.
    pub connection: &'a str,
    /// Auditors that must all sign off, in order; they stop at the first FAIL.
    /// Each must be a different model from the lead and the builder.
    pub auditors: &'a [Auditor],
    /// Prices and caps every specialist round.
    pub meter: &'a Meter,
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
    /// Where progress is reported.
    pub progress: Option<Progress>,
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
    // A retry reopens the rejected attempt instead of starting over: fixing
    // findings in place costs a fraction of rebuilding the task from scratch.
    let resumed = git::open_worktree(job.repo, &wt, &branch)?;
    let result = build_inner(job, task, &onto, &wt, &branch, &scratch, resumed).await;
    let _ = std::fs::remove_dir_all(&scratch);
    match result {
        Ok(o) => Ok(o),
        // A cap stopped the work partway: keep what it produced.
        Err(Error::TaskBudget(why)) => {
            let gate = Gate {
                cost: job.meter.task(&task.id).label(),
                ..Gate::default()
            };
            Ok(keep_branch(job, task, &wt, &branch, why, "", &gate))
        }
        Err(e @ Error::Budget { .. }) => {
            git::remove_worktree_keep_branch(job.repo, &wt);
            Err(e)
        }
        Err(e) => {
            let _ = git::remove_worktree(job.repo, &wt, &branch);
            Err(e)
        }
    }
}

/// Gate state carried across integrations.
#[derive(Default, Clone)]
struct Gate {
    /// Worktree HEAD at which the checks last passed.
    checked_at: Option<String>,
    /// The auditor has signed off on the builder's code as it stands.
    signed_off: bool,
    /// Check results, for the report.
    checks: String,
    /// Auditor text, for the report.
    audit: String,
    /// Task cost so far, for the report.
    cost: String,
}

async fn build_inner(
    job: &BuildJob<'_>,
    task: &Task,
    onto: &str,
    wt: &Path,
    branch: &str,
    scratch: &Path,
    resumed: bool,
) -> Result<TaskOutcome> {
    let bill = Bill {
        meter: job.meter,
        task: &task.id,
        connection: job.connection,
        progress: job.progress.as_ref(),
    };
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

    let mut brief = builder_brief(task);
    if resumed {
        brief.push_str(
            "\nYour previous attempt is already committed in this worktree. Fix the \
             findings above in place; do not start over.\n",
        );
    }
    let msgs = specialist_messages(
        job.home,
        job.project_root,
        job.trusted,
        Role::Builder,
        "",
        &brief,
        &task.files,
    );
    let handback = run_specialist(
        job.provider.as_ref(),
        job.model,
        Role::Builder,
        msgs,
        &ctx,
        &bill,
    )
    .await?;
    git::commit_all(wt, &format!("ryter: {}", task.title))?;

    let mut gate = Gate::default();
    for _ in 0..MAX_INTEGRATIONS {
        // 1. Pull the target into the worktree. Conflicts land here, in the
        //    builder's copy, and a builder resolves them — never the user's.
        let target = git::rev(job.repo, onto)?;
        if let git::Integration::Conflict(files) = git::integrate(wt, onto)? {
            if !resolve_in(job, task, wt, onto, &files, &ctx, &bill).await? {
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
            match run_checks(job, wt, &ctx).await {
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
        if !job.auditor_enabled || job.auditors.is_empty() {
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
            match sign_off(job, task, wt, &ctx, &target, &handback, &mut gate).await? {
                SignOff::Passed => gate.signed_off = true,
                SignOff::Failed(findings) => {
                    return Ok(failed(job, task, wt, branch, &findings, &handback, &gate));
                }
                SignOff::Uncovered => {
                    return Ok(keep_branch(
                        job,
                        task,
                        wt,
                        branch,
                        "no auditor on the panel covers the paths this task changed".into(),
                        &handback,
                        &gate,
                    ));
                }
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
        gate.cost = job.meter.task(&task.id).label();
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

/// Have a builder resolve conflict markers in `wt`, then commit. Returns false
/// (after aborting the merge) when anything is still unresolved.
async fn resolve_in(
    job: &BuildJob<'_>,
    task: &Task,
    wt: &Path,
    onto: &str,
    files: &[String],
    ctx: &ToolContext,
    bill: &Bill<'_>,
) -> Result<bool> {
    let prompt = format!(
        "Merging `{onto}` into your branch conflicted in:\n{}\n\n\
         Resolve every conflict marker, keeping both the upstream change and \
         the intent of your task:\n\n{}",
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
        files,
    );
    let _ = run_specialist(
        job.provider.as_ref(),
        job.model,
        Role::Builder,
        msgs,
        ctx,
        bill,
    )
    .await?;
    // A path stays "unmerged" until it is staged, so stage first, then refuse
    // if anything is still unmerged or still carries markers.
    let _ = git::git(wt, &["add", "-A"]);
    let unresolved =
        !git::unmerged(wt).is_empty() || files.iter().any(|f| has_conflict_markers(&wt.join(f)));
    if unresolved || git::commit_all(wt, &format!("ryter: integrate {onto}")).is_err() {
        git::merge_abort(wt);
        return Ok(false);
    }
    Ok(true)
}

/// How the auditor panel ruled.
enum SignOff {
    /// Every seat that covers the change passed it.
    Passed,
    /// A seat failed it; its findings.
    Failed(String),
    /// No seat covers the changed paths, so nobody can sign off.
    Uncovered,
}

/// Run the panel over everything `wt` would land on top of `target`. Seats run
/// in order and stop at the first FAIL, so a cheap general reviewer first
/// saves paying a specialist to reject the same work.
#[allow(clippy::too_many_arguments)]
async fn sign_off(
    job: &BuildJob<'_>,
    task: &Task,
    wt: &Path,
    ctx: &ToolContext,
    target: &str,
    handback: &str,
    gate: &mut Gate,
) -> Result<SignOff> {
    let changed = git::changed_paths(wt, target, "HEAD");
    let mut reviews = Vec::new();
    for seat in job.auditors.iter().filter(|a| a.applies_to(&changed)) {
        let text = audit(job, seat, task, wt, ctx, target, handback, gate).await;
        // Reviewers probe: they write scratch tests, run them, sometimes leave
        // them. The builder's work is committed, so reset to it. On the first
        // live crew run an audit probe was swept into the next commit.
        git::discard_uncommitted(wt);
        let text = text?;
        let pass = parse_verdict(&text);
        let lens = if seat.focus.is_empty() {
            "review"
        } else {
            seat.focus.as_str()
        };
        reviews.push(format!("[{} · {lens}]\n{}", seat.model, text.trim()));
        gate.audit = reviews.join("\n\n");
        if !pass {
            return Ok(SignOff::Failed(gate.audit.clone()));
        }
    }
    Ok(if reviews.is_empty() {
        SignOff::Uncovered
    } else {
        SignOff::Passed
    })
}

/// The same model, whichever route reached it: `x-ai/grok-4.6`, `grok-4.6`,
/// and `grok-4.6-latest` are one model.
pub fn same_model(a: &str, b: &str) -> bool {
    let norm = |m: &str| {
        let m = m
            .rsplit('/')
            .next()
            .unwrap_or(m)
            .trim()
            .to_ascii_lowercase();
        m.trim_end_matches("-latest").to_string()
    };
    norm(a) == norm(b)
}

/// Why this panel cannot sign off work built under `lead` and `builder`, if
/// it cannot. A pass from the model that wrote (or directs) the code is not a
/// second opinion, so every auditor must be a different model from both.
pub fn independence_problem(lead: &str, builder: &str, panel: &[Auditor]) -> Option<String> {
    if !panel.iter().any(|a| a.paths.is_empty()) {
        return Some(
            "the auditor panel needs at least one seat without `paths`, so every change has a reviewer"
                .into(),
        );
    }
    for a in panel {
        for (who, model) in [("lead", lead), ("builder", builder)] {
            if same_model(&a.model, model) {
                return Some(format!(
                    "the auditor ({}) is the same model as the {who} ({model}). Sign-off \
                     must come from a different model: open /crew and press `b` for the \
                     crew builder (or run `ryter crew suggest --apply`), assign one with \
                     /crew → auditor, or add seats under [[auditor.panel]]. \
                     If you already assigned one, check its connection has a key: an \
                     unresolvable route falls back to the lead's model.",
                    a.model
                ));
            }
        }
    }
    None
}

/// True when a file still holds a conflict marker line.
fn has_conflict_markers(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|t| {
        t.lines()
            .any(|l| l.starts_with("<<<<<<< ") || l.starts_with(">>>>>>> ") || l == "=======")
    })
}

/// What happened when the crew tried to land its patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchLanding {
    /// On the user's branch as one commit.
    Landed(String),
    /// Held back; why, and what would unblock it.
    Waiting(String),
}

/// Land a finished patch on the user's branch (`job.repo`) as one commit.
///
/// Every task was checked and signed off against the patch branch as it stood
/// then; the combined tree is checked once more here, because two tasks that
/// pass alone can still break each other. If the user committed meanwhile,
/// their branch is integrated into the patch first — in the patch worktree,
/// never in their checkout — and any resolution goes back to the auditors.
pub async fn land_patch(job: &BuildJob<'_>, patch: &crate::session::Patch) -> Result<PatchLanding> {
    let user = job.repo;
    let wt = patch.worktree.as_path();
    let _held = LAND.lock().await;
    let current = git::branch(user)?;
    if current != patch.target {
        return Ok(PatchLanding::Waiting(format!(
            "you are on `{current}`, but the patch lands on `{}`. Switch back and say continue.",
            patch.target
        )));
    }
    let scratch = wt.with_extension("scratch");
    let _ = std::fs::create_dir_all(&scratch);
    let task = Task {
        id: "patch".into(),
        title: format!("integrate {} into the patch", patch.target),
        brief: format!(
            "The user committed to `{}` while the crew worked. Bring those commits \
             into the patch, keeping both the user's changes and the crew's work.",
            patch.target
        ),
        files: Vec::new(),
        by: "orchestrator".into(),
        role: "builder".into(),
        hold: false,
        status: TaskStatus::Running,
        retries: 0,
        findings: String::new(),
    };
    let ctx = ToolContext {
        workspace: wt.to_path_buf(),
        notes_dir: scratch.clone(),
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
    let bill = Bill {
        meter: job.meter,
        task: &task.id,
        connection: job.connection,
        progress: job.progress.as_ref(),
    };
    let mut gate = Gate::default();
    let user_head = git::rev(user, &patch.target)?;
    if user_head != patch.base {
        if let git::Integration::Conflict(files) = git::integrate(wt, &patch.target)? {
            if !resolve_in(job, &task, wt, &patch.target, &files, &ctx, &bill).await? {
                return Ok(PatchLanding::Waiting(format!(
                    "your new commits on `{}` conflict with the patch in {}, and the \
                     conflict could not be resolved automatically",
                    patch.target,
                    files.join(", ")
                )));
            }
            match sign_off(
                job,
                &task,
                wt,
                &ctx,
                &user_head,
                "(conflict resolution)",
                &mut gate,
            )
            .await?
            {
                SignOff::Passed => {}
                SignOff::Failed(f) => {
                    return Ok(PatchLanding::Waiting(format!(
                        "the auditors rejected the conflict resolution:\n{f}"
                    )));
                }
                SignOff::Uncovered => {
                    return Ok(PatchLanding::Waiting(
                        "no auditor covers the files the conflict resolution changed".into(),
                    ));
                }
            }
        }
    }
    if let Err(out) = run_checks(job, wt, &ctx).await {
        return Ok(PatchLanding::Waiting(format!(
            "each task passed on its own, but the combined patch fails its checks — \
             queue a fix task and it lands into this patch:\n{out}"
        )));
    }
    let touched = git::changed_paths(wt, &user_head, "HEAD");
    let dirty = git::dirty_paths(user);
    let clash: Vec<&str> = touched
        .iter()
        .filter(|p| dirty.contains(p))
        .map(String::as_str)
        .collect();
    if !clash.is_empty() {
        return Ok(PatchLanding::Waiting(format!(
            "you have uncommitted changes to files the patch changes: {}. Commit or \
             stash them and say continue.",
            clash.join(", ")
        )));
    }
    let _ = std::fs::remove_dir_all(&scratch);
    if touched.is_empty() {
        let _ = git::remove_worktree(user, wt, &patch.branch);
        return Ok(PatchLanding::Landed(
            "the patch changed nothing; closed it".into(),
        ));
    }
    let n = patch.landed.len();
    let mut title = patch.titles.join("; ");
    if title.chars().count() > 60 {
        title = format!("{}…", title.chars().take(59).collect::<String>());
    }
    let message = format!("ryter: {title}\n\n{n} task(s): {}", patch.landed.join(", "));
    if let Err(e) = git::land(user, &patch.branch, &message) {
        return Ok(PatchLanding::Waiting(format!(
            "the merge failed and was aborted: {e}"
        )));
    }
    let _ = git::remove_worktree(user, wt, &patch.branch);
    Ok(PatchLanding::Landed(format!(
        "landed {n} task(s) on `{}` as one commit ({} files). Undo it all with `git revert -m 1 HEAD`.",
        patch.target,
        touched.len()
    )))
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
/// Run a tool call on a blocking thread. Crew jobs share one async task, so a
/// synchronous shell command or file walk on the executor stalled every other
/// builder until it finished: "parallel" builders ran one tool at a time.
async fn run_tool(
    name: &str,
    args: serde_json::Value,
    ctx: &ToolContext,
) -> Result<crate::tools::ToolOutput> {
    let (name, ctx) = (name.to_string(), ctx.clone());
    tokio::task::spawn_blocking(move || gated_execute(&name, &args, &ctx))
        .await
        .map_err(|e| Error::Config(format!("tool thread: {e}")))?
}

/// Run the configured checks in the worktree, off the executor (they can take
/// minutes). `Err` carries the failing output.
async fn run_checks(
    job: &BuildJob<'_>,
    wt: &Path,
    ctx: &ToolContext,
) -> std::result::Result<String, String> {
    let checks = job.checks.to_vec();
    let wt = wt.to_path_buf();
    let timeout = job.check_timeout;
    let cancel = ctx.cancel.clone();
    tokio::task::spawn_blocking(move || checks_blocking(&checks, &wt, timeout, &cancel))
        .await
        .unwrap_or_else(|e| Err(format!("checks thread: {e}")))
}

fn checks_blocking(
    checks: &[String],
    wt: &Path,
    timeout: std::time::Duration,
    cancel: &Cancel,
) -> std::result::Result<String, String> {
    use crate::tools::shell::{Run, run_command};
    let mut out = String::new();
    for cmd in checks {
        let run = run_command(cmd, wt, timeout, cancel).map_err(|e| format!("$ {cmd}\n{e}"))?;
        let (ok, text) = match run {
            Run::Ok(t) => (true, t),
            Run::Failed(t) => (false, t),
            Run::Cancelled => (false, "cancelled".into()),
            Run::TimedOut => (false, format!("timed out after {}s", timeout.as_secs())),
        };
        out.push_str(&format!(
            "$ {cmd}  → {}\n{}\n",
            if ok { "ok" } else { "FAILED" },
            crate::tools::cap_output(text)
        ));
        if !ok {
            return Err(out);
        }
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
async fn audit(
    job: &BuildJob<'_>,
    seat: &Auditor,
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
    let body = if seat.focus.is_empty() {
        body
    } else {
        format!(
            "You sit on an auditor panel with a {focus} focus. Other seats cover \
             general correctness; weigh {focus} most, but fail anything that must \
             not merge.\n\n{body}",
            focus = seat.focus
        )
    };
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
        &task.files,
    );
    if let Some(p) = &job.progress {
        p.say(Role::Auditor, format!("reviewing ({})", seat.model));
    }
    let bill = Bill {
        meter: job.meter,
        task: &task.id,
        connection: &seat.connection,
        progress: job.progress.as_ref(),
    };
    run_specialist(
        seat.provider.as_ref(),
        &seat.model,
        Role::Auditor,
        msgs,
        &audit_ctx,
        &bill,
    )
    .await
}

/// The gate state with the task's cost filled in, for the report.
fn priced(job: &BuildJob<'_>, task: &Task, gate: &Gate) -> Gate {
    Gate {
        cost: job.meter.task(&task.id).label(),
        ..gate.clone()
    }
}

/// The gate refused. While retries remain the attempt is kept, so the next
/// one fixes it in place.
fn failed(
    job: &BuildJob<'_>,
    task: &Task,
    wt: &Path,
    branch: &str,
    findings: &str,
    handback: &str,
    gate: &Gate,
) -> TaskOutcome {
    let retries = task.retries + 1;
    let status = if retries > job.max_retries {
        TaskStatus::Blocked
    } else {
        TaskStatus::Pending
    };
    // A retry fixes this attempt in place, so the worktree stays. Out of
    // retries, the branch stays for the user and only the directory goes.
    if status == TaskStatus::Blocked {
        git::remove_worktree_keep_branch(job.repo, wt);
    }
    let _ = branch;
    let summary = if status == TaskStatus::Blocked {
        format!("rejected {retries} times; blocked")
    } else {
        "rejected; retrying".into()
    };
    let gate = &priced(job, task, gate);
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
    let gate = &priced(job, task, gate);
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
    if !gate.cost.is_empty() {
        s.push_str(&format!("Cost: {}\n", gate.cost));
    }
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
    meter: &Meter,
    connection: &str,
    progress: Option<Progress>,
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
        &task.files,
    );
    let bill = Bill {
        meter,
        task: &task.id,
        connection,
        progress: progress.as_ref(),
    };
    let wrote =
        |q: &crate::queue::TaskQueue| q.tasks.iter().filter(|t| t.by == "architect").count();
    let before = ctx.queue.lock().map(|q| wrote(&q)).unwrap_or(0);
    let text = run_specialist(provider.as_ref(), model, role, msgs, &ctx, &bill).await?;
    let after = ctx.queue.lock().map(|q| wrote(&q)).unwrap_or(0);
    // An architect that wrote no tasks and said nothing produced nothing; do
    // not call that done. (The first live run "finished" exactly like this.)
    if role == Role::Architect && after == before && text.trim().is_empty() {
        return Ok(TaskOutcome {
            id: task.id.clone(),
            status: TaskStatus::Blocked,
            findings: "the architect returned no design and no tasks".into(),
            summary: "architect returned nothing".into(),
            report: format!(
                "### {} — {} (architect returned nothing)\nCost: {}\nNo design, no builder tasks. Retrying unchanged is unlikely to help.\n",
                task.id,
                task.title,
                meter.task(&task.id).label()
            ),
        });
    }
    let body = clip_handback(&text);
    persist_workspace_note(workspace, role, &body);
    Ok(TaskOutcome {
        id: task.id.clone(),
        status: TaskStatus::Done,
        report: format!(
            "### {} — {} ({role})\nCost: {}\n{body}\n",
            task.id,
            task.title,
            meter.task(&task.id).label()
        ),
        findings: body.clone(),
        summary: first_line(&body).unwrap_or_else(|| format!("{} {}", role, task.title)),
    })
}

fn persist_workspace_note(workspace: &Path, role: Role, body: &str) {
    let name = match role {
        Role::Architect => "architect.md",
        Role::Auditor => "audit.md",
        Role::Builder => "build.md",
        _ => return,
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

/// Who a specialist's tokens are charged to, and where its progress goes.
pub struct Bill<'a> {
    /// The crew run's meter.
    pub meter: &'a Meter,
    /// Queue task id.
    pub task: &'a str,
    /// Connection name, for the spend log.
    pub connection: &'a str,
    /// Progress events, when someone is watching.
    pub progress: Option<&'a Progress>,
}

/// Where a specialist's activity is reported.
#[derive(Clone)]
pub struct Progress {
    /// Event sink (TUI, `--json`).
    pub sink: std::sync::mpsc::Sender<AgentEvent>,
    /// The specialist's id on the crew card.
    pub id: SubagentId,
}

impl Progress {
    fn say(&self, role: Role, text: impl Into<String>) {
        let _ = self.sink.send(AgentEvent::SubagentActivity {
            id: self.id.clone(),
            role,
            text: text.into(),
        });
    }
}

/// Tool rounds and output ceiling per role. A builder writing a whole file puts
/// it in its tool arguments, so it needs room; an auditor reviewing one diff
/// with the checks already run does not need forty rounds.
fn limits(role: Role) -> (usize, u32) {
    // Output ceilings, not budgets: only what is generated is billed. Models
    // that reason spend output tokens before they answer, and on the first
    // live run an architect used its whole 8k on that and returned nothing.
    // The per-task caps are what bound spend.
    match role {
        Role::Builder => (40, 32_768),
        Role::Architect => (30, 32_768),
        _ => (12, 16_384),
    }
}

/// A reply cut off at the output limit this many times in a row ends the task.
const MAX_TRUNCATIONS: usize = 3;

async fn run_specialist(
    provider: &dyn Provider,
    model: &str,
    role: Role,
    mut messages: Vec<crate::llm::Message>,
    ctx: &ToolContext,
    bill: &Bill<'_>,
) -> Result<String> {
    let (rounds, max_tokens) = limits(role);
    let mut last = String::new();
    let mut cutoffs = 0usize;
    for _ in 0..rounds {
        if ctx.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let req = CompletionRequest {
            model: model.to_string(),
            system: None,
            messages: messages.clone(),
            tools: crate::tools::specs_for_opts(role, ctx.web),
            max_tokens: Some(max_tokens),
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
        let mut usage = crate::spend::Usage::default();
        let mut reported: Option<f64> = None;
        let mut truncated = false;
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
                StreamDelta::Usage(u) => usage = u,
                StreamDelta::ReportedCost(c) => reported = Some(c),
                StreamDelta::Truncated => truncated = true,
                _ => {}
            }
        }
        // Charged every round, so a cap stops a runaway loop mid-task rather
        // than after it has spent the money.
        bill.meter
            .charge(bill.task, role, bill.connection, model, usage, reported)?;
        last = text.clone();
        let mut calls = calls.finish();
        if truncated {
            // Cut off mid-reply. A call whose arguments are not complete JSON
            // would run with nothing; drop it and ask for smaller steps rather
            // than ending the task with an empty result.
            cutoffs += 1;
            let before = calls.len();
            calls.retain(|c| {
                serde_json::from_str::<serde_json::Value>(&c.arguments).is_ok_and(|v| v.is_object())
            });
            let dropped = before - calls.len();
            if let Some(p) = bill.progress {
                p.say(
                    role,
                    "reply cut off at the output limit; continuing in smaller steps",
                );
            }
            if cutoffs >= MAX_TRUNCATIONS {
                return Err(Error::TaskBudget(format!(
                    "{role} was cut off at the output limit {cutoffs} times in a row"
                )));
            }
            messages.push(crate::llm::Message {
                role: "assistant".into(),
                content: text,
                tool_call_id: None,
                tool_calls: (!calls.is_empty()).then(|| calls.clone()),
            });
            for call in &calls {
                let parsed: serde_json::Value =
                    serde_json::from_str(&call.arguments).unwrap_or(serde_json::Value::Null);
                let out = run_tool(&call.name, parsed, ctx).await?;
                messages.push(crate::llm::Message {
                    role: "tool".into(),
                    content: out.text,
                    tool_call_id: Some(call.id.clone()),
                    tool_calls: None,
                });
            }
            messages.push(crate::llm::Message {
                role: "user".into(),
                content: format!(
                    "Your last reply was cut off at the output limit{}. Continue from \
                     where you stopped, in smaller steps: think less before acting, \
                     write large files in parts (one file per call, or several \
                     search_replace edits), and keep each reply short.",
                    if dropped > 0 {
                        format!(", and {dropped} unfinished tool call(s) were discarded")
                    } else {
                        String::new()
                    }
                ),
                tool_call_id: None,
                tool_calls: None,
            });
            continue;
        }
        cutoffs = 0;
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
            if let Some(p) = bill.progress {
                p.say(role, crate::agent::tool_summary(&call.name, &parsed));
            }
            let out = run_tool(&call.name, parsed, ctx).await?;
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
            by: String::new(),
            role: "builder".into(),
            hold: false,
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

    fn seat(p: &Arc<Scripted>, model: &str, focus: &str, paths: &[&str]) -> Auditor {
        Auditor {
            provider: p.clone(),
            model: model.into(),
            connection: "c".into(),
            focus: focus.into(),
            paths: paths.iter().map(|s| s.to_string()).collect(),
        }
    }

    async fn run(
        f: &Fixture,
        p: &Arc<Scripted>,
        t: &Task,
        auditor_enabled: bool,
        checks: &[String],
    ) -> TaskOutcome {
        let panel = vec![seat(p, "auditor-m", "", &[])];
        let meter = Meter::new(
            crate::spend::PriceBook::new(),
            crate::meter::Caps::default(),
        );
        run_with(f, p, t, auditor_enabled, checks, &panel, &meter, 0).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_with(
        f: &Fixture,
        p: &Arc<Scripted>,
        t: &Task,
        auditor_enabled: bool,
        checks: &[String],
        panel: &[Auditor],
        meter: &Meter,
        max_retries: u32,
    ) -> TaskOutcome {
        let job = BuildJob {
            provider: p.clone(),
            model: "m",
            connection: "c",
            auditors: panel,
            meter,
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
            max_retries,
            hooks: None,
            cancel: Cancel::new(),
            progress: None,
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
            &Meter::new(
                crate::spend::PriceBook::new(),
                crate::meter::Caps::default(),
            ),
            "c",
            None,
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
    fn say_with_usage(text: &str, input: u64, output: u64) -> Vec<StreamDelta> {
        vec![
            StreamDelta::Text(text.into()),
            StreamDelta::Usage(crate::spend::Usage {
                input_tokens: input,
                output_tokens: output,
                cached_tokens: 0,
            }),
            StreamDelta::ReportedCost(0.01),
            StreamDelta::Done,
        ]
    }

    /// Crew spend used to be invisible: every specialist round is now priced
    /// and attributed to its task and role.
    #[tokio::test]
    async fn every_specialist_round_is_metered() {
        let f = fixture();
        let p = Scripted::new(vec![
            write_call("extra.txt", "x\n"),
            say_with_usage("STATUS: DONE", 1_000, 50),
            say_with_usage("VERDICT: PASS", 400, 10),
        ]);
        let panel = vec![seat(&p, "auditor-m", "", &[])];
        let meter = Meter::new(
            crate::spend::PriceBook::new(),
            crate::meter::Caps::default(),
        );
        let out = run_with(&f, &p, &task("t1", "x"), true, &[], &panel, &meter, 0).await;
        assert_eq!(out.status, TaskStatus::Done, "{out:?}");
        let t = meter.task("t1");
        assert_eq!(t.billable_tokens, 1_460);
        assert!((t.usd - 0.02).abs() < 1e-9, "{t:?}");
        assert!(meter.by_role().contains_key("auditor"));
        // The first round reported no price, so the total is a lower bound
        // rather than a claim.
        assert!(out.report.contains("Cost: ≥$0.02"), "{}", out.report);
    }

    /// A runaway task stops at its cap and keeps what it produced.
    #[tokio::test]
    async fn a_task_cap_stops_the_task_and_keeps_its_branch() {
        let f = fixture();
        let p = Scripted::new(vec![
            write_call("extra.txt", "x\n"),
            say_with_usage("STATUS: DONE", 5_000, 50),
            say("VERDICT: PASS"),
        ]);
        let panel = vec![seat(&p, "auditor-m", "", &[])];
        let caps = crate::meter::Caps {
            task_tokens: 1_000,
            ..crate::meter::Caps::default()
        };
        let meter = Meter::new(crate::spend::PriceBook::new(), caps);
        let out = run_with(&f, &p, &task("t1", "x"), true, &[], &panel, &meter, 0).await;
        assert_eq!(out.status, TaskStatus::Blocked);
        assert!(out.summary.contains("token cap"), "{out:?}");
        assert!(!f.repo.path().join("extra.txt").exists());
        let b = git::git(f.repo.path(), &["branch", "--list", "ryter-*"]).unwrap();
        assert!(!b.trim().is_empty(), "work kept for the user");
    }

    /// A rejected attempt is fixed in place: the retry reopens the same
    /// worktree instead of paying to rebuild the task from scratch.
    #[tokio::test]
    async fn a_retry_fixes_the_rejected_attempt_in_place() {
        let f = fixture();
        let p = Scripted::new(vec![
            write_call("feature.txt", "half done\n"),
            say("STATUS: PARTIAL"),
            say("VERDICT: FAIL\n- feature.txt: unfinished"),
            // Retry: one targeted edit, not a rebuild.
            write_call("feature.txt", "done\n"),
            say("STATUS: DONE"),
            say("VERDICT: PASS"),
        ]);
        let panel = vec![seat(&p, "auditor-m", "", &[])];
        let meter = Meter::new(
            crate::spend::PriceBook::new(),
            crate::meter::Caps::default(),
        );
        let mut t = task("t1", "feature");
        let first = run_with(&f, &p, &t, true, &[], &panel, &meter, 1).await;
        assert_eq!(first.status, TaskStatus::Pending, "{first:?}");
        t.retries = 1;
        t.findings = first.findings;
        let second = run_with(&f, &p, &t, true, &[], &panel, &meter, 1).await;
        assert_eq!(second.status, TaskStatus::Done, "{second:?}");
        let retry_brief = p.request(3);
        assert!(
            retry_brief.contains("already committed in this worktree"),
            "{retry_brief}"
        );
        assert!(retry_brief.contains("feature.txt: unfinished"));
        assert_eq!(
            std::fs::read_to_string(f.repo.path().join("feature.txt")).unwrap(),
            "done\n"
        );
    }

    /// Every covering seat must pass; the first FAIL ends the review, so the
    /// seats after it are never paid for.
    #[tokio::test]
    async fn the_panel_stops_at_the_first_fail() {
        let f = fixture();
        let p = Scripted::new(vec![
            write_call("extra.txt", "x\n"),
            say("done"),
            say("VERDICT: FAIL\n- extra.txt: wrong"),
            say("VERDICT: PASS"),
        ]);
        let panel = vec![
            seat(&p, "cheap-auditor", "", &[]),
            seat(&p, "security-auditor", "security", &[]),
        ];
        let meter = Meter::new(
            crate::spend::PriceBook::new(),
            crate::meter::Caps::default(),
        );
        let out = run_with(&f, &p, &task("t1", "x"), true, &[], &panel, &meter, 0).await;
        assert_eq!(out.status, TaskStatus::Blocked);
        assert_eq!(p.calls(), 3, "the second seat must not run after a FAIL");
        assert!(out.findings.contains("cheap-auditor"));
    }

    /// A path-scoped seat only reviews changes that touch its paths.
    #[tokio::test]
    async fn a_path_scoped_seat_skips_unrelated_changes() {
        let f = fixture();
        let p = Scripted::new(vec![
            write_call("docs/guide.md", "words\n"),
            say("done"),
            say("VERDICT: PASS"),
        ]);
        let panel = vec![
            seat(&p, "general", "", &[]),
            seat(&p, "security", "security", &["src/auth/**"]),
        ];
        let meter = Meter::new(
            crate::spend::PriceBook::new(),
            crate::meter::Caps::default(),
        );
        let out = run_with(&f, &p, &task("t1", "docs"), true, &[], &panel, &meter, 0).await;
        assert_eq!(out.status, TaskStatus::Done, "{out:?}");
        assert_eq!(p.calls(), 3, "only the general seat reviewed a docs change");

        let f = fixture();
        let p = Scripted::new(vec![
            write_call("src/auth/login.rs", "fn login() {}\n"),
            say("done"),
            say("VERDICT: PASS"),
            say("VERDICT: FAIL\n- src/auth/login.rs: no rate limit"),
        ]);
        let panel = vec![
            seat(&p, "general", "", &[]),
            seat(&p, "security", "security", &["src/auth/**"]),
        ];
        let out = run_with(&f, &p, &task("t2", "login"), true, &[], &panel, &meter, 0).await;
        assert_eq!(out.status, TaskStatus::Blocked);
        assert!(p.request(3).contains("security focus"));
    }

    /// Live run 1: a reply cut off at the output limit ended the task with
    /// nothing. Now the broken call is dropped, the model is asked to continue
    /// in smaller steps, and the task still lands.
    #[tokio::test]
    async fn a_cut_off_reply_is_resumed_not_lost() {
        let f = fixture();
        let p = Scripted::new(vec![
            // Cut off mid tool call: the arguments are not complete JSON.
            vec![
                StreamDelta::ToolCall {
                    id: "w".into(),
                    name: "write".into(),
                    arguments: r#"{"path": "extra.txt", "content": "hal"#.into(),
                },
                StreamDelta::Truncated,
                StreamDelta::Done,
            ],
            write_call("extra.txt", "whole\n"),
            say("STATUS: DONE"),
            say("VERDICT: PASS"),
        ]);
        let out = run(&f, &p, &task("t1", "x"), true, &[]).await;
        assert_eq!(out.status, TaskStatus::Done, "{out:?}");
        assert!(
            p.request(1).contains("cut off at the output limit"),
            "{}",
            p.request(1)
        );
        assert_eq!(
            std::fs::read_to_string(f.repo.path().join("extra.txt")).unwrap(),
            "whole\n"
        );
    }

    /// An architect that writes nothing and says nothing did not finish.
    #[tokio::test]
    async fn an_empty_architect_result_is_blocked_not_done() {
        let ws = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        let queue = Arc::new(Mutex::new(crate::queue::TaskQueue::open(
            home.path().join("tasks.json"),
        )));
        let out = run_note_task(
            Arc::new(ReplayProvider::scripted(vec![say("")])),
            ws.path(),
            home.path(),
            &task("a1", "design"),
            "m",
            Role::Architect,
            true,
            false,
            Some(ws.path()),
            false,
            None,
            home.path().join("notes"),
            "",
            Cancel::new(),
            queue,
            &Meter::new(
                crate::spend::PriceBook::new(),
                crate::meter::Caps::default(),
            ),
            "c",
            None,
        )
        .await
        .unwrap();
        assert_eq!(out.status, TaskStatus::Blocked, "{out:?}");
        assert!(out.report.contains("returned nothing"));
    }

    /// Specialists report what they are doing, for the crew card.
    #[tokio::test]
    async fn specialists_report_their_activity() {
        let f = fixture();
        let p = Scripted::new(vec![
            write_call("extra.txt", "x\n"),
            say("STATUS: DONE"),
            say("VERDICT: PASS"),
        ]);
        let (tx, rx) = std::sync::mpsc::channel();
        let panel = vec![seat(&p, "auditor-m", "", &[])];
        let meter = Meter::new(
            crate::spend::PriceBook::new(),
            crate::meter::Caps::default(),
        );
        let job = BuildJob {
            provider: p.clone(),
            model: "m",
            connection: "c",
            auditors: &panel,
            meter: &meter,
            repo: f.repo.path(),
            home: f.home.path(),
            session_id: "sess0001",
            project_root: None,
            trusted: false,
            always_approve: true,
            web: false,
            auditor_enabled: true,
            checks: &[],
            check_timeout: std::time::Duration::from_secs(30),
            max_retries: 0,
            hooks: None,
            cancel: Cancel::new(),
            progress: Some(Progress {
                sink: tx,
                id: crate::queue::new_sub_id(),
            }),
        };
        run_build_task(&job, &task("t1", "x")).await.unwrap();
        let said: Vec<String> = rx
            .try_iter()
            .filter_map(|e| match e {
                AgentEvent::SubagentActivity { text, .. } => Some(text),
                _ => None,
            })
            .collect();
        assert!(
            said.iter().any(|t| t.contains("edit extra.txt")),
            "{said:?}"
        );
        assert!(said.iter().any(|t| t.contains("reviewing")), "{said:?}");
    }

    /// Live run 2 committed an auditor's probe test and __pycache__ files.
    /// Neither may reach the branch.
    #[tokio::test]
    async fn audit_scratch_and_caches_never_reach_the_branch() {
        let f = fixture();
        let p = Scripted::new(vec![
            write_call("feature.py", "X = 1\n"),
            say("STATUS: DONE"),
            // The auditor leaves a probe and a cache behind, then fails it…
            vec![
                StreamDelta::ToolCall {
                    id: "probe".into(),
                    name: "bash".into(),
                    arguments: serde_json::json!({
                        // As the live auditor did: a redirect it is allowed,
                        // and a compile that leaves a cache behind.
                        "command": "printf 'X = 3\\n' > probe_test.py && python3 -m py_compile probe_test.py"
                    })
                    .to_string(),
                },
                StreamDelta::Done,
            ],
            say("VERDICT: FAIL\n- feature.py: X must be 2"),
            // …so the builder retries in place, and commits.
            write_call("feature.py", "X = 2\n"),
            say("STATUS: DONE"),
            say("VERDICT: PASS"),
        ]);
        let panel = vec![seat(&p, "auditor-m", "", &[])];
        let meter = Meter::new(
            crate::spend::PriceBook::new(),
            crate::meter::Caps::default(),
        );
        let mut t = task("t1", "feature");
        let first = run_with(&f, &p, &t, true, &[], &panel, &meter, 1).await;
        assert_eq!(first.status, TaskStatus::Pending, "{first:?}");
        t.retries = 1;
        t.findings = first.findings;
        let second = run_with(&f, &p, &t, true, &[], &panel, &meter, 1).await;
        assert_eq!(second.status, TaskStatus::Done, "{second:?}");
        let tracked = git::git(f.repo.path(), &["ls-files"]).unwrap();
        assert!(
            !tracked.contains("probe_test.py"),
            "audit probe committed:\n{tracked}"
        );
        assert!(
            !tracked.contains("__pycache__"),
            "cache committed:\n{tracked}"
        );
        assert!(tracked.contains("feature.py"));
    }

    #[test]
    fn same_model_sees_through_routes() {
        assert!(same_model("x-ai/grok-4.6", "grok-4.6"));
        assert!(same_model("grok-4.6-latest", "grok-4.6"));
        assert!(same_model(
            "Anthropic/Claude-Sonnet-4.6",
            "claude-sonnet-4.6"
        ));
        assert!(!same_model("grok-4.6", "grok-4.5"));
    }

    /// Sign-off must come from a model that neither directs nor wrote the work.
    #[test]
    fn auditors_must_differ_from_lead_and_builder() {
        let p = Scripted::new(vec![]);
        let ok = vec![seat(&p, "claude-sonnet-4.6", "", &[])];
        assert_eq!(independence_problem("grok-4.6", "grok-4.6", &ok), None);
        let same_as_lead = vec![seat(&p, "x-ai/grok-4.6", "", &[])];
        let why = independence_problem("grok-4.6", "deepseek-v4", &same_as_lead).unwrap();
        assert!(why.contains("same model as the lead"), "{why}");
        let same_as_builder = vec![seat(&p, "deepseek-v4", "", &[])];
        let why = independence_problem("grok-4.6", "deepseek-v4", &same_as_builder).unwrap();
        assert!(why.contains("same model as the builder"), "{why}");
        // Any seat, not only the first.
        let mixed = vec![
            seat(&p, "claude-sonnet-4.6", "", &[]),
            seat(&p, "grok-4.6", "security", &[]),
        ];
        assert!(independence_problem("grok-4.6", "grok-4.6", &mixed).is_some());
        // Every change needs a reviewer: an all-scoped panel is refused.
        let scoped = vec![seat(&p, "claude-sonnet-4.6", "security", &["src/**"])];
        assert!(independence_problem("grok-4.6", "grok-4.6", &scoped).is_some());
    }
}
