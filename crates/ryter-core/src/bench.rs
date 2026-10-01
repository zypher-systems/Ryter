//! `ryter bench`: run real tasks through the real crew, and measure what lands.
//!
//! Each task is a fixture repository, a brief, the checks the crew's gate runs,
//! and *hidden* acceptance tests the crew never sees. The hidden tests run only
//! after the work lands on the branch, and they are the ground truth: they
//! separate "an auditor signed it off" from "it works". The number that decides
//! whether a tiering is worth it is cost per accepted task, and the false-pass
//! count says how far the auditor can be trusted.
//!
//! Layout of a suite:
//!
//! ```text
//! bench/<task>/task.toml   title, brief, files, checks, accept, needs, [[tasks]]
//! bench/<task>/repo/       the fixture the crew works in
//! bench/<task>/hidden/     copied in only for acceptance, after landing
//! ```
//!
//! A task says what it `needs` (`cargo --version`), and is skipped where
//! that isn't installed. `[[tasks]]` splits it into several builder tasks,
//! some waiting on others, the way an architect's plan does: independent
//! ones run side by side.
//!
//! A run is published as a [`Report`]: a page for people and a file the next
//! run is compared with, so a release can be checked against the last.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::agent::Agent;
use crate::config::Config;
use crate::error::{Error, Result};
use crate::llm::Provider;
use crate::phase::Phase;
use crate::role::Role;
use crate::session::Session;
use crate::spend::PriceBook;
use crate::tools::ToolContext;

/// One task definition (`task.toml`).
#[derive(Debug, Clone, Deserialize)]
pub struct TaskSpec {
    /// One-line title.
    pub title: String,
    /// The builder's spec.
    pub brief: String,
    /// Paths the task owns.
    #[serde(default)]
    pub files: Vec<String>,
    /// Gate checks the crew runs (visible to it).
    #[serde(default)]
    pub checks: Vec<String>,
    /// Hidden acceptance commands, run after landing with `hidden/` copied in.
    pub accept: Vec<String>,
    /// Commands that must succeed for the task to run on this machine
    /// (`cargo --version`). A task whose tools are missing is skipped.
    #[serde(default)]
    pub needs: Vec<String>,
    /// Several builder tasks in place of the one. Empty: `title`, `brief`
    /// and `files` are the single task.
    #[serde(default)]
    pub tasks: Vec<SubTask>,
}

/// One builder task of a multi-task benchmark.
#[derive(Debug, Clone, Deserialize)]
pub struct SubTask {
    /// Its id in the queue, named by `after`.
    pub id: String,
    /// One-line title.
    pub title: String,
    /// The builder's spec.
    pub brief: String,
    /// Paths it owns.
    #[serde(default)]
    pub files: Vec<String>,
    /// Tasks that must land first.
    #[serde(default)]
    pub after: Vec<String>,
}

/// A task on disk.
#[derive(Debug, Clone)]
pub struct BenchTask {
    /// Directory name.
    pub name: String,
    /// Its definition.
    pub spec: TaskSpec,
    /// `bench/<name>`.
    pub dir: PathBuf,
}

/// What happened to one task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BenchResult {
    /// Task name.
    pub task: String,
    /// The work reached the branch (checks and every auditor passed).
    pub landed: bool,
    /// The hidden acceptance tests passed on the landed work.
    pub accepted: bool,
    /// Known USD across the crew (a lower bound if `unpriced`).
    pub usd: f64,
    /// Some call had no price.
    pub unpriced: bool,
    /// Uncached input plus output tokens.
    pub billable_tokens: u64,
    /// USD by role.
    pub usd_by_role: std::collections::BTreeMap<String, f64>,
    /// Wall-clock seconds.
    pub secs: f64,
    /// First line of the crew report, or the error. Where no model answered,
    /// the reason follows it.
    pub outcome: String,
    /// Model calls that were answered, across the crew.
    #[serde(default)]
    pub calls: u64,
}

impl BenchResult {
    /// No model answered, so the crew wasn't measured: a refused key, a
    /// model the account can't reach, a crew that won't start. Such a run
    /// says nothing about the crew, and isn't a result.
    pub fn unanswered(&self) -> bool {
        self.calls == 0 && !self.landed
    }
}

impl BenchTask {
    /// The first of its `needs` this machine can't meet, if any.
    pub fn missing(&self) -> Option<String> {
        let cancel = crate::cancel::Cancel::new();
        self.spec.needs.iter().find_map(|need| {
            let ran =
                crate::tools::shell::run_command(need, &self.dir, Duration::from_secs(20), &cancel);
            (!matches!(ran, Ok(crate::tools::shell::Run::Ok(_)))).then(|| need.clone())
        })
    }

    /// The builder tasks it queues: its own, or the one it is.
    fn queue_items(&self) -> Vec<serde_json::Value> {
        if self.spec.tasks.is_empty() {
            return vec![serde_json::json!({
                "id": "t1",
                "title": self.spec.title,
                "brief": self.spec.brief,
                "files": self.spec.files,
            })];
        }
        self.spec
            .tasks
            .iter()
            .map(|t| {
                serde_json::json!({
                    "id": t.id, "title": t.title, "brief": t.brief,
                    "files": t.files, "after": t.after,
                })
            })
            .collect()
    }
}

