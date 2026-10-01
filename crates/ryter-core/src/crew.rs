//! Spawn specialists from the task queue: worktrees, auditor, auto-merge.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::cancel::Cancel;
use crate::error::{Error, Result};
use crate::event::{AgentEvent, LivePhase};
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
    /// Landed on review alone: the auditor could not build or test it,
    /// because something outside the task had not landed yet. The patch
    /// must build and test it before it reaches the user's branch.
    pub unverified: bool,
    /// The builder's work is committed and waits only on the gate; the next
    /// run goes straight there.
    pub gate_next: bool,
    /// The builder's handback for that work.
    pub handback: String,
    /// The checks or the audit rejected the work this attempt.
    pub rejected: bool,
}

/// An auditor's decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Merge it.
    Pass,
    /// Fix it first.
    Fail,
    /// The code reads right, but it can't be built or tested until
    /// something outside the task lands. Not a rejection: no retry is spent.
    Unverified,
}

/// Parse the auditor's verdict.
///
/// The contract (`prompts/auditor.md`) is a `VERDICT: PASS` or `VERDICT: FAIL`
/// line; the last one wins, since reviewers often restate the criteria before
/// deciding. A bare `PASS`/`FAIL` first line is still accepted. Anything else
/// is a fail: an unparseable review must not merge.
pub fn parse_verdict(text: &str) -> bool {
    verdict(text) == Verdict::Pass
}

/// The auditor's verdict: the last `VERDICT:` line wins; a bare first-line
/// `PASS` still counts; anything else is a fail.
pub fn verdict(text: &str) -> Verdict {
    stated_verdict(text).unwrap_or(Verdict::Fail)
}

/// The verdict the auditor gave, or `None` when it gave none: it ran out of
/// steps, or stopped at "I'll now run the tests". No verdict must not merge
/// ([`verdict`] reads it as a fail), and it isn't a finding against the
/// work either: nothing was decided.
pub fn stated_verdict(text: &str) -> Option<Verdict> {
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
                verdict = Some(Verdict::Pass);
            } else if rest.starts_with("FAIL") {
                verdict = Some(Verdict::Fail);
            } else if rest.starts_with("UNVERIFIED") {
                verdict = Some(Verdict::Unverified);
            }
        }
    }
    if verdict.is_some() {
        return verdict;
    }
    // A bare first line still counts.
    let first = text.lines().map(clean).find(|l| !l.is_empty())?;
    if first.starts_with("PASS") {
        return Some(Verdict::Pass);
    }
    if first.starts_with("FAIL") {
        return Some(Verdict::Fail);
    }
    None
}

/// How a specialist's last message has to end. A run that stops short of it
/// is asked for it once, with no tools, before the message is taken as it
/// is: an auditor that ends on "I'll run the checks now" has decided
/// nothing, and a builder cut off mid-sentence has handed nothing back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Closing {
    /// A builder's handback: a `STATUS:` line.
    Handback,
    /// An auditor's review: a `VERDICT:` line.
    Verdict,
}

impl Closing {
    /// The message can be taken as it is. Only a review is held to its
    /// line: without a verdict nothing can be done with it. A handback
    /// without its `STATUS:` line still says what the builder did, and the
    /// checks and the audit judge the work itself.
    fn met(self, text: &str) -> bool {
        match self {
            Self::Verdict => stated_verdict(text).is_some(),
            Self::Handback => true,
        }
    }

    /// What to write, said to a specialist that is being stopped.
    fn shape(self) -> &'static str {
        match self {
            Self::Verdict => {
                "List your findings and end with your verdict line. If you could not confirm \
                 the work by running it, say what you did and did not confirm, and give \
                 `VERDICT: UNVERIFIED` if that is why you can't pass it. A review with no \
                 verdict line decides nothing."
            }
            Self::Handback => {
                "Use the handback shape. If the task isn't finished, hand back \
                 `STATUS: PARTIAL` and say exactly what is done and what is left."
            }
        }
    }

    fn missing(self) -> &'static str {
        match self {
            Self::Verdict => "no verdict yet: asking for one",
            Self::Handback => "no handback yet: asking for one",
        }
    }
}

/// The builder's `STATUS: BLOCKED` handback: it hit something outside its
/// task (a missing system package, a file another task owns that isn't
/// there). Checked and audited anyway, it only spent a retry; one builder
/// spent 80 calls working around a missing sound library, `sudo` included.
/// The line of a blocked handback that says what would unblock it, for the
/// lead and for every task waiting on this one. Builders write NOTES as a
/// list; the fix (`sudo dnf install libpq-devel`) is the line that matters.
fn blocked_reason(handback: &str) -> Option<String> {
    let clean = |l: &str| {
        l.trim()
            .trim_start_matches(['-', '*', ' '])
            .trim()
            .to_string()
    };
    let mut notes: Vec<String> = Vec::new();
    let mut in_notes = false;
    for line in handback.lines() {
        let t = line.trim().trim_matches('*');
        if let Some(rest) = t.strip_prefix("NOTES:") {
            in_notes = true;
            let rest = clean(rest.trim_start_matches('*'));
            if !rest.is_empty() {
                notes.push(rest);
            }
            continue;
        }
        if in_notes {
            if t.starts_with("```") {
                break;
            }
            let l = clean(t);
            if !l.is_empty() {
                notes.push(l);
            }
        }
    }
    let wants = ["unblock", "sudo ", "install", "missing", "not installed"];
    let pick = notes
        .iter()
        .find(|l| {
            let low = l.to_ascii_lowercase();
            wants.iter().any(|w| low.contains(w))
        })
        .or(notes.first())?;
    let mut r: String = pick.chars().take(200).collect();
    if pick.chars().count() > 200 {
        r.push('…');
    }
    Some(r)
}

