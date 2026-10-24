//! The runner: each task to the build hat in a fresh copy, then the hidden
//! tests, under one budget.
//!
//! The build hat runs as its own `ryter` process, the way the comparison
//! runner starts it: `--json --hat build --always-approve`, the user's
//! environment for its login, a per-task cap in the copy's `.ryter/config.toml`.
//! The fixture's git and the scoring run in a bare environment (a private
//! `HOME`, no git config, an offline Cargo), so nothing in the user's shell
//! can pass or fail a tree for the model.
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::report::{Check, Report, TaskResult};
use crate::task::{File, Task};

/// The sandbox a run defaults to: Landlock's `workspace` where the kernel
/// has it, `off` elsewhere, including a Linux without Landlock.
pub fn default_sandbox() -> &'static str {
    if cfg!(target_os = "linux") && ryter_core::sandbox::probe() == "available" {
        "workspace"
    } else {
        "off"
    }
}

/// `parent/name`, or the first of `name-2`, `name-3`… that does not exist
/// yet: two runs started in the same minute keep their own folders.
pub fn fresh_dir(parent: &Path, name: &str) -> PathBuf {
    let first = parent.join(name);
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| parent.join(format!("{name}-{n}")))
        .find(|p| !p.exists())
        .expect("an unused folder name")
}

/// The `ryter` to run the tasks with: this process's own binary, as long as
/// it is still there. A rebuild while ryter runs replaces the file, and the
/// bench would otherwise fail every task with "No such file or directory".
pub fn own_binary() -> Result<PathBuf, String> {
    let binary =
        std::env::current_exe().map_err(|e| format!("can't find the ryter binary: {e}"))?;
    check_binary(&binary)?;
    Ok(binary)
}

/// `Err` with the message for the user when `binary` is gone.
pub fn check_binary(binary: &Path) -> Result<(), String> {
    if binary.is_file() {
        return Ok(());
    }
    Err(format!(
        "the ryter binary at {} is gone, replaced or rebuilt since this ryter started: restart ryter and run the bench again",
        binary.display()
    ))
}

/// Kept back from the budget for the last task's overshoot.
pub const RESERVE_USD: f64 = 0.10;
/// A cap is written with two decimals, and `0` means no cap to Ryter: a
/// task starts only with a cent or more to spend.
pub const MIN_CAP_USD: f64 = 0.01;
/// How long one hidden-test command may run.
const CHECK_TIMEOUT: Duration = Duration::from_secs(120);
/// How much of a check's output is kept.
const OUTPUT_TAIL: usize = 8_000;

/// How a run is made.
#[derive(Clone, Debug)]
pub struct Options {
    /// The `ryter` binary to run the build hat with: this one.
    pub binary: PathBuf,
    pub connection: String,
    pub model: String,
    /// Dollars for the whole run: for each model's, when several are compared.
    pub budget_usd: f64,
    /// Per task.
    pub timeout: Duration,
    /// The `--sandbox` the build hat gets: `workspace` on Linux, `off` elsewhere.
    pub sandbox: String,
    /// The run's directory: a folder per task with its logs; the project
    /// copies are removed once scored.
    pub dir: PathBuf,
    pub cancel: Cancel,
}

/// A flag the caller raises to end the run at the next check.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// What the runner reports as it goes.
#[derive(Clone, Debug, PartialEq)]
pub enum Progress {
    /// A model's run is starting: the `index`th of `total` compared.
    Model {
        connection: String,
        model: String,
        index: usize,
        total: usize,
    },
    /// A task's build hat is starting, under this cap.
    Started {
        name: String,
        index: usize,
        total: usize,
        cap_usd: f64,
    },
    /// What the model is doing: a tool call's summary.
    Activity { name: String, what: String },
    /// The task's cost so far.
    Spent { name: String, usd: f64 },
    /// The hidden tests are running.
    Scoring { name: String },
    /// A task is done, scored or skipped.
    Finished(TaskResult),
    /// The run ended early.
    Stopped { why: String },
}