/// Load every task under `suite`, sorted by name.
pub fn load_suite(suite: &Path) -> Result<Vec<BenchTask>> {
    let mut out = Vec::new();
    let rd =
        std::fs::read_dir(suite).map_err(|e| Error::Io(format!("{}: {e}", suite.display())))?;
    for ent in rd.flatten() {
        let dir = ent.path();
        let spec_path = dir.join("task.toml");
        if !spec_path.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&spec_path).map_err(|e| Error::Io(e.to_string()))?;
        let spec: TaskSpec = toml::from_str(&text)
            .map_err(|e| Error::Config(format!("{}: {e}", spec_path.display())))?;
        out.push(BenchTask {
            name: ent.file_name().to_string_lossy().into_owned(),
            spec,
            dir,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Everything a run needs besides the task.
pub struct BenchEnv {
    /// Config with the crew routing under test.
    pub cfg: Config,
    /// The lead's provider (specialists resolve from `cfg`).
    pub provider: Arc<dyn Provider>,
    /// Lead connection.
    pub connection: String,
    /// Lead model.
    pub model: String,
    /// Scratch home for sessions and worktrees; never the user's `~/.ryter`.
    pub home: PathBuf,
    /// Spend cap for one task (session budget).
    pub budget_usd: f64,
    /// Builders at once, for a task split into several.
    pub max_crew: u32,
    /// Wall clock for each acceptance command.
    pub accept_timeout: Duration,
}

/// Run one task end to end in a fresh repository.
pub async fn run_task(task: &BenchTask, env: &BenchEnv) -> BenchResult {
    let started = Instant::now();
    let mut result = BenchResult {
        task: task.name.clone(),
        landed: false,
        accepted: false,
        usd: 0.0,
        unpriced: false,
        billable_tokens: 0,
        usd_by_role: Default::default(),
        secs: 0.0,
        outcome: String::new(),
        calls: 0,
    };
    match run_inner(task, env, &mut result).await {
        Ok(()) => {}
        Err(e) => result.outcome = format!("error: {e}"),
    }
    result.secs = started.elapsed().as_secs_f64();
    result
}

async fn run_inner(task: &BenchTask, env: &BenchEnv, result: &mut BenchResult) -> Result<()> {
    let work = env.home.join("repos").join(format!(
        "{}-{}",
        task.name,
        crate::ids::SessionId::generate().as_str()
    ));
    copy_dir(&task.dir.join("repo"), &work)?;
    crate::git::git(&work, &["init", "-q", "-b", "main"])?;
    crate::git::git(&work, &["config", "user.email", "bench@ryter"])?;
    crate::git::git(&work, &["config", "user.name", "ryter bench"])?;
    crate::git::git(&work, &["config", "commit.gpgsign", "false"])?;
    crate::git::commit_all(&work, "bench: fixture")?;
    let before = crate::git::head(&work)?;

    let session = Session::create(
        &env.home,
        &work,
        Phase::Build,
        env.connection.clone(),
        env.model.clone(),
    )?;
    let queue = Arc::new(Mutex::new(crate::queue::TaskQueue::open(
        session.dir.join("tasks.json"),
    )));
    queue
        .lock()
        .map_err(|e| Error::Config(e.to_string()))?
        .apply_todo(&serde_json::json!({ "items": task.queue_items() }))?;
    let mut agent = Agent {
        provider: env.provider.clone(),
        book: PriceBook::from_config(&env.cfg),
        ctx: ToolContext {
            live: None,
            workspace: work.clone(),
            notes_dir: session.notes_dir(),
            role: Role::Orchestrator,
            // Nobody is watching a benchmark; Ask would otherwise deny.
            always_approve: true,
            queue: queue.clone(),
            mcp: None,
            hooks: None,
            cancel: crate::cancel::Cancel::new(),
            user_io: None,
            allowed: Default::default(),
            web: false,
        },
        session,
        connection: env.connection.clone(),
        model: env.model.clone(),
        role: Role::Orchestrator,
        max_turns: 1,
        budget_usd: env.budget_usd,
        sink: None,
        home: env.home.clone(),
        project_root: Some(work.clone()),
        trusted: false,
        queue,
        max_crew: env.max_crew.max(1),
        max_retries: env.cfg.auditor.max_retries,
        checks: task.spec.checks.clone(),
        check_timeout_secs: env.cfg.auditor.check_timeout_secs,
        context_window: 0,
        cfg: Some(env.cfg.clone()),
        running: Arc::new(Mutex::new(Vec::new())),
    };
    let report = agent.drain_crew().await;
    tally(&agent.session, result)?;
    let report = report?;
    let mut lines = report.lines().skip_while(|l| !l.starts_with("### "));
    result.outcome = match lines.next() {
        Some(heading) => heading.trim_start_matches("### ").to_string(),
        None => report.lines().next().unwrap_or("").to_string(),
    };
    result.landed = crate::git::head(&work)? != before && agent.session.meta.patch.is_none();
    if !result.landed {
        // Why no model answered is the line under the heading.
        if let Some(why) = lines
            .find(|l| !l.trim().is_empty())
            .filter(|_| result.calls == 0)
        {
            result.outcome = format!("{}: {}", result.outcome, why.trim());
        }
        return Ok(());
    }
    // Ground truth: tests the crew never saw, on exactly what landed.
    let hidden = task.dir.join("hidden");
    if hidden.is_dir() {
        copy_dir(&hidden, &work)?;
    }
    let cancel = crate::cancel::Cancel::new();
    for cmd in &task.spec.accept {
        match crate::tools::shell::run_command(cmd, &work, env.accept_timeout, &cancel)? {
            crate::tools::shell::Run::Ok(_) => {}
            other => {
                result.outcome = format!("landed, but acceptance failed: {cmd} ({other:?})");
                return Ok(());
            }
        }
    }
    result.accepted = true;
    Ok(())
}

fn tally(session: &Session, result: &mut BenchResult) -> Result<()> {
    for rec in session.spend_log()? {
        // One record per call a model answered.
        result.calls += 1;
        result.billable_tokens +=
            rec.input_tokens.saturating_sub(rec.cached_tokens) + rec.output_tokens;
        match rec.total_usd {
            Some(u) => {
                result.usd += u;
                *result.usd_by_role.entry(rec.role.to_string()).or_default() += u;
            }
            None => result.unpriced = true,
        }
    }
    Ok(())
}

/// Totals over a run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Summary {
    /// Tasks run.
    pub tasks: usize,
    /// Tasks whose work landed.
    pub landed: usize,
    /// Tasks whose landed work passed the hidden tests.
    pub accepted: usize,
    /// Landed (so an auditor signed off) but failed the hidden tests.
    pub false_passes: usize,
    /// Known USD.
    pub usd: f64,
    /// Some call was unpriced.
    pub unpriced: bool,
}

impl Summary {
    /// Totals for `results`.
    pub fn of(results: &[BenchResult]) -> Self {
        let mut s = Self {
            tasks: results.len(),
            ..Self::default()
        };
        for r in results {
            s.landed += usize::from(r.landed);
            s.accepted += usize::from(r.accepted);
            s.false_passes += usize::from(r.landed && !r.accepted);
            s.usd += r.usd;
            s.unpriced |= r.unpriced;
        }
        s
    }

    /// The number that decides whether a tiering is worth it.
    pub fn usd_per_accepted(&self) -> Option<f64> {
        (self.accepted > 0).then(|| self.usd / self.accepted as f64)
    }

    /// A few lines for the terminal.
    pub fn render(&self) -> String {
        let bound = if self.unpriced { "≥" } else { "" };
        let per = self
            .usd_per_accepted()
            .map(|u| format!("{bound}${u:.3}"))
            .unwrap_or_else(|| "n/a (nothing accepted)".into());
        format!(
            "landed {}/{} · accepted {}/{} · false passes {} · total {bound}${:.3} · per accepted task {per}",
            self.landed, self.tasks, self.accepted, self.tasks, self.false_passes, self.usd
        )
    }
}

/// A task that didn't run, and why.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Skipped {
    /// Task name.
    pub task: String,
    /// What it needs that this machine lacks.
    pub needs: String,
}

/// One published run: who ran it, on what, and what happened to each task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Report {
    /// Ryter's version.
    pub ryter: String,
    /// The day it ran, `YYYY-MM-DD` (UTC).
    pub date: String,
    /// The lead's model.
    pub lead: String,
    /// The builder's model.
    pub builder: String,
    /// The auditor's model.
    pub auditor: String,
    /// The spend cap per task.
    pub budget_usd: f64,
    /// Every run of every task.
    pub results: Vec<BenchResult>,
    /// Tasks left out here.
    #[serde(default)]
    pub skipped: Vec<Skipped>,
}

