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
//! bench/<task>/task.toml   title, brief, files, checks, accept
//! bench/<task>/repo/       the fixture the crew works in
//! bench/<task>/hidden/     copied in only for acceptance, after landing
//! ```

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
    /// First line of the crew report, or the error.
    pub outcome: String,
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
        .apply_todo(&serde_json::json!({"items": [{
            "id": "t1",
            "title": task.spec.title,
            "brief": task.spec.brief,
            "files": task.spec.files,
        }]}))?;
    let mut agent = Agent {
        provider: env.provider.clone(),
        book: PriceBook::from_config(&env.cfg),
        ctx: ToolContext {
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
            sticky_approve: Arc::new(std::sync::atomic::AtomicBool::new(false)),
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
        max_crew: 1,
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
    result.outcome = report
        .lines()
        .find(|l| l.starts_with("### "))
        .map(|l| l.trim_start_matches("### ").to_string())
        .unwrap_or_else(|| report.lines().next().unwrap_or("").to_string());
    result.landed = crate::git::head(&work)? != before && agent.session.meta.patch.is_none();
    if !result.landed {
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

    /// Every shipped task is sound: the work is not already done (the hidden
    /// tests fail on the fixture), and the reference solution passes both the
    /// gate's checks and the hidden tests. Without this, a benchmark number
    /// measures the suite, not the crew.
    #[test]
    fn the_starter_suite_is_sound() {
        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("python3 not found; skipping");
            return;
        }
        let suite = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench");
        let tasks = load_suite(&suite).unwrap();
        assert!(tasks.len() >= 4, "starter suite went missing");
        let cancel = crate::cancel::Cancel::new();
        let run = |dir: &Path, cmds: &[String]| {
            cmds.iter().all(|c| {
                matches!(
                    crate::tools::shell::run_command(c, dir, Duration::from_secs(60), &cancel),
                    Ok(crate::tools::shell::Run::Ok(_))
                )
            })
        };
        for t in &tasks {
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
            assert!(
                !t.spec.brief.trim().is_empty() && !t.spec.files.is_empty(),
                "{}",
                t.name
            );
        }
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