/// What one task may cost now, in whole cents: zero when the next task
/// must not start.
pub fn allowance(budget_usd: f64, spent_usd: f64) -> f64 {
    let cents = ((budget_usd - spent_usd - RESERVE_USD) * 100.0 * 1e6).round() / 1e6;
    (cents.floor() / 100.0).max(0.0)
}

/// The run: tasks in order, one attempt each, stopping when the budget's
/// reserve is reached, when a task goes unpriced, or when cancelled.
/// `on` hears each step; the report is also saved into `opts.dir` after
/// every task.
pub fn run(tasks: &[&Task], opts: &Options, mut on: impl FnMut(Progress)) -> Report {
    let level = match tasks.iter().map(|t| t.level).collect::<Vec<_>>().as_slice() {
        [] => None,
        [first, rest @ ..] if rest.iter().all(|l| l == first) => Some(*first),
        _ => None,
    };
    let mut report = Report {
        date: ryter_core::clock::today(),
        ryter: ryter_core::VERSION.to_string(),
        connection: opts.connection.clone(),
        model: opts.model.clone(),
        level,
        budget_usd: opts.budget_usd,
        timeout_seconds: opts.timeout.as_secs(),
        sandbox: opts.sandbox.clone(),
        total_usd: 0.0,
        results: Vec::new(),
        stopped: None,
        directory: opts.dir.clone(),
        save_error: None,
    };
    let _ = fs::create_dir_all(&opts.dir);
    let total = tasks.len();
    for (index, task) in tasks.iter().enumerate() {
        if opts.cancel.is_cancelled() {
            report.stopped = Some(format!("cancelled before {}", task.name));
            break;
        }
        let cap = allowance(opts.budget_usd, report.total_usd);
        if cap < MIN_CAP_USD {
            report.stopped = Some(format!("budget reserve reached before {}", task.name));
            break;
        }
        let base = opts.dir.join(&task.name);
        let mut started = false;
        let result = match needs_met(task, &base) {
            Err(why) => TaskResult::skipped(&task.name, &task.title, task.level, why),
            Ok(()) => {
                started = true;
                on(Progress::Started {
                    name: task.name.clone(),
                    index,
                    total,
                    cap_usd: cap,
                });
                execute(task, opts, &base, cap, &mut on)
            }
        };
        let _ = fs::remove_dir_all(base.join("project"));
        let _ = fs::remove_dir_all(base.join("fixture"));
        let _ = fs::remove_dir_all(base.join("score-home"));
        if let Some(usd) = result.usd {
            report.total_usd += usd;
        }
        let unpriced = result.solved.is_some() && result.usd.is_none();
        let cancelled = result.cancelled;
        // Started but never scored: the next task would go the same way.
        let broken = started && result.solved.is_none();
        let why = result.error.clone().unwrap_or_default();
        report.results.push(result.clone());
        if let Err(e) = report.save(&opts.dir) {
            report.save_error = Some(e.to_string());
        }
        on(Progress::Finished(result));
        if cancelled {
            report.stopped = Some(format!("cancelled during {}", task.name));
            break;
        }
        if broken {
            report.stopped = Some(format!("{}: {why}", task.name));
            break;
        }
        if unpriced {
            report.stopped = Some(format!(
                "{} went unpriced; reconcile before more paid calls",
                task.name
            ));
            break;
        }
    }
    if let Some(why) = &report.stopped {
        on(Progress::Stopped { why: why.clone() });
    }
    // The last save is the one that counts: a report that could not be
    // written is said so, not announced as a file.
    match report.save(&opts.dir) {
        Ok(_) => report.save_error = None,
        Err(e) => report.save_error = Some(e.to_string()),
    }
    report
}

/// Whether the task's `needs` succeed here: run from a copy of the fixture,
/// as the Python runner runs them, in the bare environment.
fn needs_met(task: &Task, base: &Path) -> Result<(), String> {
    if task.needs.is_empty() {
        return Ok(());
    }
    let fixture = base.join("fixture");
    materialize(&task.repo, &fixture.join("repo"))
        .map_err(|e| format!("could not copy the fixture: {e}"))?;
    let home = base.join("score-home");
    fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    let env = isolated_env(&home);
    for need in &task.needs {
        let check = sh(&fixture, need, &env, CHECK_TIMEOUT);
        if check.exit != Some(0) {
            let tail: String = check
                .output
                .lines()
                .last()
                .unwrap_or("")
                .chars()
                .take(120)
                .collect();
            return Err(format!(
                "needs `{need}`{}",
                if tail.is_empty() {
                    String::new()
                } else {
                    format!(": {tail}")
                }
            ));
        }
    }
    Ok(())
}