/// One task across its runs.
struct TaskLine {
    task: String,
    runs: usize,
    landed: usize,
    accepted: usize,
    usd: f64,
    secs: f64,
}

impl Report {
    /// Totals over every run.
    pub fn summary(&self) -> Summary {
        Summary::of(&self.results)
    }

    fn by_task(&self) -> Vec<TaskLine> {
        let mut lines: Vec<TaskLine> = Vec::new();
        for r in &self.results {
            let at = match lines.iter().position(|l| l.task == r.task) {
                Some(i) => i,
                None => {
                    lines.push(TaskLine {
                        task: r.task.clone(),
                        runs: 0,
                        landed: 0,
                        accepted: 0,
                        usd: 0.0,
                        secs: 0.0,
                    });
                    lines.len() - 1
                }
            };
            let l = &mut lines[at];
            l.runs += 1;
            l.landed += usize::from(r.landed);
            l.accepted += usize::from(r.accepted);
            l.usd += r.usd;
            l.secs += r.secs;
        }
        lines
    }

    /// The page: the headline, who ran, and a row per task.
    pub fn markdown(&self) -> String {
        let s = self.summary();
        let bound = if s.unpriced { "≥" } else { "" };
        let per = s
            .usd_per_accepted()
            .map(|u| format!("{bound}${u:.3}"))
            .unwrap_or_else(|| "n/a".into());
        let mut md = format!(
            "# Benchmark\n\n`ryter bench` runs each task in `bench/` through the real crew, in a \
             fresh repository: a builder does the work, the task's checks and the auditor decide \
             whether it lands, and tests the crew never saw decide whether it was right. This \
             page is the last published run. `ryter bench --publish docs/bench` replaces it and \
             says how the new run compares.\n\n\
             ## The last run\n\n\
             - **Ryter:** {} on {}\n\
             - **Crew:** lead `{}`, builder `{}`, auditor `{}`\n\
             - **Cap:** ${:.2} per task\n\n\
             | Measure | Result |\n|---|---|\n\
             | Landed (checks and the auditor passed) | {} of {} |\n\
             | Accepted (the hidden tests passed too) | {} of {} |\n\
             | False passes (landed, but wrong) | {} |\n\
             | Total cost | {bound}${:.3} |\n\
             | Cost per accepted task | {per} |\n\n\
             One run of each task is a small sample, and the same crew can take several times \
             as many steps on one run as on the next. Read the cost as what this run cost, not \
             as what the next will.\n\n\
             ## By task\n\n\
             | Task | Runs | Landed | Accepted | Cost per run | Time per run |\n\
             |---|---|---|---|---|---|\n",
            self.ryter,
            self.date,
            self.lead,
            self.builder,
            self.auditor,
            self.budget_usd,
            s.landed,
            s.tasks,
            s.accepted,
            s.tasks,
            s.false_passes,
            s.usd,
        );
        for l in self.by_task() {
            let n = l.runs.max(1) as f64;
            md.push_str(&format!(
                "| `{}` | {} | {} | {} | ${:.3} | {:.0} s |\n",
                l.task,
                l.runs,
                l.landed,
                l.accepted,
                l.usd / n,
                l.secs / n
            ));
        }
        if !self.skipped.is_empty() {
            md.push_str("\n## Not run\n\n");
            for sk in &self.skipped {
                md.push_str(&format!(
                    "- `{}`: this machine lacks `{}`\n",
                    sk.task, sk.needs
                ));
            }
        }
        md
    }