fn builder_blocked(handback: &str) -> bool {
    handback.lines().any(|l| {
        let l = l
            .trim()
            .trim_matches(|c: char| matches!(c, '*' | '`' | '#' | ' '))
            .to_ascii_uppercase();
        l.strip_prefix("STATUS")
            .map(|r| r.trim_start_matches([':', ' ', '*']))
            .is_some_and(|r| r.starts_with("BLOCKED"))
    })
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
    /// Who to ask when the task reaches its cap; `None` stops it there.
    pub ask: Option<&'a CapAsk>,
    /// Times the checks or the audit rejected this task before this attempt,
    /// in all (the lead requeues and recreates tasks).
    pub rejections: u32,
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
    // Already over its cap (a retry of a task that stopped there): a step
    // could only be paid for and stop again. Asked, the user can raise it.
    let cap = job.meter.task_cap(&task.id);
    if cap > 0.0 && job.meter.task(&task.id).usd >= cap {
        let raised = match job.ask {
            Some(ask) => raise_cap(job.meter, &task.id, ask).await,
            None => false,
        };
        if !raised {
            let mut o = outcome(
                task,
                TaskStatus::Blocked,
                &format!(
                    "still at its ${cap:.2} cap ({}); raising the cap lets it go on",
                    job.meter.task(&task.id).label()
                ),
                "",
                "",
            );
            o.gate_next = task.gate_next;
            o.handback = task.handback.clone();
            return Ok(o);
        }
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
    // The builder's handback, once its work is committed.
    let built = std::sync::Mutex::new(None::<String>);
    let result = build_inner(job, task, &onto, &wt, &branch, &scratch, resumed, &built).await;
    let _ = std::fs::remove_dir_all(&scratch);
    let built = built.into_inner().ok().flatten();
    // Stopped at the gate, not rejected: the next run goes back to the gate.
    let at_gate = |mut o: TaskOutcome| {
        if let Some(h) = &built {
            if o.status == TaskStatus::Blocked && !o.rejected {
                o.gate_next = true;
                o.handback.clone_from(h);
            }
        }
        o
    };
    match result {
        Ok(o) => Ok(at_gate(o)),
        // A cap stopped the work partway: keep what it produced. Recreating
        // the task would start the build over; raising its cap goes on.
        Err(Error::TaskBudget(why)) => {
            let gate = Gate {
                cost: job.meter.task(&task.id).label(),
                ..Gate::default()
            };
            let why = format!(
                "{why}. Raising the task's cap and setting it to pending goes on from here; \
                 recreating the task starts the build over"
            );
            Ok(at_gate(keep_branch(
                job, task, &wt, &branch, why, "", &gate,
            )))
        }
        Err(e @ Error::Budget { .. }) => {
            git::remove_worktree_keep_branch(job.repo, &wt);
            Err(e)
        }
        // A failure after the builder committed (a provider refused the
        // conflict resolver) is no verdict on that work, and it was paid for.
        Err(e) if !matches!(e, Error::Cancelled) && has_work(job.repo, &onto, &branch) => {
            let why = match &e {
                Error::Provider(raw) => {
                    crate::llm::explain_error(raw).unwrap_or_else(|| e.to_string())
                }
                other => other.to_string(),
            };
            let gate = Gate {
                cost: job.meter.task(&task.id).label(),
                ..Gate::default()
            };
            Ok(at_gate(keep_branch(
                job,
                task,
                &wt,
                &branch,
                format!("stopped after the build: {why}"),
                "",
                &gate,
            )))
        }
        Err(e) => {
            let _ = git::remove_worktree(job.repo, &wt, &branch);
            Err(e)
        }
    }
}

/// `branch` changes something since it left `onto`.
fn has_work(repo: &Path, onto: &str, branch: &str) -> bool {
    git::git(repo, &["merge-base", onto, branch])
        .is_ok_and(|base| !git::changed_paths(repo, base.trim(), branch).is_empty())
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
    /// Signed off on review alone (`VERDICT: UNVERIFIED`).
    unverified: bool,
}

#[allow(clippy::too_many_arguments)]
async fn build_inner(
    job: &BuildJob<'_>,
    task: &Task,
    onto: &str,
    wt: &Path,
    branch: &str,
    scratch: &Path,
    resumed: bool,
    built: &std::sync::Mutex<Option<String>>,
) -> Result<TaskOutcome> {
    let bill = Bill {
        meter: job.meter,
        task: &task.id,
        connection: job.connection,
        progress: job.progress.as_ref(),
        wrap_up_usd: None,
        last_text: None,
        ask: job.ask,
        closing: None,
    };
    let ctx = ToolContext {
        live: None,
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
        allowed: Default::default(),
        web: job.web,
    };

    // A run that stopped at the gate goes back to it: the builder's work is
    // committed, and building it again would pay for it twice.
    let handback = if resumed && task.gate_next {
        if let Some(p) = &job.progress {
            p.say(
                Role::Builder,
                "built on an earlier run: straight to the gate",
            );
        }
        task.handback.clone()
    } else {
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
            &Bill {
                closing: Some(Closing::Handback),
                ..bill
            },
        )
        .await?;
        git::commit_all(wt, &format!("ryter: {}", task.title))?;
        if builder_blocked(&handback) {
            let gate = Gate::default();
            let why = match blocked_reason(&handback) {
                Some(r) => format!("the builder is blocked: {r}"),
                None => "the builder is blocked; its handback says why".into(),
            };
            let mut o = keep_branch(job, task, wt, branch, why, &handback, &gate);
            o.summary = "blocked; needs the lead or the user".into();
            return Ok(o);
        }
        handback
    };
    if let Ok(mut b) = built.lock() {
        *b = Some(handback.clone());
    }

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
                SignOff::Unverified => {
                    gate.signed_off = true;
                    gate.unverified = true;
                }
                SignOff::Failed(findings) => {
                    return Ok(failed(job, task, wt, branch, &findings, &handback, &gate));
                }
                // Neither is a finding against the work: it stays at the
                // gate, no retry is spent, and the builder isn't run again.
                SignOff::Unreachable(why) | SignOff::NoVerdict(why) => {
                    return Ok(keep_branch(job, task, wt, branch, why, &handback, &gate));
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
        let status = if gate.unverified {
            "merged on review alone; the patch builds and tests it"
        } else {
            "merged"
        };
        return Ok(TaskOutcome {
            id: task.id.clone(),
            status: TaskStatus::Done,
            findings: gate.audit.clone(),
            report: report(task, status, &summary, &handback, &gate),
            summary,
            unverified: gate.unverified,
            gate_next: false,
            handback: String::new(),
            rejected: false,
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
    /// Every seat passed or couldn't verify, and at least one couldn't: no
    /// checks ran, and the code can't be built until another task lands.
    Unverified,
    /// A seat failed it; its findings.
    Failed(String),
    /// No seat covers the changed paths, so nobody can sign off.
    Uncovered,
    /// A seat's provider refused or failed the call: no verdict on the work,
    /// which stays. Why, for a person.
    Unreachable(String),
    /// A seat ended without a verdict, even when asked for one: nothing was
    /// decided about the work, which stays. Why, for a person.
    NoVerdict(String),
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
    let mut unverified = false;
    for seat in job.auditors.iter().filter(|a| a.applies_to(&changed)) {
        let text = audit(job, seat, task, wt, ctx, target, handback, gate).await;
        // Reviewers probe: they write scratch tests, run them, sometimes leave
        // them. The builder's work is committed, so reset to it. On the first
        // live crew run an audit probe was swept into the next commit.
        git::discard_uncommitted(wt);
        let text = match text {
            Ok(t) => t,
            // An auditor the account cannot use (OpenRouter under zero data
            // retention) used to fail the task and delete the builder's
            // paid-for branch.
            Err(Error::Provider(e)) => {
                return Ok(SignOff::Unreachable(format!(
                    "the auditor ({}) could not run: {}",
                    seat.model,
                    crate::llm::explain_error(&e).unwrap_or_else(|| e
                        .lines()
                        .next()
                        .unwrap_or(&e)
                        .to_string())
                )));
            }
            Err(e) => return Err(e),
        };
        let lens = if seat.focus.is_empty() {
            "review"
        } else {
            seat.focus.as_str()
        };
        reviews.push(format!("[{} · {lens}]\n{}", seat.model, text.trim()));
        gate.audit = reviews.join("\n\n");
        // No verdict is not a FAIL. It used to count as one: the builder was
        // run again on work nobody had faulted, the round was added to its
        // rejections, and the user was told to choose a stronger builder.
        let Some(v) = stated_verdict(&text) else {
            return Ok(SignOff::NoVerdict(format!(
                "the auditor ({}) ended without a verdict, even when asked for one, so \
                 nothing was decided about this work. It was not rejected, and the builder \
                 was not run again. Setting the task to pending audits it again without \
                 rebuilding it. If the same thing happens, the audit needs more steps than \
                 it has, or the user needs to choose another auditor in /models → auditor",
                seat.model
            )));
        };
        // Checks that ran and passed on this tree verified it already.
        if v == Verdict::Unverified && gate.checks.trim().is_empty() {
            unverified = true;
        }
        let pass = v != Verdict::Fail;
        if !pass {
            return Ok(SignOff::Failed(gate.audit.clone()));
        }
    }
    Ok(if reviews.is_empty() {
        SignOff::Uncovered
    } else if unverified {
        SignOff::Unverified
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
                     must come from a different model: choose one in /models → auditor, \
                     set up the crew with /models → `b` (or run `ryter crew suggest \
                     --apply`), or add seats under [[auditor.panel]]. \
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
        after: Vec::new(),
        status: TaskStatus::Running,
        retries: 0,
        findings: String::new(),
        spent: Default::default(),
        cap_usd: None,
        gate_next: false,
        handback: String::new(),
    };
    let ctx = ToolContext {
        live: None,
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
        allowed: Default::default(),
        web: job.web,
    };
    let bill = Bill {
        meter: job.meter,
        task: &task.id,
        connection: job.connection,
        progress: job.progress.as_ref(),
        wrap_up_usd: None,
        last_text: None,
        ask: job.ask,
        closing: None,
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
                // The checks below build and test the whole patch.
                SignOff::Passed | SignOff::Unverified => {}
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
                SignOff::Unreachable(why) | SignOff::NoVerdict(why) => {
                    return Ok(PatchLanding::Waiting(why));
                }
            }
        }
    }
    // Work signed off on review alone lands only on a patch that builds
    // and tests: that was the promise when it was let through.
    if !patch.unverified.is_empty() && job.checks.is_empty() {
        return Ok(PatchLanding::Waiting(format!(
            "{} passed review but were never built or tested, and no checks are \
             set to build and test the patch. Set `checks` under [auditor] in \
             .ryter/config.toml (for example `cargo test`), then continue.",
            patch
                .unverified
                .iter()
                .map(|t| format!("`{t}`"))
                .collect::<Vec<_>>()
                .join(", ")
        )));
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
        "No checks are configured for this project, so nothing has built or tested this code. \
         Run its tests yourself if your shell can: it runs test runners, linters and read-only \
         commands (`pytest`, `cargo test`, `npm test`, `go test`, `ruff check`), in the \
         project's containers too (`docker compose run --rm <service> pytest`, `docker \
         compose exec <service> …`), and refuses everything else, including building, \
         starting and stopping containers, servers, installs and the project's own shell \
         scripts. That is a limit on you, not on the project or the machine: the builder \
         can run those, and its handback says what it ran. If confirming the work needs a \
         command your shell refuses, or something outside this task that hasn't landed (the \
         manifest or module another task creates), don't look for a way round: review it by \
         reading, list what you could not confirm, and end with `VERDICT: UNVERIFIED`. The \
         work then waits on the patch, and reaches the user's branch only once checks have \
         built and tested it."
            .to_string()
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
        live: None,
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
        wrap_up_usd: None,
        last_text: None,
        ask: job.ask,
        closing: Some(Closing::Verdict),
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
    // In all, not this run's: a requeued task gets one more attempt, and the
    // count said "rejected 3 times" after each single one.
    let total = job.rejections + 1;
    let summary = if status == TaskStatus::Blocked {
        format!(
            "rejected; blocked ({total} rejection{} in all)",
            if total == 1 { "" } else { "s" }
        )
    } else {
        format!("rejected; retrying ({total} in all)")
    };
    let gate = &priced(job, task, gate);
    TaskOutcome {
        id: task.id.clone(),
        status,
        findings: findings.to_string(),
        report: report(task, &summary, findings, handback, gate),
        summary,
        unverified: false,
        gate_next: false,
        handback: String::new(),
        rejected: true,
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
        unverified: false,
        gate_next: false,
        handback: String::new(),
        rejected: false,
    }
}

fn outcome(task: &Task, status: TaskStatus, why: &str, handback: &str, audit: &str) -> TaskOutcome {
    TaskOutcome {
        id: task.id.clone(),
        status,
        findings: why.to_string(),
        summary: why.to_string(),
        report: format!("### {} — {}\n{why}\n{handback}{audit}", task.id, task.title),
        unverified: false,
        gate_next: false,
        handback: String::new(),
        rejected: false,
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
    ask: Option<&CapAsk>,
) -> Result<TaskOutcome> {
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    // The session queue, so an architect's task list is what builders run.
    // It used to be a throwaway queue at `<repo>/tasks.json`: the tasks never
    // reached a builder, and the file landed in the user's project.
    let ctx = ToolContext {
        live: None,
        workspace: workspace.to_path_buf(),
        notes_dir,
        role,
        always_approve,
        queue,
        mcp: None,
        hooks,
        cancel,
        user_io: None,
        allowed: Default::default(),
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
        wrap_up_usd: None,
        last_text: None,
        ask,
        closing: None,
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
            unverified: false,
            gate_next: false,
            handback: String::new(),
            rejected: false,
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
        unverified: false,
        gate_next: false,
        handback: String::new(),
        rejected: false,
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
#[derive(Clone, Copy)]
pub struct Bill<'a> {
    /// The crew run's meter.
    pub meter: &'a Meter,
    /// Queue task id.
    pub task: &'a str,
    /// Connection name, for the spend log.
    pub connection: &'a str,
    /// Progress events, when someone is watching.
    pub progress: Option<&'a Progress>,
    /// The most the task may spend, enforced before each step, not after:
    /// a step that would pass it isn't sent, and when about one step's room
    /// is left (or [`WRAP_UP_SHARE`] of it is spent) the specialist is told
    /// to write its answer now, with no more tools. A cap that only stops
    /// after a step has passed it leaves the user paying for nothing.
    pub wrap_up_usd: Option<f64>,
    /// The specialist's latest text, kept as it goes, so a stop at the cap
    /// still has something to show.
    pub last_text: Option<&'a std::sync::Mutex<String>>,
    /// Who to ask when the task reaches its cap; `None` stops it there.
    pub ask: Option<&'a CapAsk>,
    /// How the specialist's last message has to end, when it has to.
    pub closing: Option<Closing>,
}

/// Asks the user whether to raise a task's cap when it reaches it. The task
/// used to stop there with its step paid for and thrown away, and the lead
/// recreated it, starting the build over.
pub struct CapAsk {
    /// Where the question goes.
    pub io: crate::user_io::UserIo,
    /// The turn's stop flag: an unanswered question ends with the turn.
    pub cancel: Arc<Cancel>,
    /// What else to weigh: the rejections so far, a stronger builder.
    pub note: String,
}

/// Dollars a cap can be raised by, most first.
const RAISE_STEPS: [f64; 2] = [5.0, 2.0];

/// `task` is at its dollar cap: ask whether to raise it, and raise it if
/// so. True when raised; the task carries on from where it is.
async fn raise_cap(meter: &Meter, task: &str, ask: &CapAsk) -> bool {
    let cap = meter.task_cap(task);
    if cap <= 0.0 {
        return false;
    }
    let spent = meter.task(task).usd;
    let roles: Vec<String> = meter
        .task_roles(task)
        .iter()
        .map(|(r, usd)| format!("{r} ${usd:.2}"))
        .collect();
    let roles = if roles.is_empty() {
        String::new()
    } else {
        format!(" ({} this run)", roles.join(", "))
    };
    let note = if ask.note.is_empty() {
        String::new()
    } else {
        format!("\n\n{}", ask.note)
    };
    let question = format!(
        "Task {task} has spent ${spent:.2} of its ${cap:.2} cap{roles}.{note}\n\n\
         Raise its cap and carry on from where it is?"
    );
    let options: Vec<String> = RAISE_STEPS
        .iter()
        .map(|step| format!("Add ${step:.0} (cap ${:.2})", cap + step))
        .chain(["Stop here; its branch keeps the work".to_string()])
        .collect();
    let (io, cancel, asked) = (ask.io.clone(), ask.cancel.clone(), options.clone());
    let answer = tokio::task::spawn_blocking(move || {
        io.ask_as(Some("task budget"), &question, asked, &cancel)
    })
    .await
    .unwrap_or_default();
    match RAISE_STEPS
        .iter()
        .zip(&options)
        .find(|(_, o)| **o == answer)
    {
        Some((step, _)) => {
            meter.raise_task_cap(task, cap + step);
            true
        }
        None => false,
    }
}

/// Where a specialist's activity is reported.
#[derive(Clone)]
pub struct Progress {
    /// Event sink (TUI, `--json`), when someone is watching.
    pub sink: Option<std::sync::mpsc::Sender<AgentEvent>>,
    /// The specialist's id on the crew card.
    pub id: SubagentId,
    /// The session's `activity.jsonl`: what each specialist did, kept. A
    /// builder's 80 calls of workarounds used to leave no trace but its bill.
    pub log: Option<std::path::PathBuf>,
}

impl Progress {
    pub(crate) fn say(&self, role: Role, text: impl Into<String>) {
        let text = text.into();
        if let Some(path) = &self.log {
            let _ = crate::session::append_jsonl(
                path,
                &serde_json::json!({
                    "ts": std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0),
                    "id": self.id.as_str(),
                    "role": role.to_string(),
                    "text": text,
                }),
            );
        }
        if let Some(sink) = &self.sink {
            let _ = sink.send(AgentEvent::SubagentActivity {
                id: self.id.clone(),
                role,
                text,
            });
        }
    }

    /// Tell the board what the specialist is doing this moment. Not logged:
    /// it comes several times a second.
    fn live(
        &self,
        role: Role,
        phase: LivePhase,
        target: &str,
        tokens: u64,
        lines: u32,
        tail: Vec<String>,
    ) {
        if let Some(sink) = &self.sink {
            let _ = sink.send(AgentEvent::SubagentLive {
                id: self.id.clone(),
                role,
                phase,
                target: target.to_string(),
                tokens,
                lines,
                tail,
            });
        }
    }
}

/// How often a streaming step is reported, at most. A change of what it is
/// doing is reported at once.
const LIVE_EVERY: std::time::Duration = std::time::Duration::from_millis(200);
/// Lines of what a specialist is producing that the board shows.
const LIVE_TAIL: usize = 3;
/// Bytes of reasoning or reply kept for the tail.
const LIVE_KEEP: usize = 2000;

/// One model step of a specialist, as the board sees it while it streams:
/// waiting for the first byte, thinking, writing a reply or a tool call.
struct LiveStep<'a> {
    progress: Option<&'a Progress>,
    role: Role,
    sent: Option<(std::time::Instant, LivePhase, String)>,
    chars: usize,
    reasoning: String,
    text: String,
}

impl<'a> LiveStep<'a> {
    /// A request went out: waiting until something comes back.
    fn start(progress: Option<&'a Progress>, role: Role) -> Self {
        let mut s = Self {
            progress,
            role,
            sent: None,
            chars: 0,
            reasoning: String::new(),
            text: String::new(),
        };
        s.report(LivePhase::Waiting, String::new(), 0, Vec::new());
        s
    }

    fn reasoning(&mut self, t: &str) {
        self.chars += t.len();
        keep_tail(&mut self.reasoning, t);
        if self.due(LivePhase::Thinking) {
            let tail = tail_lines(&self.reasoning, LIVE_TAIL);
            self.report(LivePhase::Thinking, String::new(), 0, tail);
        }
    }

    fn text(&mut self, t: &str) {
        self.chars += t.len();
        keep_tail(&mut self.text, t);
        if self.due(LivePhase::Writing) {
            let tail = tail_lines(&self.text, LIVE_TAIL);
            self.report(LivePhase::Writing, "its reply".into(), 0, tail);
        }
    }

    /// A tool call is arriving: what it is on, and for a file being
    /// written, the file so far. Read only when a report is due: a long
    /// edit arrives in thousands of fragments.
    fn call(&mut self, fragment: &str, calls: &crate::llm::ToolCallAccumulator) {
        self.chars += fragment.len();
        if !self.due(LivePhase::Writing) {
            return;
        }
        let Some(call) = calls.last() else {
            return;
        };
        let (target, body) = partial_call(&call.name, &call.arguments);
        let lines = body.as_deref().map_or(0, |b| b.lines().count() as u32);
        let tail = body.map(|b| tail_lines(&b, LIVE_TAIL)).unwrap_or_default();
        self.report(LivePhase::Writing, target, lines, tail);
    }

    /// A change of phase is reported at once, anything else at most every
    /// [`LIVE_EVERY`].
    fn due(&self, phase: LivePhase) -> bool {
        self.progress.is_some()
            && self
                .sent
                .as_ref()
                .is_none_or(|(at, ph, _)| *ph != phase || at.elapsed() >= LIVE_EVERY)
    }

    fn report(&mut self, phase: LivePhase, target: String, lines: u32, tail: Vec<String>) {
        let Some(p) = self.progress else {
            return;
        };
        // A rough count, about four characters a token: usage arrives only
        // when the step ends.
        let tokens = (self.chars / 4) as u64;
        p.live(self.role, phase, &target, tokens, lines, tail);
        self.sent = Some((std::time::Instant::now(), phase, target));
    }
}

/// Report a tool about to run, and give it a context whose command output
/// goes to the board as it arrives.
fn running_tool(
    progress: Option<&Progress>,
    role: Role,
    target: &str,
    ctx: &ToolContext,
) -> ToolContext {
    let mut ctx = ctx.clone();
    if let Some(p) = progress {
        p.live(role, LivePhase::Running, target, 0, 0, Vec::new());
        let (p, target) = (p.clone(), target.to_string());
        ctx.live = Some(crate::tools::LiveOutput(std::sync::Arc::new(
            move |tail: &[String]| {
                p.live(role, LivePhase::Running, &target, 0, 0, tail.to_vec());
            },
        )));
    }
    ctx
}

/// Append `t`, keeping about the last [`LIVE_KEEP`] bytes.
fn keep_tail(buf: &mut String, t: &str) {
    buf.push_str(t);
    if buf.len() > 2 * LIVE_KEEP {
        let mut cut = buf.len() - LIVE_KEEP;
        while !buf.is_char_boundary(cut) {
            cut += 1;
        }
        buf.drain(..cut);
    }
}

/// The last `n` non-blank lines of `text`, the line still being written
/// included.
fn tail_lines(text: &str, n: usize) -> Vec<String> {
    let mut lines: Vec<String> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim_end().to_string())
        .collect();
    let from = lines.len().saturating_sub(n);
    lines.split_off(from)
}

/// What a tool call still arriving is on (`edit src/ui.rs`), and for a file
/// being written, its text so far.
fn partial_call(name: &str, args: &str) -> (String, Option<String>) {
    let mut known = serde_json::Map::new();
    for key in [
        "path",
        "target_file",
        "command",
        "pattern",
        "query",
        "url",
        "name",
        "question",
    ] {
        if let Some(v) = partial_str(args, key) {
            known.insert(key.into(), serde_json::Value::String(v));
        }
    }
    let target = crate::agent::tool_summary(name, &serde_json::Value::Object(known));
    let body = match name {
        "write" => partial_str(args, "content"),
        "search_replace" => partial_str(args, "new_string"),
        _ => None,
    };
    (target, body)
}

/// The string value of `key` in JSON still arriving: as much of it as has
/// come, unescaped. `None` until the value has started.
fn partial_str(json: &str, key: &str) -> Option<String> {
    let at = json.find(&format!("\"{key}\""))? + key.len() + 2;
    let rest = json[at..].trim_start().strip_prefix(':')?.trim_start();
    let mut chars = rest.strip_prefix('"')?.chars();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => {}
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        out.push(ch);
                    }
                }
                Some(other) => out.push(other),
                None => break,
            },
            other => out.push(other),
        }
    }
    Some(out)
}