fn execute(
    task: &Task,
    opts: &Options,
    base: &Path,
    cap_usd: f64,
    on: &mut impl FnMut(Progress),
) -> TaskResult {
    let mut result = TaskResult::skipped(&task.name, &task.title, task.level, String::new());
    result.error = None;
    let project = base.join("project");
    if let Err(why) = prepare(task, base, cap_usd).and_then(|()| {
        ryter_core::config::trust(&project).map_err(|e| format!("could not trust the copy: {e}"))
    }) {
        result.error = Some(why);
        return result;
    }
    let home = base.join("score-home");
    let env = isolated_env(&home);
    let prompt = task.prompt();
    let _ = fs::write(base.join("prompt.txt"), &prompt);
    let mut cmd = Command::new(&opts.binary);
    cmd.args([
        "--json",
        "--hat",
        "build",
        "--always-approve",
        "--no-remember",
        "--sandbox",
        &opts.sandbox,
    ])
    .args(["--connection", &opts.connection, "--model", &opts.model])
    .args(["-p", &prompt])
    .current_dir(&project)
    // Every tool runs in the copy; one of them reads `PWD` first.
    .env("PWD", &project);
    let started = Instant::now();
    let mut out_log = fs::File::create(base.join("stdout.ndjson")).ok();
    let mut err_log = fs::File::create(base.join("stderr.txt")).ok();
    let name = task.name.clone();
    let mut usd_known = true;
    let mut usd = 0.0;
    let mut last_err = String::new();
    let outcome = spawn_and_wait(cmd, opts.timeout, &opts.cancel, |line| match line {
        Line::Err(text) => {
            if let Some(f) = err_log.as_mut() {
                let _ = writeln!(f, "{text}");
            }
            if !text.trim().is_empty() {
                last_err = text;
            }
        }
        Line::Out(text) => {
            if let Some(f) = out_log.as_mut() {
                let _ = writeln!(f, "{text}");
            }
            let Ok(event) = serde_json::from_str::<serde_json::Value>(&text) else {
                return;
            };
            match event.get("kind").and_then(|k| k.as_str()) {
                Some("spend") => {
                    result.calls += 1;
                    result.input_tokens += event["input_tokens"].as_u64().unwrap_or(0);
                    result.cached_tokens += event["cached_tokens"].as_u64().unwrap_or(0);
                    result.output_tokens += event["output_tokens"].as_u64().unwrap_or(0);
                    match event["total_usd"].as_f64() {
                        Some(v) if !event["incomplete"].as_bool().unwrap_or(false) => usd += v,
                        _ => usd_known = false,
                    }
                    if usd_known {
                        on(Progress::Spent {
                            name: name.clone(),
                            usd,
                        });
                    }
                }
                Some("tool_call") => {
                    let what = event["summary"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .or_else(|| event["name"].as_str())
                        .unwrap_or("")
                        .to_string();
                    on(Progress::Activity {
                        name: name.clone(),
                        what,
                    });
                }
                _ => {}
            }
        }
    });
    result.seconds = started.elapsed().as_secs_f64();
    result.exit = outcome.status;
    result.timed_out = outcome.timed_out;
    result.cancelled = outcome.cancelled;
    result.complete = usd_known;
    result.usd = (usd_known && result.calls > 0).then_some(usd);
    let _ = ryter_core::config::untrust(&project);
    if let Some(why) = outcome.error {
        result.error = Some(why);
        return result;
    }
    if outcome.cancelled {
        // Cut short by the user: nothing to score, and no hidden test is
        // dropped into a tree a tool may still be writing.
        result.error = Some("cancelled".into());
        return result;
    }
    if result.calls == 0 && outcome.status != Some(0) && !outcome.timed_out {
        // Nothing was asked of the model: the build hat refused to start.
        result.error = Some(format!(
            "ryter did not start: {}",
            if last_err.is_empty() {
                format!("exit {:?}", outcome.status)
            } else {
                last_err
            }
        ));
        return result;
    }
    on(Progress::Scoring { name: name.clone() });
    // The files the model left changed, then the hidden tests, copied in
    // only now and run under the sandbox the build hat had: they import
    // and build what the model wrote.
    match score(task, &project, &home, &opts.sandbox, &env) {
        Ok((changed, acceptance)) => {
            result.changed = changed;
            result.acceptance = acceptance;
            result.solved = Some(
                !result.acceptance.is_empty()
                    && result.acceptance.iter().all(|c| c.exit == Some(0)),
            );
        }
        Err(why) => result.error = Some(why),
    }
    result
}

/// The scoring, on a thread of its own so the sandbox binds it and what
/// it starts, and nothing else: the files changed, then the hidden tests.
fn score(
    task: &Task,
    project: &Path,
    home: &Path,
    sandbox: &str,
    env: &HashMap<String, String>,
) -> Result<(Vec<String>, Vec<Check>), String> {
    std::thread::scope(|s| {
        s.spawn(|| {
            if sandbox != "off" {
                let profile = sandbox
                    .parse::<ryter_core::sandbox::SandboxProfile>()
                    .map_err(|e| format!("could not sandbox the scoring: {e}"))?;
                ryter_core::sandbox::apply_with(
                    profile,
                    project,
                    &ryter_core::config::home_dir(),
                    &[home.to_path_buf()],
                )
                .map_err(|e| format!("could not sandbox the scoring: {e}"))?;
            }
            let status = sh(project, "git status --porcelain", env, CHECK_TIMEOUT);
            let changed = status
                .output
                .lines()
                .filter(|l| l.len() > 3)
                .map(|l| l[3..].to_string())
                .filter(|p| !p.starts_with(".ryter/") && !p.contains("__pycache__"))
                .collect();
            materialize_hidden(&task.hidden, project)?;
            let acceptance = task
                .accept
                .iter()
                .map(|c| sh(project, c, env, CHECK_TIMEOUT))
                .collect();
            Ok((changed, acceptance))
        })
        .join()
        .map_err(|_| "the scoring thread panicked".to_string())?
    })
}

/// The hidden tests into a tree the model has had: through no link of
/// its making, over no file of its making. A link where a test or one of
/// its folders should be is a tampered tree, and the task is not scored.
fn materialize_hidden(files: &[File], project: &Path) -> Result<(), String> {
    for file in files {
        let rel = Path::new(&file.path);
        let mut here = project.to_path_buf();
        for part in rel.components() {
            here.push(part);
            if here
                .symlink_metadata()
                .is_ok_and(|m| m.file_type().is_symlink())
            {
                return Err(format!(
                    "could not copy the hidden tests: {} is a link",
                    here.strip_prefix(project).unwrap_or(&here).display()
                ));
            }
        }
        let path = project.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("could not copy the hidden tests: {e}"))?;
        }
        match path.symlink_metadata() {
            Ok(m) if m.is_dir() => {
                return Err(format!(
                    "could not copy the hidden tests: {} is a folder",
                    file.path
                ));
            }
            Ok(_) => fs::remove_file(&path)
                .map_err(|e| format!("could not copy the hidden tests: {e}"))?,
            Err(_) => {}
        }
        let bytes = file.contents();
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .and_then(|mut f| f.write_all(&bytes))
            .map_err(|e| format!("could not copy the hidden tests: {e}"))?;
        #[cfg(unix)]
        if file.executable {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                .map_err(|e| format!("could not copy the hidden tests: {e}"))?;
        }
    }
    Ok(())
}