    /// How this run compares with `before`, in lines, and whether it is
    /// worse where it counts: fewer accepted on a task, or more false
    /// passes. Cost is reported and never counted as worse: it varies run to
    /// run by more than any change to Ryter is likely to move it.
    pub fn compare(&self, before: &Report) -> (Vec<String>, bool) {
        let (now, then) = (self.summary(), before.summary());
        let mut lines = vec![format!(
            "against {} ({}): accepted {}/{} then, {}/{} now · false passes {} then, {} now",
            before.ryter,
            before.date,
            then.accepted,
            then.tasks,
            now.accepted,
            now.tasks,
            then.false_passes,
            now.false_passes
        )];
        let mut worse = now.false_passes > then.false_passes;
        let old = before.by_task();
        for l in self.by_task() {
            let Some(o) = old.iter().find(|o| o.task == l.task) else {
                lines.push(format!("  {}: new", l.task));
                continue;
            };
            // Rates, so a different --repeat still compares.
            let rate = |t: &TaskLine| t.accepted as f64 / t.runs.max(1) as f64;
            if rate(&l) < rate(o) {
                worse = true;
                lines.push(format!(
                    "  {}: WORSE, accepted {}/{} then, {}/{} now",
                    l.task, o.accepted, o.runs, l.accepted, l.runs
                ));
            } else if rate(&l) > rate(o) {
                lines.push(format!(
                    "  {}: better, accepted {}/{} then, {}/{} now",
                    l.task, o.accepted, o.runs, l.accepted, l.runs
                ));
            }
        }
        // A task that ran then and not now would otherwise drop out unseen.
        let ran = self.by_task();
        for o in &old {
            if !ran.iter().any(|l| l.task == o.task) {
                let why = match self.skipped.iter().find(|s| s.task == o.task) {
                    Some(s) => format!("skipped, this machine lacks `{}`", s.needs),
                    None => "no longer in the suite".into(),
                };
                lines.push(format!("  {}: not run this time ({why})", o.task));
            }
        }
        if let (Some(a), Some(b)) = (then.usd_per_accepted(), now.usd_per_accepted()) {
            lines.push(format!(
                "  cost per accepted task: ${a:.3} then, ${b:.3} now (it varies run to run)"
            ));
        }
        (lines, worse)
    }
}

/// What publishing a run did.
#[derive(Debug, Clone, PartialEq)]
pub struct Published {
    /// How the run compares with the one published before, in lines.
    pub lines: Vec<String>,
    /// Why the published run was kept and this one not written, if it was.
    pub kept: Option<String>,
}

impl Report {
    /// Publish this run as `<stem>.md` and `<stem>.json`.
    ///
    /// The published run is what the next is compared with, so it is
    /// replaced only by a run that can stand in for it: one that ran
    /// something, had a model answer on every task, ran every task of the
    /// published run that is still in the suite, and did no worse. Otherwise
    /// the published run is kept: an error where this run measured nothing,
    /// and `kept` saying why where it measured something worse. Writing
    /// regardless let an empty run (`--repeat 0`, every task skipped) wipe
    /// the baseline and report success.
    pub fn publish(&self, stem: &Path) -> Result<Published> {
        let (md, json) = (stem.with_extension("md"), stem.with_extension("json"));
        if self.results.is_empty() {
            return Err(Error::Config(format!(
                "no task ran, so there is nothing to publish; {} is kept",
                json.display()
            )));
        }
        if let Some(r) = self.results.iter().find(|r| r.unanswered()) {
            return Err(Error::Config(format!(
                "no model answered on {} ({}), so the run isn't a result; {} is kept",
                r.task,
                r.outcome,
                json.display()
            )));
        }
        let mut published = Published {
            lines: Vec::new(),
            kept: None,
        };
        if json.exists() {
            let before: Report = std::fs::read_to_string(&json)
                .map_err(|e| Error::Io(format!("{}: {e}", json.display())))
                .and_then(|t| {
                    serde_json::from_str(&t).map_err(|e| {
                        Error::Config(format!(
                            "{} isn't a published run ({e}); move it aside to publish afresh",
                            json.display()
                        ))
                    })
                })?;
            let (lines, worse) = self.compare(&before);
            published.lines = lines;
            let unrun: Vec<&str> = before
                .by_task()
                .iter()
                .filter_map(|o| self.skipped.iter().find(|s| s.task == o.task))
                .map(|s| s.task.as_str())
                .collect();
            if worse {
                published.kept = Some("this run is worse than the published one".into());
            } else if !unrun.is_empty() {
                published.kept = Some(format!(
                    "{} of the published run didn't run here",
                    unrun.join(", ")
                ));
            }
        }
        if published.kept.is_none() {
            let io = |e: std::io::Error| Error::Io(e.to_string());
            if let Some(dir) = md.parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir).map_err(io)?;
            }
            let body = serde_json::to_string_pretty(self).map_err(|e| Error::Io(e.to_string()))?;
            // Each file is written beside itself and renamed into place, the
            // data last: a failure part-way leaves the published run whole.
            for (ext, text) in [("md", self.markdown()), ("json", format!("{body}\n"))] {
                let tmp = stem.with_extension(format!("{ext}.{}.tmp", std::process::id()));
                std::fs::write(&tmp, text).map_err(io)?;
                std::fs::rename(&tmp, stem.with_extension(ext)).map_err(|e| {
                    let _ = std::fs::remove_file(&tmp);
                    io(e)
                })?;
            }
        }
        Ok(published)
    }
}