/// Tool rounds and output ceiling per role. A builder writing a whole file puts
/// it in its tool arguments, so it needs room; an auditor reviewing one diff
/// with the checks already run does not need forty rounds. The rounds are
/// the defaults: the user sets their own in `[subagents.steps]`, which the
/// run's meter carries.
fn limits(role: Role) -> (usize, u32) {
    // Output ceilings, not budgets: only what is generated is billed. Models
    // that reason spend output tokens before they answer, and on the first
    // live run an architect used its whole 8k on that and returned nothing.
    // The per-task caps are what bound spend.
    match role {
        Role::Builder | Role::Architect => (crate::config::Steps::default().for_role(role), 32_768),
        _ => (crate::config::Steps::default().for_role(role), 16_384),
    }
}

/// Share of [`Bill::wrap_up_usd`] after which the specialist writes up.
pub const WRAP_UP_SHARE: f64 = 0.75;

/// Output a step is priced at before it is sent.
const STEP_OUTPUT: u64 = 4_000;

/// Tokens a request of `messages` will be billed for, roughly (4 bytes a
/// token), with room for the tool list.
fn request_tokens(messages: &[crate::llm::Message]) -> u64 {
    let bytes: usize = messages
        .iter()
        .map(|m| {
            m.content.len()
                + m.tool_calls
                    .as_ref()
                    .map_or(0, |c| c.iter().map(|c| c.arguments.len() + 32).sum())
        })
        .sum();
    bytes as u64 / 4 + 1_500
}