/// The fresh copy: the project, its baseline commit, the run file with the
/// task's checks and the cap. The caller trusts the folder.
fn prepare(task: &Task, base: &Path, cap_usd: f64) -> Result<(), String> {
    let project = base.join("project");
    let _ = fs::remove_dir_all(&project);
    materialize(&task.repo, &project).map_err(|e| format!("could not copy the project: {e}"))?;
    let home = base.join("score-home");
    fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    let env = isolated_env(&home);
    for step in [
        "git init -q -b main",
        "git config user.name \"Ryter bench\"",
        "git config user.email bench@localhost",
        "git config commit.gpgsign false",
        "git add -A && git commit -qm baseline",
    ] {
        let check = sh(&project, step, &env, CHECK_TIMEOUT);
        if check.exit != Some(0) {
            return Err(format!(
                "fixture git initialization failed at `{step}`: {}",
                check.output.trim()
            ));
        }
    }
    let ryter = project.join(".ryter");
    fs::create_dir_all(&ryter).map_err(|e| e.to_string())?;
    let run_file = format!(
        "test = {}\ncheck = {}\n",
        toml::Value::Array(
            task.checks
                .iter()
                .cloned()
                .map(toml::Value::String)
                .collect()
        ),
        toml::Value::String(task.checks.first().cloned().unwrap_or_default())
    );
    fs::write(ryter.join("run.toml"), run_file).map_err(|e| e.to_string())?;
    let cap = cap_text(cap_usd)?;
    let config = ryter.join("config.toml");
    fs::write(
        &config,
        format!("[spend]\nsession_budget_usd = {cap}\n[update]\nmode = \"off\"\n"),
    )
    .map_err(|e| e.to_string())?;
    // Ryter refuses a project config anyone can read once a connection holds
    // its key in a file, whatever the project config itself says.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A cap as the config takes it, two decimals, never `0.00`.
fn cap_text(cap_usd: f64) -> Result<String, String> {
    let text = format!("{cap_usd:.2}");
    if text.parse::<f64>().unwrap_or(0.0) <= 0.0 {
        return Err(format!(
            "a cap of ${cap_usd} would be written as 0, which turns the cap off"
        ));
    }
    Ok(text)
}

/// Write the files under `dir`, with their executable bits.
fn materialize(files: &[File], dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    for file in files {
        let path = dir.join(&file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, file.contents())?;
        #[cfg(unix)]
        if file.executable {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(())
}

/// The bare environment the scoring runs in: the user's `PATH` and toolchain
/// homes, a private `HOME`, no git config, an offline Cargo.
pub fn isolated_env(home: &Path) -> HashMap<String, String> {
    let user_home = std::env::var("HOME").unwrap_or_default();
    let mut env = HashMap::new();
    env.insert("PATH".into(), std::env::var("PATH").unwrap_or_default());
    env.insert("HOME".into(), home.to_string_lossy().into_owned());
    env.insert("LANG".into(), "C.UTF-8".into());
    env.insert(
        "GIT_CONFIG_GLOBAL".into(),
        if cfg!(windows) {
            "NUL".into()
        } else {
            "/dev/null".into()
        },
    );
    env.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
    env.insert(
        "RUSTUP_HOME".into(),
        std::env::var("RUSTUP_HOME").unwrap_or_else(|_| format!("{user_home}/.rustup")),
    );
    env.insert(
        "CARGO_HOME".into(),
        home.join("cargo").to_string_lossy().into_owned(),
    );
    env.insert("CARGO_NET_OFFLINE".into(), "true".into());
    env
}

/// Run `sh -c command` in `dir` with `env`, for at most `timeout`.
fn sh(dir: &Path, command: &str, env: &HashMap<String, String>, timeout: Duration) -> Check {
    let mut cmd = Command::new("sh");
    cmd.args(["-c", command])
        .current_dir(dir)
        .env_clear()
        .envs(env);
    let mut output = String::new();
    let outcome = spawn_and_wait(cmd, timeout, &Cancel::default(), |line| {
        let (Line::Out(text) | Line::Err(text)) = line;
        output.push_str(&text);
        output.push('\n');
    });
    if let Some(e) = outcome.error {
        output.push_str(&e);
    }
    if outcome.timed_out {
        output.push_str("timed out\n");
    }
    let tail = output.chars().count().saturating_sub(OUTPUT_TAIL);
    Check {
        command: command.to_string(),
        exit: if outcome.timed_out {
            None
        } else {
            outcome.status
        },
        output: output.chars().skip(tail).collect(),
    }
}

enum Line {
    Out(String),
    Err(String),
}

struct Outcome {
    status: Option<i32>,
    timed_out: bool,
    cancelled: bool,
    error: Option<String>,
}

/// Start `cmd` in its own process group, feed its output lines to `each`
/// as they come, and end the whole group when it runs out of time or the
/// run is cancelled.
fn spawn_and_wait(
    mut cmd: Command,
    timeout: Duration,
    cancel: &Cancel,
    mut each: impl FnMut(Line),
) -> Outcome {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Outcome {
                status: None,
                timed_out: false,
                cancelled: false,
                error: Some(format!("could not start: {e}")),
            };
        }
    };
    let (tx, rx) = mpsc::channel();
    let mut readers = Vec::new();
    if let Some(out) = child.stdout.take() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if tx.send(Line::Out(line)).is_err() {
                    break;
                }
            }
        }));
    }
    if let Some(err) = child.stderr.take() {
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                if tx.send(Line::Err(line)).is_err() {
                    break;
                }
            }
        }));
    }
    let started = Instant::now();
    let mut timed_out = false;
    let mut cancelled = false;
    let status = loop {
        for line in rx.try_iter() {
            each(line);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) => {}
            Err(_) => break None,
        }
        if cancel.is_cancelled() {
            cancelled = true;
            end_group(&mut child);
            break None;
        }
        if started.elapsed() >= timeout {
            timed_out = true;
            end_group(&mut child);
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    for reader in readers {
        let _ = reader.join();
    }
    for line in rx.try_iter() {
        each(line);
    }
    Outcome {
        status,
        timed_out,
        cancelled,
        error: None,
    }
}