/// Today, `YYYY-MM-DD` (UTC), for a report.
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    crate::prompt::civil_date(secs / 86_400)
}

/// Copy a directory tree (files and subdirectories).
fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to).map_err(|e| Error::Io(e.to_string()))?;
    let rd = std::fs::read_dir(from).map_err(|e| Error::Io(format!("{}: {e}", from.display())))?;
    for ent in rd.flatten() {
        let src = ent.path();
        let dst = to.join(ent.file_name());
        if src.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst).map_err(|e| Error::Io(e.to_string()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{ReplayProvider, StreamDelta};
    use tempfile::TempDir;

    /// A one-task suite: `add(a, b)` is wrong, a visible test that only
    /// checks one case, and a hidden test that checks another.
    fn suite() -> TempDir {
        let d = TempDir::new().unwrap();
        let t = d.path().join("fix-add");
        std::fs::create_dir_all(t.join("repo")).unwrap();
        std::fs::create_dir_all(t.join("hidden")).unwrap();
        std::fs::write(
            t.join("task.toml"),
            r#"
title = "fix add"
brief = "add(a, b) must return a + b."
files = ["calc.py"]
checks = ["python3 -c 'import calc; assert calc.add(2, 2) == 4'"]
accept = ["python3 hidden_check.py"]
"#,
        )
        .unwrap();
        std::fs::write(t.join("repo/calc.py"), "def add(a, b):\n    return a * b\n").unwrap();
        std::fs::write(
            t.join("hidden/hidden_check.py"),
            "import calc\nassert calc.add(2, 3) == 5\n",
        )
        .unwrap();
        d
    }

    fn write(content: &str) -> Vec<StreamDelta> {
        vec![
            StreamDelta::ToolCall {
                id: "w".into(),
                name: "write".into(),
                arguments: serde_json::json!({"path": "calc.py", "content": content}).to_string(),
            },
            StreamDelta::Done,
        ]
    }

    fn say(text: &str) -> Vec<StreamDelta> {
        vec![
            StreamDelta::Text(text.into()),
            StreamDelta::Usage(crate::spend::Usage {
                input_tokens: 1_000,
                output_tokens: 50,
                cached_tokens: 0,
                cache_write_tokens: 0,
            }),
            StreamDelta::ReportedCost(0.01),
            StreamDelta::Done,
        ]
    }

    fn env(home: &Path, turns: Vec<Vec<StreamDelta>>) -> BenchEnv {
        let mut cfg = Config::default();
        cfg.specialists.insert(
            "auditor".into(),
            crate::config::RoleModel {
                connection: Some("spacexai".into()),
                model: Some("auditor-model".into()),
            },
        );
        cfg.auditor.max_retries = 0;
        BenchEnv {
            cfg,
            provider: Arc::new(ReplayProvider::scripted(turns)),
            connection: "spacexai".into(),
            model: "grok-4.6".into(),
            home: home.to_path_buf(),
            budget_usd: 1.0,
            max_crew: 2,
            accept_timeout: Duration::from_secs(30),
        }
    }

    #[tokio::test]
    async fn a_correct_fix_lands_and_is_accepted() {
        let s = suite();
        let home = TempDir::new().unwrap();
        let tasks = load_suite(s.path()).unwrap();
        assert_eq!(tasks.len(), 1);
        let e = env(
            home.path(),
            vec![
                write("def add(a, b):\n    return a + b\n"),
                say("STATUS: DONE"),
                say("VERDICT: PASS"),
            ],
        );
        let r = run_task(&tasks[0], &e).await;
        assert!(r.landed && r.accepted, "{r:?}");
        assert!(
            r.usd > 0.0 && r.usd_by_role.contains_key("auditor"),
            "{r:?}"
        );
    }

    /// A task no model answered isn't a result: the crew was never measured.
    /// It says why, and a run published before this field existed reads as
    /// it always did.
    #[tokio::test]
    async fn a_task_no_model_answered_is_not_a_result() {
        let s = suite();
        let home = TempDir::new().unwrap();
        let tasks = load_suite(s.path()).unwrap();
        // The provider refuses every call: a key it doesn't know.
        struct Refuses;
        #[async_trait::async_trait]
        impl Provider for Refuses {
            async fn stream(
                &self,
                _req: crate::llm::CompletionRequest,
            ) -> Result<crate::llm::DeltaStream> {
                Err(Error::Provider("401: no such key".into()))
            }
            async fn list_models(&self) -> Result<Vec<crate::llm::ModelInfo>> {
                Ok(Vec::new())
            }
        }
        let mut e = env(home.path(), Vec::new());
        e.provider = Arc::new(Refuses);
        let r = run_task(&tasks[0], &e).await;
        assert!(r.unanswered() && r.calls == 0 && !r.landed, "{r:?}");
        assert!(
            r.outcome.starts_with("t1 — failed: ") && r.outcome.contains("key was refused"),
            "{}",
            r.outcome
        );
        // Every seat on one model: the crew won't start.
        let mut e = env(home.path(), Vec::new());
        e.cfg.specialists.clear();
        let r = run_task(&tasks[0], &e).await;
        assert!(r.unanswered(), "{r:?}");
        assert!(r.outcome.starts_with("builds paused: "), "{}", r.outcome);
        // A seat the account can't reach: the crew is paused before a call.
        let mut e = env(home.path(), Vec::new());
        e.provider = Arc::new(ReplayProvider::scripted(Vec::new()).refusing(&["auditor-model"]));
        let r = run_task(&tasks[0], &e).await;
        assert!(r.unanswered(), "{r:?}");
        assert!(r.outcome.starts_with("crew paused: "), "{}", r.outcome);
        // A rejected fix was answered, and is a result.
        let e = env(
            home.path(),
            vec![
                write("def add(a, b):\n    return a - b\n"),
                say("STATUS: DONE"),
            ],
        );
        let r = run_task(&tasks[0], &e).await;
        assert!(!r.landed && r.calls > 0 && !r.unanswered(), "{r:?}");
        // A result saved before `calls` was counted still loads.
        let old = r#"{"task":"t","landed":true,"accepted":true,"usd":0.1,"unpriced":false,
            "billable_tokens":9,"usd_by_role":{},"secs":1.0,"outcome":"t1 (merged)"}"#;
        let old: BenchResult = serde_json::from_str(old).unwrap();
        assert!(old.calls == 0 && !old.unanswered());
    }

    /// The case the benchmark exists for: the visible check and the auditor
    /// both pass a wrong fix, and only the hidden test catches it.
    #[tokio::test]
    async fn a_wrong_fix_the_auditor_passed_is_a_false_pass() {
        let s = suite();
        let home = TempDir::new().unwrap();
        let tasks = load_suite(s.path()).unwrap();
        let e = env(
            home.path(),
            vec![
                // Satisfies add(2, 2) == 4 and nothing else.
                write("def add(a, b):\n    return 4\n"),
                say("STATUS: DONE"),
                say("VERDICT: PASS"),
            ],
        );
        let r = run_task(&tasks[0], &e).await;
        assert!(r.landed, "{r:?}");
        assert!(!r.accepted, "{r:?}");
        let sum = Summary::of(&[r]);
        assert_eq!(sum.false_passes, 1);
        assert_eq!(sum.usd_per_accepted(), None);
        assert!(sum.render().contains("false passes 1"));
    }

    #[tokio::test]
    async fn a_rejected_fix_neither_lands_nor_counts() {
        let s = suite();
        let home = TempDir::new().unwrap();
        let tasks = load_suite(s.path()).unwrap();
        let e = env(
            home.path(),
            vec![
                write("def add(a, b):\n    return a - b\n"),
                say("STATUS: DONE"),
            ],
        );
        // add(2, 2) == 0: the visible check fails before any audit.
        let r = run_task(&tasks[0], &e).await;
        assert!(!r.landed && !r.accepted, "{r:?}");
    }

    fn write_file(path: &str, content: &str) -> Vec<StreamDelta> {
        vec![
            StreamDelta::ToolCall {
                id: "w".into(),
                name: "write".into(),
                arguments: serde_json::json!({"path": path, "content": content}).to_string(),
            },
            StreamDelta::Done,
        ]
    }

    /// A task split into two builder tasks, the second waiting on the
    /// first, runs both and lands once: the hidden test needs both files.
    #[tokio::test]
    async fn a_task_split_in_two_lands_once_both_are_built() {
        let d = TempDir::new().unwrap();
        let t = d.path().join("two-parts");
        std::fs::create_dir_all(t.join("repo")).unwrap();
        std::fs::create_dir_all(t.join("hidden")).unwrap();
        std::fs::write(
            t.join("task.toml"),
            r#"
title = "two parts"
brief = "a.py and b.py"
files = ["a.py", "b.py"]
checks = ["python3 -c 'import a'"]
accept = ["python3 hidden_check.py"]
needs = ["python3 --version"]

[[tasks]]
id = "first"
title = "a"
brief = "a.py: A = 1"
files = ["a.py"]

[[tasks]]
id = "second"
title = "b"
brief = "b.py: B = a.A + 1"
files = ["b.py"]
after = ["first"]
"#,
        )
        .unwrap();
        std::fs::write(t.join("repo/a.py"), "A = 0\n").unwrap();
        std::fs::write(
            t.join("hidden/hidden_check.py"),
            "import a, b\nassert (a.A, b.B) == (1, 2)\n",
        )
        .unwrap();
        let tasks = load_suite(d.path()).unwrap();
        if let Some(needs) = tasks[0].missing() {
            eprintln!("skipping: needs {needs}");
            return;
        }
        assert_eq!(tasks[0].queue_items().len(), 2);
        let home = TempDir::new().unwrap();
        let mut e = env(
            home.path(),
            vec![
                write_file("a.py", "A = 1\n"),
                say("STATUS: DONE"),
                say("VERDICT: PASS"),
                write_file("b.py", "import a\nB = a.A + 1\n"),
                say("STATUS: DONE"),
                say("VERDICT: PASS"),
            ],
        );
        // One at a time, so the replayed turns arrive in order.
        e.max_crew = 1;
        let r = run_task(&tasks[0], &e).await;
        assert!(r.landed && r.accepted, "{r:?}");
    }

    /// A task says what it needs, and is skipped where that is missing.
    #[test]
    fn a_task_is_skipped_where_its_tools_are_missing() {
        let s = suite();
        let mut tasks = load_suite(s.path()).unwrap();
        assert_eq!(tasks[0].missing(), None, "nothing needed");
        tasks[0].spec.needs = vec!["true".into(), "ryter-no-such-tool --version".into()];
        assert_eq!(
            tasks[0].missing().as_deref(),
            Some("ryter-no-such-tool --version")
        );
    }

    fn result(task: &str, landed: bool, accepted: bool, usd: f64) -> BenchResult {
        BenchResult {
            task: task.into(),
            landed,
            accepted,
            usd,
            unpriced: false,
            billable_tokens: 0,
            usd_by_role: Default::default(),
            secs: 30.0,
            outcome: String::new(),
            calls: 1,
        }
    }

    fn report(results: Vec<BenchResult>) -> Report {
        Report {
            ryter: "0.1.0".into(),
            date: "2026-01-01".into(),
            lead: "lead-model".into(),
            builder: "builder-model".into(),
            auditor: "auditor-model".into(),
            budget_usd: 1.0,
            results,
            skipped: vec![Skipped {
                task: "rust-task".into(),
                needs: "cargo --version".into(),
            }],
        }
    }

    /// The published page says who ran, the headline, and each task; the
    /// file beside it reads back as the same run.
    #[test]
    fn a_run_is_published_as_a_page_and_a_file() {
        let r = report(vec![
            result("a", true, true, 0.10),
            result("b", true, false, 0.30),
            result("c", false, false, 0.20),
        ]);
        let md = r.markdown();
        for part in [
            "lead `lead-model`, builder `builder-model`, auditor `auditor-model`",
            "| Landed (checks and the auditor passed) | 2 of 3 |",
            "| Accepted (the hidden tests passed too) | 1 of 3 |",
            "| False passes (landed, but wrong) | 1 |",
            "| Cost per accepted task | $0.600 |",
            "| `b` | 1 | 1 | 0 | $0.300 | 30 s |",
            "`rust-task`: this machine lacks `cargo --version`",
        ] {
            assert!(md.contains(part), "{part}\n{md}");
        }
        let back: Report = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back, r);
    }

    /// A release is checked against the last run: worse means a task
    /// accepted less often, or more false passes. Cost is shown, and never
    /// counts as worse.
    #[test]
    fn a_run_is_compared_with_the_last() {
        let then = report(vec![
            result("a", true, true, 0.10),
            result("b", true, true, 0.10),
        ]);
        // The same outcomes at ten times the cost, and a new task.
        let same = report(vec![
            result("a", true, true, 1.0),
            result("b", true, true, 1.0),
            result("c", false, false, 1.0),
        ]);
        let (lines, worse) = same.compare(&then);
        assert!(!worse, "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("c: new")), "{lines:?}");
        assert!(
            lines.iter().any(|l| l.contains("$0.100 then, $1.500 now")),
            "{lines:?}"
        );
        // `b` no longer accepted, and passed wrongly.
        let bad = report(vec![
            result("a", true, true, 0.10),
            result("b", true, false, 0.10),
        ]);
        let (lines, worse) = bad.compare(&then);
        assert!(worse);
        assert!(lines.iter().any(|l| l.contains("b: WORSE")), "{lines:?}");
        // Rates compare across --repeat: 1 of 2 is worse than 1 of 1.
        let twice = report(vec![
            result("a", true, true, 0.1),
            result("a", false, false, 0.1),
            result("b", true, true, 0.1),
            result("b", true, true, 0.1),
        ]);
        assert!(twice.compare(&then).1);
        let (lines, worse) = then.compare(&bad);
        assert!(
            !worse && lines.iter().any(|l| l.contains("b: better")),
            "{lines:?}"
        );
        // A task that ran then and not now is named, with why.
        let mut fewer = report(vec![result("a", true, true, 0.10)]);
        let (lines, _) = fewer.compare(&then);
        assert!(
            lines
                .iter()
                .any(|l| l.contains("b: not run this time (no longer in the suite)")),
            "{lines:?}"
        );
        fewer.skipped = vec![Skipped {
            task: "b".into(),
            needs: "cargo --version".into(),
        }];
        let (lines, _) = fewer.compare(&then);
        assert!(
            lines.iter().any(|l| l
                .contains("b: not run this time (skipped, this machine lacks `cargo --version`)")),
            "{lines:?}"
        );
    }

    /// The published run is replaced only by a run that can stand in for
    /// it. An empty run, a worse one, and one that skipped a published task
    /// all leave it as it was. A reviewer's case: `--repeat 0` published
    /// nothing over nine results and exited 0.
    #[test]
    fn the_published_run_is_replaced_only_by_one_as_good() {
        let dir = TempDir::new().unwrap();
        let stem = dir.path().join("docs/bench");
        let json = stem.with_extension("json");
        let good = report(vec![
            result("a", true, true, 0.10),
            result("b", true, true, 0.10),
        ]);
        // The first run publishes: nothing to compare with.
        let first = good.publish(&stem).unwrap();
        assert_eq!((first.lines.len(), first.kept), (0, None));
        let published = std::fs::read_to_string(&json).unwrap();
        assert!(stem.with_extension("md").exists());

        // No results: an error, and the published run untouched.
        let err = report(Vec::new()).publish(&stem).unwrap_err().to_string();
        assert!(err.contains("no task ran"), "{err}");
        assert_eq!(std::fs::read_to_string(&json).unwrap(), published);

        // A task no model answered: an error too. The reviewer's other
        // case, a crew that wouldn't start, exited 0.
        let mut dead = result("b", false, false, 0.0);
        dead.calls = 0;
        dead.outcome = "t1 — failed: provider: invalid api key".into();
        let err = report(vec![result("a", true, true, 0.10), dead])
            .publish(&stem)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("no model answered on b") && err.contains("invalid api key"),
            "{err}"
        );
        assert_eq!(std::fs::read_to_string(&json).unwrap(), published);

        // Worse: kept.
        let worse = report(vec![
            result("a", true, true, 0.10),
            result("b", false, false, 0.10),
        ]);
        let out = worse.publish(&stem).unwrap();
        assert!(
            out.kept.as_deref().is_some_and(|k| k.contains("worse")),
            "{out:?}"
        );
        assert_eq!(std::fs::read_to_string(&json).unwrap(), published);

        // A published task skipped here: kept, however well the rest did.
        let mut partial = report(vec![result("a", true, true, 0.10)]);
        partial.skipped = vec![Skipped {
            task: "b".into(),
            needs: "cargo --version".into(),
        }];
        let out = partial.publish(&stem).unwrap();
        assert!(
            out.kept
                .as_deref()
                .is_some_and(|k| k.contains("b of the published run")),
            "{out:?}"
        );
        assert_eq!(std::fs::read_to_string(&json).unwrap(), published);

        // As good, with a task gone from the suite and another added: replaced.
        let mut next = report(vec![
            result("a", true, true, 0.50),
            result("c", true, true, 0.50),
        ]);
        next.skipped.clear();
        next.ryter = "0.2.0".into();
        let out = next.publish(&stem).unwrap();
        assert_eq!(out.kept, None, "{out:?}");
        assert!(
            out.lines
                .iter()
                .any(|l| l.contains("b: not run this time (no longer in the suite)"))
        );
        let now: Report = serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
        assert_eq!(now.ryter, "0.2.0");
        let mut left: Vec<_> = std::fs::read_dir(stem.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["bench.json", "bench.md"], "no temporary file left");

        // A file there that isn't a published run is never overwritten.
        std::fs::write(&json, "not json").unwrap();
        assert!(good.publish(&stem).is_err());
        assert_eq!(std::fs::read_to_string(&json).unwrap(), "not json");
    }

    /// Every shipped task is sound: the work is not already done (the hidden
    /// tests fail on the fixture), and the reference solution passes both the
    /// gate's checks and the hidden tests. Without this, a benchmark number
    /// measures the suite, not the crew. A task whose tools aren't installed
    /// here is skipped, and said to be.
    #[test]
    fn the_suite_is_sound() {
        let suite = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench");
        let tasks = load_suite(&suite).unwrap();
        assert!(tasks.len() >= 9, "tasks went missing: {}", tasks.len());
        // Real work: several files, more than one language, and a split task.
        assert!(tasks.iter().filter(|t| t.spec.files.len() >= 3).count() >= 3);
        assert!(tasks.iter().any(|t| t.spec.tasks.len() >= 3));
        for need in ["python3", "cargo", "node"] {
            assert!(
                tasks
                    .iter()
                    .any(|t| t.spec.needs.iter().any(|n| n.starts_with(need))),
                "no task needs {need}"
            );
        }
        let cancel = crate::cancel::Cancel::new();
        let run = |dir: &Path, cmds: &[String]| {
            cmds.iter().all(|c| {
                matches!(
                    crate::tools::shell::run_command(c, dir, Duration::from_secs(300), &cancel),
                    Ok(crate::tools::shell::Run::Ok(_))
                )
            })
        };
        let mut checked = 0;
        for t in &tasks {
            assert!(!t.spec.needs.is_empty(), "{}: says what it needs", t.name);
            assert!(
                !t.spec.brief.trim().is_empty() && !t.spec.files.is_empty(),
                "{}",
                t.name
            );
            // A split task's parts cover its files, and wait only on each other.
            for part in &t.spec.tasks {
                assert!(
                    part.files.iter().all(|f| t.spec.files.contains(f)),
                    "{}",
                    t.name
                );
                assert!(
                    part.after
                        .iter()
                        .all(|a| t.spec.tasks.iter().any(|p| &p.id == a)),
                    "{}: {} waits on a task that isn't there",
                    t.name,
                    part.id
                );
            }
            if let Some(needs) = t.missing() {
                eprintln!("{}: skipped, this machine lacks `{needs}`", t.name);
                continue;
            }
            checked += 1;
            let before = TempDir::new().unwrap();
            copy_dir(&t.dir.join("repo"), before.path()).unwrap();
            copy_dir(&t.dir.join("hidden"), before.path()).unwrap();
            assert!(
                !run(before.path(), &t.spec.accept),
                "{}: the hidden tests already pass on the fixture",
                t.name
            );
            let after = TempDir::new().unwrap();
            copy_dir(&t.dir.join("repo"), after.path()).unwrap();
            copy_dir(&t.dir.join("solution"), after.path()).unwrap();
            assert!(
                run(after.path(), &t.spec.checks),
                "{}: solution fails the checks",
                t.name
            );
            copy_dir(&t.dir.join("hidden"), after.path()).unwrap();
            assert!(
                run(after.path(), &t.spec.accept),
                "{}: solution fails the hidden tests",
                t.name
            );
        }
        eprintln!("{checked} of {} tasks checked", tasks.len());
    }

    #[test]
    fn cost_per_accepted_task_is_the_headline() {
        let r = |accepted, usd| BenchResult {
            task: "t".into(),
            landed: true,
            accepted,
            usd,
            unpriced: false,
            billable_tokens: 0,
            usd_by_role: Default::default(),
            secs: 0.0,
            outcome: String::new(),
            calls: 1,
        };
        let s = Summary::of(&[r(true, 0.10), r(true, 0.20), r(false, 0.30)]);
        assert_eq!(s.accepted, 2);
        assert_eq!(s.false_passes, 1);
        assert!(
            (s.usd_per_accepted().unwrap() - 0.30).abs() < 1e-9,
            "failures are paid for too"
        );
    }
}