/// A reply cut off at the output limit this many times in a row ends the task.
const MAX_TRUNCATIONS: usize = 3;

pub(crate) async fn run_specialist(
    provider: &dyn Provider,
    model: &str,
    role: Role,
    mut messages: Vec<crate::llm::Message>,
    ctx: &ToolContext,
    bill: &Bill<'_>,
) -> Result<String> {
    let (_, max_tokens) = limits(role);
    let rounds = bill.meter.steps(role);
    let mut last = String::new();
    let mut cutoffs = 0usize;
    // Why the specialist has been told to stop and write up, once it has.
    let mut wrap: Option<Wrap> = None;
    let mut asked_closing = false;
    // What it had said when it was asked for its verdict or handback.
    let mut before_asking: Option<String> = None;
    // What the run gave back, with a line from Ryter when it was stopped at
    // its step limit: the lead and the user read "it ran out of steps", not
    // a report that trails off.
    let finish = |text: String, wrap: Option<Wrap>| match wrap {
        Some(Wrap::Steps) => format!(
            "{}\n\n[Ryter] The {role} reached its limit of {rounds} steps and was told to \
             write up on the last one.",
            text.trim_end()
        ),
        _ => text,
    };
    for round in 0..rounds {
        if ctx.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let mut live = LiveStep::start(bill.progress, role);
        // A limit is kept before each step: price the step from what will
        // be sent, stop if it doesn't fit, and write up while one still does.
        if let Some(limit) = bill.wrap_up_usd {
            let spent = bill.meter.task(bill.task).usd;
            let step = bill
                .meter
                .price(
                    bill.connection,
                    model,
                    crate::spend::Usage {
                        input_tokens: request_tokens(&messages),
                        output_tokens: STEP_OUTPUT,
                        cached_tokens: 0,
                        cache_write_tokens: 0,
                    },
                )
                .unwrap_or(0.0);
            if spent + step > limit {
                return Err(Error::TaskBudget(format!(
                    "the next step (about ${step:.2}) would pass the ${limit:.2} limit \
                     (${spent:.2} spent)"
                )));
            }
            if wrap.is_none() && (spent >= limit * WRAP_UP_SHARE || spent + 2.0 * step > limit) {
                wrap = Some(Wrap::Spend);
                if let Some(p) = bill.progress {
                    p.say(role, "near the limit: writing up");
                }
                messages.push(crate::llm::Message {
                    role: "user".into(),
                    content: "[Ryter] You are near the spending limit the user set for this. \
                              Stop now: use no more tools, and write your answer from what you \
                              have, saying what you didn't get to check."
                        .into(),
                    tool_call_id: None,
                    tool_calls: None,
                });
            }
        }
        // The last step is for writing up, whether or not there is a
        // spending limit. Without one, a specialist used to run out of steps
        // unwarned, and the text beside its last tool call became its report:
        // an auditor's "I'll use Podman to run the six checks" was read as a
        // review with no verdict, and counted against the builder.
        if wrap.is_none() && round + 1 == rounds {
            wrap = Some(Wrap::Steps);
            if let Some(p) = bill.progress {
                p.say(role, "out of steps: writing up");
            }
            messages.push(crate::llm::Message {
                role: "user".into(),
                content: format!(
                    "[Ryter] This is the last of your {rounds} steps. Use no more tools: \
                     write your answer now from what you have, and say what you didn't get \
                     to. {}",
                    bill.closing.map_or("", Closing::shape)
                ),
                tool_call_id: None,
                tool_calls: None,
            });
        }
        let wrapping = wrap.is_some();
        let req = CompletionRequest {
            model: model.to_string(),
            system: None,
            messages: messages.clone(),
            tools: if wrapping {
                Vec::new()
            } else {
                crate::tools::specs_for_opts(role, ctx.web)
            },
            max_tokens: Some(max_tokens),
            reasoning: bill.meter.effort(role, model),
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
                StreamDelta::Text(t) => {
                    live.text(&t);
                    text.push_str(&t);
                }
                StreamDelta::Reasoning(t) => live.reasoning(&t),
                StreamDelta::ToolCall {
                    id,
                    name,
                    arguments,
                } => {
                    calls.push(&id, &name, &arguments);
                    live.call(&arguments, &calls);
                }
                StreamDelta::Usage(u) => usage = usage.merge(u),
                StreamDelta::ReportedCost(c) => reported = Some(c),
                StreamDelta::Truncated => truncated = true,
                _ => {}
            }
        }
        if let Some(keep) = bill.last_text {
            if !text.trim().is_empty() {
                if let Ok(mut k) = keep.lock() {
                    k.clone_from(&text);
                }
            }
        }
        // Charged every round, so a cap stops a runaway loop mid-task rather
        // than after it has spent the money.
        if let Err(e) = bill
            .meter
            .charge(bill.task, role, bill.connection, model, usage, reported)
        {
            // At the dollar cap, the user may raise it and the step goes on;
            // the reply it paid for is kept. A token cap stops as before.
            let raise = matches!(&e, Error::TaskBudget(why) if !why.contains("token cap"));
            let raised = match bill.ask {
                Some(ask) if raise => raise_cap(bill.meter, bill.task, ask).await,
                _ => false,
            };
            if !raised {
                return Err(e);
            }
        }
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
                let target = crate::agent::tool_summary(&call.name, &parsed);
                let tool_ctx = running_tool(bill.progress, role, &target, ctx);
                let out = run_tool(&call.name, parsed, &tool_ctx).await?;
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
        // Wrapping up: whatever it asked to run, this is the answer.
        if calls.is_empty() || (wrapping && !last.trim().is_empty()) {
            // An answer that stops short of its verdict or handback is asked
            // for it once, with no tools, while a step is left to ask in.
            if let Some(closing) = bill.closing.filter(|c| !c.met(&last)) {
                if !asked_closing && round + 1 < rounds {
                    asked_closing = true;
                    before_asking = Some(last.clone());
                    if let Some(p) = bill.progress {
                        p.say(role, closing.missing());
                    }
                    messages.push(crate::llm::Message {
                        role: "assistant".into(),
                        content: if last.trim().is_empty() {
                            "(no answer)".into()
                        } else {
                            text
                        },
                        tool_call_id: None,
                        tool_calls: None,
                    });
                    messages.push(crate::llm::Message {
                        role: "user".into(),
                        content: format!(
                            "[Ryter] That isn't a finished answer: nothing can be done with \
                             it as it is. Use no more tools, and write your final message \
                             now from what you have. {}",
                            closing.shape()
                        ),
                        tool_call_id: None,
                        tool_calls: None,
                    });
                    wrap = wrap.or(Some(Wrap::Closing));
                    continue;
                }
                // Asked, and still short of it: both answers are the
                // record of what it was doing when it stopped.
                if let Some(first) = before_asking.filter(|f| !f.trim().is_empty()) {
                    last = format!("{}\n\n{}", first.trim_end(), last.trim_start());
                }
            }
            return Ok(finish(last, wrap));
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
            let target = crate::agent::tool_summary(&call.name, &parsed);
            if let Some(p) = bill.progress {
                p.say(role, target.clone());
            }
            let tool_ctx = running_tool(bill.progress, role, &target, ctx);
            let out = run_tool(&call.name, parsed, &tool_ctx).await?;
            messages.push(crate::llm::Message {
                role: "tool".into(),
                content: out.text,
                tool_call_id: Some(call.id),
                tool_calls: None,
            });
        }
    }
    // Cut off on the last step itself: there was no step left to write up in.
    Ok(finish(last, Some(Wrap::Steps)))
}