/// End a process that ran out of time, and everything it started: its own
/// group, then every descendant, since a shell tool runs in a group of its
/// own and a killed hat cannot end it.
fn end_group(child: &mut Child) {
    #[cfg(unix)]
    {
        // Listed before the hat dies: once it has, its children are
        // nobody's and the tree is gone.
        let others = descendants(child.id());
        let group = rustix::process::Pid::from_child(child);
        let _ = rustix::process::kill_process_group(group, rustix::process::Signal::TERM);
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut reaped = false;
        while Instant::now() < deadline {
            if let Ok(Some(_)) = child.try_wait() {
                reaped = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if !reaped {
            let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
        }
        for pid in others.into_iter().chain(descendants(child.id())) {
            if let Some(pid) = rustix::process::Pid::from_raw(pid as i32) {
                let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
                let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Every process under `root`, by `ps`: the same on Linux and macOS.
#[cfg(unix)]
fn descendants(root: u32) -> Vec<u32> {
    let Ok(out) = Command::new("ps").args(["-eo", "pid=,ppid="]).output() else {
        return Vec::new();
    };
    let pairs: Vec<(u32, u32)> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
        })
        .collect();
    let mut found = vec![root];
    let mut i = 0;
    while i < found.len() {
        let parent = found[i];
        for (pid, ppid) in &pairs {
            if *ppid == parent && !found.contains(pid) {
                found.push(*pid);
            }
        }
        i += 1;
    }
    found.remove(0);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_binary_that_is_gone_is_named_with_the_remedy() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("ryter (deleted)");
        let msg = check_binary(&gone).unwrap_err();
        assert!(
            msg.contains("ryter (deleted)") && msg.contains("restart ryter"),
            "{msg}"
        );
        let here = dir.path().join("ryter");
        std::fs::write(&here, b"").unwrap();
        assert_eq!(check_binary(&here), Ok(()));
        assert!(own_binary().is_ok());
    }

    #[test]
    fn a_fresh_dir_is_numbered_past_one_that_exists() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(fresh_dir(dir.path(), "run"), dir.path().join("run"));
        fs::create_dir(dir.path().join("run")).unwrap();
        assert_eq!(fresh_dir(dir.path(), "run"), dir.path().join("run-2"));
        fs::create_dir(dir.path().join("run-2")).unwrap();
        assert_eq!(fresh_dir(dir.path(), "run"), dir.path().join("run-3"));
        assert!(["workspace", "off"].contains(&default_sandbox()));
    }

    /// A tree the model left a link in gets no hidden test through it:
    /// the link's target is untouched and the task is not scored.
    #[cfg(unix)]
    #[test]
    fn hidden_tests_go_through_no_link_and_over_no_folder() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        let outside = dir.path().join("outside.txt");
        fs::create_dir_all(project.join("tests")).unwrap();
        fs::write(&outside, "mine").unwrap();
        let files = [File {
            path: "tests/hidden.rs".into(),
            bytes: b"fn t() {}\n",
            executable: false,
            encoded: false,
        }];
        // The file itself a link.
        std::os::unix::fs::symlink(&outside, project.join("tests/hidden.rs")).unwrap();
        let why = materialize_hidden(&files, &project).unwrap_err();
        assert!(why.contains("tests/hidden.rs is a link"), "{why}");
        assert_eq!(fs::read_to_string(&outside).unwrap(), "mine");
        // Its folder a link.
        fs::remove_file(project.join("tests/hidden.rs")).unwrap();
        fs::remove_dir(project.join("tests")).unwrap();
        std::os::unix::fs::symlink(dir.path(), project.join("tests")).unwrap();
        let why = materialize_hidden(&files, &project).unwrap_err();
        assert!(why.contains("tests is a link"), "{why}");
        assert!(!dir.path().join("hidden.rs").exists());
        // A plain file of the model's in the way is replaced; a folder is not.
        fs::remove_file(project.join("tests")).unwrap();
        fs::create_dir_all(project.join("tests")).unwrap();
        fs::write(project.join("tests/hidden.rs"), "theirs").unwrap();
        materialize_hidden(&files, &project).unwrap();
        assert_eq!(
            fs::read_to_string(project.join("tests/hidden.rs")).unwrap(),
            "fn t() {}\n"
        );
        fs::remove_file(project.join("tests/hidden.rs")).unwrap();
        fs::create_dir(project.join("tests/hidden.rs")).unwrap();
        let why = materialize_hidden(&files, &project).unwrap_err();
        assert!(why.contains("is a folder"), "{why}");
    }

    /// A command that leaves a child in a group of its own still ends with
    /// its group: the descendants are listed before the kill and ended after.
    #[cfg(unix)]
    #[test]
    fn ending_a_group_ends_what_it_started_in_other_groups() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("pid");
        // A grandchild in a session of its own: `setsid` where it is (Linux),
        // Python's `os.setsid` elsewhere (macOS has no `setsid` command).
        let has = |program: &str, arg: &str| {
            Command::new(program)
                .arg(arg)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        };
        let detached = if has("setsid", "true") {
            format!("setsid sh -c 'echo $$ > {}; sleep 300'", marker.display())
        } else if has("python3", "--version") {
            format!(
                "python3 -c 'import os, time; os.setsid(); open({:?}, \"w\").write(str(os.getpid())); time.sleep(300)'",
                marker.display().to_string()
            )
        } else {
            eprintln!("neither setsid nor python3 here; the group test has nothing to detach with");
            return;
        };
        let mut cmd = Command::new("sh");
        cmd.args(["-c", &format!("{detached} & wait")])
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default());
        let outcome = spawn_and_wait(cmd, Duration::from_millis(700), &Cancel::default(), |_| {});
        assert!(outcome.timed_out);
        let pid: i32 = fs::read_to_string(&marker)
            .unwrap_or_default()
            .trim()
            .parse()
            .expect("the grandchild wrote its pid");
        let deadline = Instant::now() + Duration::from_secs(5);
        let alive = |pid: i32| {
            Path::new(&format!("/proc/{pid}")).exists()
                || Command::new("kill")
                    .args(["-0", &pid.to_string()])
                    .status()
                    .is_ok_and(|s| s.success())
        };
        while Instant::now() < deadline && alive(pid) {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(!alive(pid), "the sleep in its own session survived");
    }

    #[test]
    fn the_allowance_is_whole_cents_past_the_reserve() {
        assert!((allowance(5.0, 0.0) - 4.9).abs() < 1e-9);
        assert!(
            (allowance(0.11, 0.0) - 0.01).abs() < 1e-9,
            "{}",
            allowance(0.11, 0.0)
        );
        assert!((allowance(10.0, 9.89) - 0.01).abs() < 1e-9);
        assert_eq!(allowance(10.0, 9.894), 0.0);
        assert_eq!(allowance(5.0, 6.0), 0.0);
        assert_eq!(cap_text(0.5).unwrap(), "0.50");
        assert!(cap_text(0.004).is_err());
    }

    #[test]
    fn the_isolated_env_has_no_user_config_in_it() {
        let dir = tempfile::tempdir().unwrap();
        let env = isolated_env(dir.path());
        assert_eq!(env["HOME"], dir.path().to_string_lossy());
        assert_eq!(env["GIT_CONFIG_NOSYSTEM"], "1");
        assert_eq!(env["CARGO_NET_OFFLINE"], "true");
        assert!(env["CARGO_HOME"].starts_with(&*dir.path().to_string_lossy()));
        assert!(!env.contains_key("PYTHONPATH") && !env.contains_key("RUSTFLAGS"));
    }

    #[test]
    fn sh_captures_output_and_ends_a_command_that_runs_long() {
        let dir = tempfile::tempdir().unwrap();
        let env = isolated_env(dir.path());
        let ok = sh(
            dir.path(),
            "echo one; echo two >&2; exit 3",
            &env,
            Duration::from_secs(5),
        );
        assert_eq!(ok.exit, Some(3));
        assert!(
            ok.output.contains("one") && ok.output.contains("two"),
            "{}",
            ok.output
        );
        let started = Instant::now();
        let slow = sh(dir.path(), "sleep 30", &env, Duration::from_millis(300));
        assert_eq!(slow.exit, None);
        assert!(slow.output.contains("timed out"));
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the group was ended"
        );
    }

    #[test]
    fn a_task_is_materialized_with_its_files_and_executable_bits() {
        let dir = tempfile::tempdir().unwrap();
        let task = crate::task::select(&["runner-script".into()], None)[0];
        materialize(&task.repo, dir.path()).unwrap();
        assert!(dir.path().join("dev").is_file() && dir.path().join("src").is_dir());
        assert!(!dir.path().join("hidden").exists());
        let files = [File {
            path: "tools/go".into(),
            bytes: b"#!/bin/sh\n",
            executable: true,
            encoded: false,
        }];
        materialize(&files, dir.path()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &str| {
                fs::metadata(dir.path().join(p))
                    .unwrap()
                    .permissions()
                    .mode()
            };
            assert!(mode("tools/go") & 0o111 != 0);
            assert!(
                mode("dev") & 0o111 == 0,
                "run with `sh dev`, not executable"
            );
        }
    }

    /// Without a model, the whole flow short of the build hat: a skipped
    /// task for a need that fails here, and a prepared copy that is a git
    /// repository with the run file and the cap. (Trust is the caller's,
    /// so the test touches no store of the user's.)
    #[test]
    fn needs_skip_and_prepare_makes_the_copy() {
        let dir = tempfile::tempdir().unwrap();
        let mut task = crate::task::select(&["slugify".into()], None)[0].clone();
        task.needs = vec!["false".into()];
        let why = needs_met(&task, dir.path()).unwrap_err();
        assert!(why.starts_with("needs `false`"), "{why}");
        task.needs = vec!["true".into()];
        assert_eq!(needs_met(&task, dir.path()), Ok(()));
        prepare(&task, dir.path(), 0.5).unwrap();
        let project = dir.path().join("project");
        assert!(project.join(".git").is_dir());
        assert_eq!(
            fs::read_to_string(project.join(".ryter/run.toml")).unwrap(),
            "test = [\"python3 -m unittest -q test_textutil\"]\ncheck = \"python3 -m unittest -q test_textutil\"\n"
        );
        assert!(
            fs::read_to_string(project.join(".ryter/config.toml"))
                .unwrap()
                .contains("session_budget_usd = 0.50")
        );
        assert!(project.join("textutil.py").is_file());
        assert!(
            !project.join("hidden_test_textutil.py").exists(),
            "hidden tests come later"
        );
    }
}