/// Why a specialist was told to stop using tools and write its answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wrap {
    /// Near the task's spending limit.
    Spend,
    /// On the last of its steps.
    Steps,
    /// Its answer stopped short of its verdict or handback.
    Closing,
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
            ("can't build yet\nVERDICT: UNVERIFIED", false),
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
            after: Vec::new(),
            status: TaskStatus::Running,
            retries: 0,
            findings: String::new(),
            spent: Default::default(),
            cap_usd: None,
            gate_next: false,
            handback: String::new(),
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
        refuse_from: Mutex<Option<usize>>,
    }

    /// OpenRouter's reply when the account's zero data retention setting
    /// leaves a model no provider, as a live crew run got it.
    const ZDR_REFUSAL: &str = r#"http 404 Not Found: {"error":{"message":"0 endpoints out of 4 requested are available matching your guardrail restrictions and data policy. We removed them for the following reasons (an endpoint may have matched multiple reasons):\nZDR violation (account settings): 4 endpoints excluded; configurable at https://openrouter.ai/settings/privacy","code":404,"metadata":{"ineligibility_reasons":[{"reason":"zdr-violation-by-account","endpoint_count":4}]}}}"#;

    impl Scripted {
        fn new(turns: Vec<Vec<StreamDelta>>) -> Arc<Self> {
            Arc::new(Self {
                inner: ReplayProvider::scripted(turns),
                seen: Mutex::new(Vec::new()),
                before: Mutex::new(None),
                refuse_from: Mutex::new(None),
            })
        }
        /// From turn `n` on, answer as OpenRouter does a model the account's
        /// data policy refuses.
        fn refuse_from(&self, n: usize) {
            *self.refuse_from.lock().unwrap() = Some(n);
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
            if self
                .refuse_from
                .lock()
                .unwrap()
                .is_some_and(|from| n >= from)
            {
                return Err(Error::Provider(ZDR_REFUSAL.into()));
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
            ask: None,
            rejections: 0,
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

    /// The unblocking line is found in a NOTES list, as a real builder wrote it.
    #[test]
    fn the_reason_a_builder_is_blocked_is_the_fix() {
        let handback = "Scaffold done but it can't link.\n\n```\nSTATUS: BLOCKED\nFILES: Cargo.toml\nNOTES:\n- Resolved pq-sys version: **0.7.6**.\n- `cargo build` fails: cannot find -lpq\n- To unblock (Fedora 44): `sudo dnf install libpq-devel`, then cargo build passes.\n```";
        assert_eq!(
            blocked_reason(handback).unwrap(),
            "To unblock (Fedora 44): `sudo dnf install libpq-devel`, then cargo build passes."
        );
        assert_eq!(
            blocked_reason("STATUS: BLOCKED\nNOTES: the audio module isn't there").unwrap(),
            "the audio module isn't there"
        );
        assert_eq!(blocked_reason("STATUS: BLOCKED"), None);
    }

    /// `UNVERIFIED` is its own verdict, not a pass or a fail.
    #[test]
    fn unverified_is_its_own_verdict() {
        assert_eq!(
            verdict("- reads right\nVERDICT: UNVERIFIED"),
            Verdict::Unverified
        );
        assert_eq!(verdict("**VERDICT: PASS**"), Verdict::Pass);
        assert_eq!(verdict("no verdict at all"), Verdict::Fail);
    }

    /// A builder that says it is blocked is not checked, audited, or
    /// retried: its reason goes to the lead. One spent 80 calls working
    /// around a missing system library instead.
    #[tokio::test]
    async fn a_blocked_builder_is_neither_checked_nor_audited() {
        let f = fixture();
        let p = Scripted::new(vec![
            write_call("half.rs", "// started\n"),
            say(
                "STATUS: BLOCKED\nFILES: half.rs\nDECISIONS: none\nNOTES: needs alsa-lib-devel: sudo dnf install alsa-lib-devel",
            ),
            say("VERDICT: PASS"),
        ]);
        let meter = Meter::new(
            crate::spend::PriceBook::new(),
            crate::meter::Caps::default(),
        );
        let panel = vec![seat(&p, "auditor-m", "", &[])];
        let checks = vec!["false".to_string()];
        let t = task("t1", "scaffold");
        let out = run_with(&f, &p, &t, true, &checks, &panel, &meter, 2).await;
        assert_eq!(out.status, TaskStatus::Blocked, "not a retry: {out:?}");
        assert_eq!(p.calls(), 2, "no auditor was asked");
        assert!(
            out.findings.contains("sudo dnf install alsa-lib-devel"),
            "{}",
            out.findings
        );
        assert!(!f.repo.path().join("half.rs").exists(), "nothing lands");
    }

    /// With no checks, an auditor that can't build the task yet passes it on
    /// review alone, marked so the patch builds and tests it.
    #[tokio::test]
    async fn an_unverified_review_lands_marked() {
        let f = fixture();
        let p = Scripted::new(vec![
            write_call("src/audio.rs", "pub fn play() {}\n"),
            say("STATUS: DONE"),
            say("- no Cargo.toml yet\nVERDICT: UNVERIFIED"),
        ]);
        let out = run(&f, &p, &task("t1", "audio"), true, &[]).await;
        assert_eq!(out.status, TaskStatus::Done, "{out:?}");
        assert!(out.unverified);
        assert!(out.report.contains("review alone"), "{}", out.report);
    }

    /// Checks that ran and passed verified the tree: an `UNVERIFIED` then is
    /// a plain pass.
    #[tokio::test]
    async fn unverified_after_passing_checks_is_a_pass() {
        let f = fixture();
        let p = Scripted::new(vec![
            write_call("src/audio.rs", "pub fn play() {}\n"),
            say("STATUS: DONE"),
            say("VERDICT: UNVERIFIED"),
        ]);
        let checks = vec!["true".to_string()];
        let out = run(&f, &p, &task("t1", "audio"), true, &checks).await;
        assert_eq!(out.status, TaskStatus::Done, "{out:?}");
        assert!(!out.unverified);
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
                cache_write_tokens: 0,
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

    /// The branch a kept task left, and the file it holds there.
    fn kept(f: &Fixture, path: &str) -> String {
        let b = git::git(f.repo.path(), &["branch", "--list", "ryter-*"]).unwrap();
        let branch = b.trim().trim_start_matches(['*', '+', ' ']).to_string();
        assert!(!branch.is_empty(), "the work's branch was deleted");
        git::git(f.repo.path(), &["show", &format!("{branch}:{path}")]).unwrap()
    }

    /// An auditor the provider refuses is no verdict on the work. It used to
    /// fail the task and delete the branch: a live run lost $1.86 of finished
    /// builder work to an auditor that zero data retention ruled out.
    #[tokio::test]
    async fn an_auditor_the_account_refuses_keeps_the_builders_work() {
        let f = fixture();
        let p = Scripted::new(vec![
            write_call("extra.txt", "built\n"),
            say("STATUS: DONE"),
        ]);
        p.refuse_from(2);
        let panel = vec![seat(&p, "anthropic/claude-fable-5.1", "", &[])];
        let meter = Meter::new(
            crate::spend::PriceBook::new(),
            crate::meter::Caps::default(),
        );
        let out = run_with(&f, &p, &task("t1", "x"), true, &[], &panel, &meter, 0).await;
        assert_eq!(out.status, TaskStatus::Blocked, "{out:?}");
        for want in [
            "the auditor (anthropic/claude-fable-5.1) could not run",
            "zero data retention",
            "https://openrouter.ai/settings/privacy",
            "has the work",
        ] {
            assert!(out.summary.contains(want), "{want}: {}", out.summary);
        }
        assert!(!out.summary.contains('{'), "no JSON: {}", out.summary);
        assert!(!f.repo.path().join("extra.txt").exists(), "nothing merged");
        assert_eq!(kept(&f, "extra.txt"), "built\n");
    }

    fn bash_call(command: &str) -> Vec<StreamDelta> {
        vec![
            StreamDelta::ToolCall {
                id: "b".into(),
                name: "bash".into(),
                arguments: serde_json::json!({ "command": command }).to_string(),
            },
            StreamDelta::Done,
        ]
    }

    /// An audit that ends with no verdict decides nothing. From a live crew
    /// run: the auditor's last words were "I'll use Podman to build and run
    /// the six checks". That was read as a FAIL: the builder was run again
    /// on work nobody had faulted, six times, and the user was told to
    /// choose a stronger builder. The auditor is asked once for its verdict,
    /// with no tools; without one the work stays at the gate, unrejected.
    #[tokio::test]
    async fn an_audit_with_no_verdict_rejects_nothing() {
        let f = fixture();
        let p = Scripted::new(vec![
            write_call("extra.txt", "built\n"),
            say("STATUS: DONE\nFILES: extra.txt"),
            say("Podman is available. I'll use it to build and run the six checks."),
            say("Starting the build now."),
        ]);
        let mut t = task("t1", "x");
        // Retries are there to be spent: none is.
        let panel = vec![seat(&p, "auditor-m", "", &[])];
        let meter = Meter::new(
            crate::spend::PriceBook::new(),
            crate::meter::Caps::default(),
        );
        let out = run_with(&f, &p, &t, true, &[], &panel, &meter, 2).await;
        assert_eq!(p.calls(), 4, "the builder was not run again");
        assert_eq!(out.status, TaskStatus::Blocked, "{out:?}");
        assert!(!out.rejected, "{out:?}");
        for want in [
            "the auditor (auditor-m) ended without a verdict, even when asked for one",
            "It was not rejected",
            "has the work",
        ] {
            assert!(out.summary.contains(want), "{want}: {}", out.summary);
        }
        assert!(
            out.report.contains("I'll use it to build"),
            "{}",
            out.report
        );
        // It was asked once, with nothing to run.
        assert!(
            p.request(3)
                .contains("[Ryter] That isn't a finished answer")
                && p.request(3).contains("end with your verdict line"),
            "{}",
            p.request(3)
        );
        assert!(p.seen.lock().unwrap()[3].tools.is_empty());
        assert!(!p.seen.lock().unwrap()[2].tools.is_empty());
        assert!(!f.repo.path().join("extra.txt").exists(), "nothing merged");
        assert_eq!(kept(&f, "extra.txt"), "built\n");

        // The next run goes straight to the audit, and a verdict lands it.
        assert!(out.gate_next, "{out:?}");
        t.gate_next = out.gate_next;
        t.handback = out.handback;
        let p = Scripted::new(vec![say("- extra.txt:1 fine\n\nVERDICT: PASS")]);
        let out = run(&f, &p, &t, true, &["true".into()]).await;
        assert_eq!(out.status, TaskStatus::Done, "{out:?}");
        assert_eq!(p.calls(), 1, "only the audit ran");
    }

    /// Asked for its verdict, an auditor that gives one is taken at it.
    #[tokio::test]
    async fn an_auditor_asked_for_its_verdict_can_give_it() {
        let f = fixture();
        let p = Scripted::new(vec![
            write_call("extra.txt", "built\n"),
            say("STATUS: DONE"),
            say("I'll run the tests now."),
            say("- extra.txt reads right; I could not run it\n\nVERDICT: PASS"),
        ]);
        let out = run(&f, &p, &task("t1", "x"), true, &["true".into()]).await;
        assert_eq!(out.status, TaskStatus::Done, "{out:?}");
        assert_eq!(p.calls(), 4);
        assert!(f.repo.path().join("extra.txt").exists(), "merged");
    }

    /// With no checks set, the auditor is told what its shell runs and what
    /// it refuses, and to review by reading where that stops it. It used to
    /// be told to "build it and run its tests yourself", which sent it after
    /// a container build its shell refuses until its steps ran out.
    #[tokio::test]
    async fn with_no_checks_the_auditor_is_told_what_it_cannot_run() {
        let f = fixture();
        let p = Scripted::new(vec![
            write_call("extra.txt", "built\n"),
            say("STATUS: DONE"),
            say(
                "- reads right; the stack needs `docker compose`, which I can't run\n\nVERDICT: UNVERIFIED",
            ),
        ]);
        let out = run(&f, &p, &task("t1", "x"), true, &[]).await;
        let brief = p.request(2);
        for want in [
            "No checks are configured for this project",
            "in the project's containers too (`docker compose run --rm <service> pytest`",
            "including building, starting and stopping containers",
            "That is a limit on you, not on the project or the machine",
            "don't look for a way round",
            "`VERDICT: UNVERIFIED`",
        ] {
            assert!(brief.contains(want), "{want}:\n{brief}");
        }
        assert!(
            !brief.contains("Build it and run its tests yourself"),
            "{brief}"
        );
        // Reviewed by reading: it goes on to the patch, marked as not run.
        assert_eq!(out.status, TaskStatus::Done, "{out:?}");
        assert!(out.unverified, "{out:?}");
    }

    /// The step limits are the user's to set: an auditor given six steps is
    /// told so on its sixth, and its report says six.
    #[tokio::test]
    async fn a_step_limit_the_user_set_is_the_one_kept() {
        let f = fixture();
        let mut turns = vec![write_call("extra.txt", "built\n"), say("STATUS: DONE")];
        turns.extend((1..6).map(|_| bash_call("true")));
        turns.push(say(
            "- nothing wrong found; I ran out of steps\n\nVERDICT: FAIL",
        ));
        let p = Scripted::new(turns);
        let panel = vec![seat(&p, "auditor-m", "", &[])];
        let meter = Meter::new(
            crate::spend::PriceBook::new(),
            crate::meter::Caps::default(),
        )
        .with_steps(crate::config::Steps {
            auditor: 6,
            ..Default::default()
        });
        let out = run_with(&f, &p, &task("t1", "x"), true, &[], &panel, &meter, 0).await;
        assert_eq!(p.calls(), 2 + 6);
        assert!(
            p.request(7)
                .contains("[Ryter] This is the last of your 6 steps"),
            "{}",
            p.request(7)
        );
        assert!(!p.request(6).contains("the last of your"));
        assert!(
            out.findings
                .contains("[Ryter] The auditor reached its limit of 6 steps"),
            "{}",
            out.findings
        );
    }

    /// Every specialist's last step is for writing up, with no tools,
    /// whether or not a spending limit is set. Without one there was no
    /// warning: a run stopped at its step limit, and the text beside its
    /// last tool call became its report.
    #[tokio::test]
    async fn the_last_step_is_for_writing_up() {
        let f = fixture();
        let (builder_steps, _) = limits(Role::Builder);
        let (auditor_steps, _) = limits(Role::Auditor);
        let mut turns = vec![write_call("extra.txt", "built\n")];
        // The builder works until one step is left, then hands back.
        turns.extend((2..builder_steps).map(|_| bash_call("true")));
        turns.push(say(
            "STATUS: PARTIAL\nFILES: extra.txt\nNOTES: the migrations are left",
        ));
        // The auditor does the same, and still gives a verdict.
        turns.extend((1..auditor_steps).map(|_| bash_call("true")));
        turns.push(say(
            "- extra.txt: the task isn't finished (blocking)\n\nVERDICT: FAIL",
        ));
        let p = Scripted::new(turns);
        let out = run(&f, &p, &task("t1", "x"), true, &[]).await;
        assert_eq!(p.calls(), builder_steps + auditor_steps);

        let last_build = builder_steps - 1;
        let told = p.request(last_build);
        assert!(
            told.contains(&format!(
                "[Ryter] This is the last of your {builder_steps} steps"
            )) && told.contains("Use no more tools")
                && told.contains("`STATUS: PARTIAL`"),
            "{told}"
        );
        assert!(!p.request(last_build - 1).contains("the last of your"));
        let seen = p.seen.lock().unwrap();
        assert!(
            seen[last_build].tools.is_empty(),
            "no tools on the last step"
        );
        assert!(!seen[last_build - 1].tools.is_empty());
        drop(seen);

        // The auditor is given a handback that says why it stops where it does.
        let brief = p.request(builder_steps);
        assert!(
            brief.contains(&format!(
                "[Ryter] The builder reached its limit of {builder_steps} steps"
            )),
            "{brief}"
        );
        let last_audit = builder_steps + auditor_steps - 1;
        let told = p.request(last_audit);
        assert!(
            told.contains(&format!(
                "[Ryter] This is the last of your {auditor_steps} steps"
            )) && told.contains("end with your verdict line"),
            "{told}"
        );
        assert!(p.seen.lock().unwrap()[last_audit].tools.is_empty());
        // A verdict given on the last step is a verdict.
        assert!(out.rejected, "{out:?}");
        assert!(
            out.findings.contains(&format!(
                "[Ryter] The auditor reached its limit of {auditor_steps} steps"
            )),
            "{}",
            out.findings
        );
    }

    /// Any other call that fails after the builder committed keeps the work
    /// too: here the conflict resolver's provider refuses.
    #[tokio::test]
    async fn a_failure_after_the_build_keeps_the_builders_work() {
        let f = fixture();
        let repo = f.repo.path().to_path_buf();
        let p = Scripted::new(vec![write_call("README.md", "builder line\n"), say("done")]);
        p.before_turn(1, move || {
            std::fs::write(repo.join("README.md"), "upstream line\n").unwrap();
            git::commit_all(&repo, "upstream edit").unwrap();
        });
        p.refuse_from(2);
        let out = run(&f, &p, &task("t8", "edit readme"), true, &[]).await;
        assert_eq!(out.status, TaskStatus::Blocked, "{out:?}");
        assert!(
            out.summary.contains("stopped after the build"),
            "{}",
            out.summary
        );
        assert!(out.summary.contains("data policy"), "{}", out.summary);
        assert_eq!(kept(&f, "README.md"), "builder line\n");
        assert!(!mid_merge(f.repo.path()));
    }

    /// `turn` with its cost reported, as OpenRouter reports it.
    fn costing(mut turn: Vec<StreamDelta>, usd: f64) -> Vec<StreamDelta> {
        turn.insert(turn.len() - 1, StreamDelta::ReportedCost(usd));
        turn
    }

    /// A user who answers every question with the option `pick` chooses,
    /// and the questions asked.
    fn answering(pick: fn(&[String]) -> String) -> (CapAsk, Arc<Mutex<Vec<String>>>) {
        let (io, rx) = crate::user_io::UserIo::pair();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let seen = asked.clone();
        std::thread::spawn(move || {
            while let Ok(req) = rx.recv() {
                if let crate::user_io::UserRequest::Question {
                    question,
                    options,
                    reply,
                    ..
                } = req
                {
                    seen.lock().unwrap().push(question);
                    let _ = reply.send(pick(&options));
                }
            }
        });
        let ask = CapAsk {
            io,
            cancel: Cancel::new(),
            note: String::new(),
        };
        (ask, asked)
    }

    async fn run_asked(
        f: &Fixture,
        p: &Arc<Scripted>,
        t: &Task,
        meter: &Meter,
        ask: Option<&CapAsk>,
    ) -> TaskOutcome {
        let panel = vec![seat(p, "auditor-m", "", &[])];
        let job = BuildJob {
            ask,
            rejections: 0,
            provider: p.clone(),
            model: "m",
            connection: "c",
            auditors: &panel,
            meter,
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
            progress: None,
        };
        run_build_task(&job, t).await.unwrap()
    }

    fn capped(usd: f64) -> Meter {
        Meter::new(
            crate::spend::PriceBook::new(),
            crate::meter::Caps {
                task_usd: usd,
                ..crate::meter::Caps::default()
            },
        )
    }

    /// At the cap the user is asked; raising it carries the step on, the
    /// reply it paid for kept. It used to stop there, the step thrown away.
    #[tokio::test]
    async fn at_the_cap_the_user_can_raise_it_and_the_task_carries_on() {
        let f = fixture();
        let p = Scripted::new(vec![
            costing(write_call("extra.txt", "x\n"), 0.04),
            costing(say("STATUS: DONE"), 0.02),
            costing(say("VERDICT: PASS"), 0.01),
        ]);
        let (ask, asked) = answering(|o| o[0].clone());
        let meter = capped(0.05);
        let out = run_asked(&f, &p, &task("t1", "x"), &meter, Some(&ask)).await;
        assert_eq!(out.status, TaskStatus::Done, "{out:?}");
        let asked = asked.lock().unwrap();
        assert_eq!(asked.len(), 1, "{asked:?}");
        assert!(
            asked[0].contains("Task t1 has spent $0.06 of its $0.05 cap (builder $0.06 this run)"),
            "{asked:?}"
        );
        assert!((meter.task_cap("t1") - 5.05).abs() < 1e-9);
    }

    /// Stopped at the cap during the audit, the next run goes straight back
    /// to the audit: it used to run the builder again, paying twice.
    #[tokio::test]
    async fn a_stop_during_the_audit_resumes_at_the_audit() {
        let f = fixture();
        let p = Scripted::new(vec![
            costing(write_call("extra.txt", "x\n"), 0.02),
            costing(say("STATUS: DONE\nFILES: extra.txt"), 0.01),
            costing(say("VERDICT: PASS"), 0.03),
        ]);
        let (ask, _) = answering(|o| o.last().unwrap().clone());
        let meter = capped(0.05);
        let mut t = task("t1", "x");
        let out = run_asked(&f, &p, &t, &meter, Some(&ask)).await;
        assert_eq!(out.status, TaskStatus::Blocked, "{out:?}");
        assert!(out.gate_next, "{out:?}");
        assert!(out.handback.contains("STATUS: DONE"), "{out:?}");
        assert!(
            out.findings
                .contains("recreating the task starts the build over"),
            "{out:?}"
        );
        assert!(!f.repo.path().join("extra.txt").exists(), "nothing merged");

        // The next run: its cap raised, only the auditor is called.
        t.gate_next = out.gate_next;
        t.handback = out.handback;
        t.spent = meter.task("t1");
        let p = Scripted::new(vec![costing(say("VERDICT: PASS"), 0.01)]);
        let meter = capped(0.05)
            .with_prior([("t1".to_string(), t.spent)].into())
            .with_task_caps([("t1".to_string(), 5.05)].into());
        let out = run_asked(&f, &p, &t, &meter, None).await;
        assert_eq!(out.status, TaskStatus::Done, "{out:?}");
        assert_eq!(p.calls(), 1, "only the audit ran");
        assert!(
            p.request(0).contains("STATUS: DONE"),
            "the auditor got the handback"
        );
        assert_eq!(
            std::fs::read_to_string(f.repo.path().join("extra.txt")).unwrap(),
            "x\n"
        );
    }

    /// A task already at its cap stops before a paid step that could only
    /// stop again. It used to pay for one call and stop.
    #[tokio::test]
    async fn a_task_at_its_cap_spends_nothing_to_stop_again() {
        let f = fixture();
        let p = Scripted::new(vec![say("should not run")]);
        let spent = crate::meter::Tally {
            usd: 0.06,
            ..Default::default()
        };
        let meter = capped(0.05).with_prior([("t1".to_string(), spent)].into());
        let out = run_asked(&f, &p, &task("t1", "x"), &meter, None).await;
        assert_eq!(out.status, TaskStatus::Blocked);
        assert!(out.findings.contains("still at its $0.05 cap"), "{out:?}");
        assert_eq!(p.calls(), 0);
        // Asked, the user can raise it first, and then it runs.
        let p = Scripted::new(vec![
            write_call("extra.txt", "x\n"),
            say("STATUS: DONE"),
            say("VERDICT: PASS"),
        ]);
        let (ask, _) = answering(|o| o[1].clone());
        let out = run_asked(&f, &p, &task("t1", "x"), &meter, Some(&ask)).await;
        assert_eq!(out.status, TaskStatus::Done, "{out:?}");
        assert!((meter.task_cap("t1") - 2.05).abs() < 1e-9);
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
            ask: None,
            rejections: 0,
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
                sink: Some(tx),
                id: crate::queue::new_sub_id(),
                log: None,
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

    /// Between tool calls the board used to show nothing, for minutes. Now
    /// each step says what it is doing as it streams: waiting, thinking with
    /// its reasoning, writing a file with the file so far, running a tool.
    #[tokio::test]
    async fn specialists_report_live_what_they_are_doing() {
        let f = fixture();
        let p = Scripted::new(vec![
            vec![
                StreamDelta::Reasoning(
                    "The panel needs the elapsed time.\nI'll add a gauge.".into(),
                ),
                StreamDelta::ToolCall {
                    id: "w".into(),
                    name: "write".into(),
                    arguments: r#"{"path":"src/ui.rs","content":"fn draw() {\n"#.into(),
                },
                StreamDelta::ToolCall {
                    id: String::new(),
                    name: String::new(),
                    arguments: r#"    let g = gauge();\n}\n"}"#.into(),
                },
                StreamDelta::Done,
            ],
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
            ask: None,
            rejections: 0,
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
                sink: Some(tx),
                id: crate::queue::new_sub_id(),
                log: None,
            }),
        };
        run_build_task(&job, &task("t1", "x")).await.unwrap();
        let live: Vec<(LivePhase, String, u32, Vec<String>)> = rx
            .try_iter()
            .filter_map(|e| match e {
                AgentEvent::SubagentLive {
                    phase,
                    target,
                    lines,
                    tail,
                    ..
                } => Some((phase, target, lines, tail)),
                _ => None,
            })
            .collect();
        assert_eq!(live[0].0, LivePhase::Waiting, "{live:?}");
        assert!(
            live.iter()
                .any(|(ph, _, _, tail)| *ph == LivePhase::Thinking
                    && tail.last().is_some_and(|l| l.contains("add a gauge"))),
            "{live:?}"
        );
        assert!(
            live.iter()
                .any(|(ph, target, _, tail)| *ph == LivePhase::Writing
                    && target == "edit src/ui.rs"
                    && tail.first().is_some_and(|l| l.contains("fn draw"))),
            "{live:?}"
        );
        assert!(
            live.iter()
                .any(|(ph, target, ..)| *ph == LivePhase::Running && target == "edit src/ui.rs"),
            "{live:?}"
        );
    }

    #[test]
    fn a_tool_call_still_arriving_is_read_as_far_as_it_goes() {
        let (target, body) = partial_call(
            "search_replace",
            r#"{"path":"src/ui.rs","old_string":"x","new_string":"fn a() {\n    let t = \"é\u00e9\";\n    le"#,
        );
        assert_eq!(target, "edit src/ui.rs");
        let body = body.unwrap();
        assert_eq!(body, "fn a() {\n    let t = \"éé\";\n    le");
        assert_eq!(tail_lines(&body, 2), ["    let t = \"éé\";", "    le"]);
        let (target, body) = partial_call("bash", r#"{"command":"cargo te"#);
        assert_eq!((target.as_str(), body), ("bash cargo te", None));
        // Before the value starts there is nothing to show.
        assert_eq!(partial_str(r#"{"path":"#, "path"), None);
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
