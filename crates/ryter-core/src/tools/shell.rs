//! Shell tool.

use serde_json::Value;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::cancel::kill_group;
use crate::error::{Error, Result};
use crate::tools::{LiveOutput, ToolContext, ToolOutput};

pub fn bash(args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    let cmd = args
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Config("bash: missing command".into()))?;
    let timeout = Duration::from_secs(timeout_secs(args));
    Ok(
        match run_command_live(cmd, &ctx.workspace, timeout, &ctx.cancel, ctx.live.as_ref())? {
            Run::Ok(text) => ToolOutput::ok(text),
            Run::Failed(text) => ToolOutput::err(text),
            Run::Cancelled => ToolOutput::err("cancelled"),
            // "Pass a larger timeout" alone sent models back with the same
            // interactive command and a ten-minute wait.
            Run::TimedOut => ToolOutput::err(format!(
                "bash: timed out after {}s. If it was waiting for something (input, an \
                 editor, a pager, a server that never exits), run it so it doesn't: \
                 non-interactive flags (-m, --yes, --no-pager, --watch=false), or start \
                 servers in the background with output to a file and poll that. Pass a \
                 larger timeout_secs only for work that is genuinely slow.",
                timeout.as_secs()
            )),
        },
    )
}

/// Environment for commands nobody can interact with.
const NON_INTERACTIVE: &[(&str, &str)] = &[
    ("GIT_EDITOR", "false"),
    ("GIT_SEQUENCE_EDITOR", "false"),
    ("EDITOR", "false"),
    ("VISUAL", "false"),
    ("GIT_PAGER", "cat"),
    ("PAGER", "cat"),
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GCM_INTERACTIVE", "never"),
    ("DEBIAN_FRONTEND", "noninteractive"),
    ("PIP_NO_INPUT", "1"),
];

/// Environment variables a key is read from, besides the two built-in ones:
/// every connection's `env_key`. A command's environment leaves them out.
/// Only `XAI_API_KEY` and `OPENROUTER_API_KEY` used to be, so a key under
/// any other name was handed to every command, and `env` put it in the
/// transcript.
static KEY_VARS: std::sync::RwLock<Vec<String>> = std::sync::RwLock::new(Vec::new());

/// Keep `var` out of the environment of every command run from here on.
pub fn hide_env(var: &str) {
    let var = var.trim();
    if var.is_empty() {
        return;
    }
    if let Ok(mut vars) = KEY_VARS.write() {
        if !vars.iter().any(|v| v == var) {
            vars.push(var.to_string());
        }
    }
}

/// How a command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Run {
    /// Exit 0; combined output.
    Ok(String),
    /// Non-zero exit; combined output.
    Failed(String),
    /// Cancel was requested; the process group was killed.
    Cancelled,
    /// Ran past its deadline; the process group was killed.
    TimedOut,
}

/// Run `cmd` under `bash -c` in `cwd`, in its own process group so cancel and
/// timeout kill everything it started.
#[cfg(test)]
pub fn run_command(
    cmd: &str,
    cwd: &std::path::Path,
    timeout: Duration,
    cancel: &crate::cancel::Cancel,
) -> Result<Run> {
    run_command_live(cmd, cwd, timeout, cancel, None)
}

/// How often a running command's newest output is passed on.
const LIVE_EVERY: Duration = Duration::from_millis(250);
/// Lines of a running command's output passed on each time.
const LIVE_LINES: usize = 3;

/// Run `cmd` under `bash -c` in `cwd`, in its own process group so cancel
/// and timeout kill everything it started, passing its newest output lines
/// to `live` as they arrive.
pub fn run_command_live(
    cmd: &str,
    cwd: &std::path::Path,
    timeout: Duration,
    cancel: &crate::cancel::Cancel,
    live: Option<&LiveOutput>,
) -> Result<Run> {
    let mut command = Command::new("bash");
    command
        // `-c`, not `-lc`: a login shell sources the user's profile on every
        // tool call, which is slow and lets a stray `echo` corrupt the output.
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        // Nobody can answer a prompt: make the usual ones fail fast instead
        // of waiting out the timeout. `git commit` without `-m` opened vi.
        .envs(NON_INTERACTIVE.iter().copied())
        // Ryter's own keys are not the project's business, and `env` would
        // put them in the transcript.
        .env_remove("XAI_API_KEY")
        .env_remove("OPENROUTER_API_KEY")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Ok(vars) = KEY_VARS.read() {
        for var in vars.iter() {
            command.env_remove(var);
        }
    }
    // Under a sandbox `/tmp` is shut, so temporary files go to Ryter's
    // scratch folder: `mktemp`, and every tool that calls it, failed.
    if let Some(dir) = crate::sandbox::scratch() {
        let _ = std::fs::create_dir_all(&dir);
        command.env("TMPDIR", dir);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    if cancel.is_cancelled() {
        return Ok(Run::Cancelled);
    }
    let mut child = command.spawn().map_err(|e| Error::Config(e.to_string()))?;
    let pgid = child.id();
    cancel.register_pgid(pgid);
    // Drain both pipes while the command runs: a command that prints more
    // than the pipe buffer would otherwise block until the timeout.
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let start = std::time::Instant::now();
    let mut told = (start, 0usize, 0usize);
    let status = loop {
        if let Some(live) = live {
            if told.0.elapsed() >= LIVE_EVERY {
                let (out, err) = (stdout.len(), stderr.len());
                // Whichever pipe moved last: tests print to stdout, cargo's
                // progress to stderr.
                let moved = if out != told.1 {
                    Some(&stdout)
                } else if err != told.2 {
                    Some(&stderr)
                } else {
                    None
                };
                if let Some(pipe) = moved {
                    live.0(&pipe.tail(LIVE_LINES));
                }
                told = (std::time::Instant::now(), out, err);
            }
        }
        if cancel.is_cancelled() {
            kill_group(pgid);
            let _ = child.kill();
            cancel.unregister_pgid(pgid);
            return Ok(Run::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() > timeout => {
                kill_group(pgid);
                let _ = child.kill();
                cancel.unregister_pgid(pgid);
                return Ok(Run::TimedOut);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                cancel.unregister_pgid(pgid);
                return Err(Error::Config(e.to_string()));
            }
        }
    };
    // The command is done. Anything it left running in the background (a
    // server started with `&`) still holds the output pipes open, and waiting
    // for them to close would wait forever: stop the group.
    let left_running = group_alive(pgid);
    if left_running {
        kill_group(pgid);
    }
    cancel.unregister_pgid(pgid);
    // A process that left the group (`setsid`) can still hold a pipe; take
    // what has arrived rather than wait on it.
    let deadline = std::time::Instant::now() + PIPE_GRACE;
    let (out_text, out_open) = stdout.collect(deadline);
    let (err_text, err_open) = stderr.collect(deadline);
    let mut text = out_text;
    if !err_text.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&err_text);
    }
    let mut note = |line: &str| {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(line);
        text.push('\n');
    };
    if left_running {
        note("[stopped the background processes this command left running]");
    }
    if out_open || err_open {
        note("[a detached background process still holds the output; stopped reading]");
    }
    Ok(if status.success() {
        Run::Ok(text)
    } else {
        // The exit code tells the model (and the chat) how it failed.
        let how = match status.code() {
            Some(c) => format!("[exit {c}]"),
            None => "[killed by a signal]".into(),
        };
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&how);
        Run::Failed(text)
    })
}

/// Default and ceiling for a command's wall clock.
///
/// 30s was below a cold `cargo test` or `npm install`. The model can raise
/// it per command up to the cap.
const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_TIMEOUT_SECS: u64 = 600;

fn timeout_secs(args: &Value) -> u64 {
    args.get("timeout_secs")
        .and_then(Value::as_u64)
        .map(|n| n.clamp(1, MAX_TIMEOUT_SECS))
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
}

/// How long to keep reading after the command exits, for output a detached
/// process may still be holding.
const PIPE_GRACE: Duration = Duration::from_millis(500);

/// A pipe read to EOF on its own thread.
struct Drain {
    buf: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    done: std::sync::mpsc::Receiver<()>,
}

impl Drain {
    /// Bytes read so far.
    fn len(&self) -> usize {
        self.buf.lock().map(|b| b.len()).unwrap_or(0)
    }

    /// The last `n` non-empty lines read so far. A line still being written
    /// counts: a progress bar never ends its line.
    fn tail(&self, n: usize) -> Vec<String> {
        let bytes = self.buf.lock().map(|b| {
            let from = b.len().saturating_sub(4096);
            b[from..].to_vec()
        });
        let text = String::from_utf8_lossy(&bytes.unwrap_or_default()).into_owned();
        let mut lines: Vec<String> = text
            .split(['\n', '\r'])
            .map(str::trim_end)
            .filter(|l| !l.trim().is_empty())
            .map(str::to_string)
            .collect();
        let from = lines.len().saturating_sub(n);
        lines.split_off(from)
    }

    /// What was read by `deadline`, and whether the pipe was still open.
    fn collect(self, deadline: std::time::Instant) -> (String, bool) {
        let wait = deadline.saturating_duration_since(std::time::Instant::now());
        let open = self
            .done
            .recv_timeout(wait)
            .is_err_and(|e| matches!(e, std::sync::mpsc::RecvTimeoutError::Timeout));
        let bytes = self.buf.lock().map(|b| b.clone()).unwrap_or_default();
        (String::from_utf8_lossy(&bytes).into_owned(), open)
    }
}

fn drain(pipe: Option<impl std::io::Read + Send + 'static>) -> Drain {
    let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let (tx, done) = std::sync::mpsc::channel();
    let sink = buf.clone();
    std::thread::spawn(move || {
        if let Some(mut pipe) = pipe {
            let mut chunk = [0u8; 8192];
            while let Ok(n) = pipe.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                if let Ok(mut b) = sink.lock() {
                    b.extend_from_slice(&chunk[..n]);
                }
            }
        }
        let _ = tx.send(());
    });
    Drain { buf, done }
}

/// Whether any process is still in group `pgid`. Bash's builtin `kill`, not
/// `/usr/bin/kill`: procps-ng's `kill -0 -PGID` reports a dead group as alive
/// and a live one as dead.
fn group_alive(pgid: u32) -> bool {
    pgid != 0
        && Command::new("bash")
            .arg("-c")
            .arg(format!("kill -0 -- -{pgid}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cancel::Cancel;
    use crate::role::Role;
    use crate::tools::ToolContext;
    use serde_json::json;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    #[test]
    fn cancel_kills_sleep() {
        let dir = TempDir::new().unwrap();
        let cancel = Cancel::new();
        let ctx = ToolContext {
            live: None,
            workspace: dir.path().to_path_buf(),
            notes_dir: dir.path().to_path_buf(),
            role: Role::SoloBuild,
            always_approve: true,
            mcp: None,
            hooks: None,
            cancel: cancel.clone(),
            user_io: None,
            allowed: Default::default(),
            web: false,
        };
        let waiter = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(80));
            waiter.cancel();
        });
        let start = Instant::now();
        let out = bash(&json!({"command": "sleep 8"}), &ctx).unwrap();
        assert!(out.is_error, "{:?}", out.text);
        assert!(out.text.contains("cancelled"), "{:?}", out.text);
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "sleep was not killed ({:?})",
            start.elapsed()
        );
    }

    fn run(cmd: &str, secs: u64) -> (Run, Duration) {
        let dir = TempDir::new().unwrap();
        let start = Instant::now();
        let r = run_command(cmd, dir.path(), Duration::from_secs(secs), &Cancel::new()).unwrap();
        (r, start.elapsed())
    }

    /// The process id a test's background job wrote to `file`.
    #[cfg(unix)]
    fn pid_in(file: &std::path::Path) -> rustix::process::Pid {
        let pid: i32 = std::fs::read_to_string(file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        rustix::process::Pid::from_raw(pid).unwrap()
    }

    /// Whether `pid` is still a running process (not gone, nor a zombie
    /// waiting for init to reap it).
    #[cfg(unix)]
    fn running(pid: rustix::process::Pid) -> bool {
        rustix::process::test_kill_process(pid).is_ok()
            && !std::fs::read_to_string(format!("/proc/{}/stat", pid.as_raw_nonzero())).is_ok_and(
                |s| {
                    s.split(") ")
                        .nth(1)
                        .is_some_and(|rest| rest.starts_with('Z'))
                },
            )
    }

    /// A server left running with `&` holds the output pipe open. The command
    /// must still return, with its output, and the server must be stopped.
    /// The job names itself in a file before the command goes on, so the
    /// test knows which process to look for without `pgrep -f`.
    #[cfg(unix)]
    #[test]
    fn a_background_process_does_not_hang_the_command() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("pid");
        let cmd = format!(
            "(bash -c 'echo $$ > {f}; exec sleep 30' &) && while [ ! -s {f} ]; do sleep 0.02; done; \
             echo started",
            f = file.display()
        );
        let (r, took) = run(&cmd, 20);
        assert!(took < Duration::from_secs(5), "hung for {took:?}");
        match r {
            Run::Ok(text) => {
                assert!(text.starts_with("started\n"), "{text:?}");
                assert!(text.contains("stopped the background"), "{text:?}");
            }
            other => panic!("{other:?}"),
        }
        // SIGKILL is sent, not awaited: give the process a moment to go.
        let pid = pid_in(&file);
        let gone = (0..100).any(|_| {
            !running(pid) || {
                std::thread::sleep(Duration::from_millis(20));
                false
            }
        });
        assert!(gone, "left running");
    }

    /// Output past the pipe buffer (64 KiB) must not stall until the timeout.
    /// A test run's output reaches the board while it runs, not only when
    /// it ends.
    #[test]
    fn output_is_passed_on_while_the_command_runs() {
        let dir = TempDir::new().unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Vec<String>>::new()));
        let sink = seen.clone();
        let live = LiveOutput(std::sync::Arc::new(move |tail: &[String]| {
            sink.lock().unwrap().push(tail.to_vec());
        }));
        let run = run_command_live(
            "for i in 1 2 3 4; do echo test $i ... ok; sleep 0.3; done",
            dir.path(),
            Duration::from_secs(10),
            &Cancel::new(),
            Some(&live),
        )
        .unwrap();
        assert!(matches!(run, Run::Ok(_)));
        let seen = seen.lock().unwrap();
        // Several updates, the first before the command was done.
        assert!(seen.len() >= 2, "{seen:?}");
        assert!(!seen[0].iter().any(|l| l.contains("test 4")), "{seen:?}");
        assert!(seen.iter().all(|t| t.len() <= LIVE_LINES), "{seen:?}");
    }

    #[test]
    fn large_output_does_not_block() {
        let (r, took) = run("head -c 1000000 /dev/zero | tr '\\0' x", 20);
        assert!(took < Duration::from_secs(5), "took {took:?}");
        match r {
            Run::Ok(text) => assert_eq!(text.len(), 1_000_000),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_command_that_wants_an_editor_fails_fast_and_keys_stay_home() {
        let dir = tempfile::TempDir::new().unwrap();
        let cancel = Cancel::new();
        let run =
            |cmd: &str| run_command(cmd, dir.path(), Duration::from_secs(20), &cancel).unwrap();
        run(
            "git init -q && git config user.email t@t && git config user.name t && touch a && git add a",
        );
        let started = std::time::Instant::now();
        // No -m: git asks the editor for a message.
        assert!(matches!(run("git commit -q"), Run::Failed(_)));
        assert!(started.elapsed() < Duration::from_secs(10));
        // Proves something where the key is set, as on a developer's machine
        // (`OPENROUTER_API_KEY=x cargo test`); setting it here would need
        // `unsafe`.
        let out = match run("env | grep -c -e '^OPENROUTER_API_KEY=' -e '^XAI_API_KEY=' || true") {
            Run::Ok(t) | Run::Failed(t) => t,
            other => panic!("{other:?}"),
        };
        assert_eq!(out.trim(), "0");
    }

    #[test]
    fn ordinary_commands_are_unchanged() {
        assert_eq!(run("echo hi", 5).0, Run::Ok("hi\n".into()));
        match run("echo out; echo err >&2; exit 3", 5).0 {
            Run::Failed(text) => assert_eq!(text, "out\n\nerr\n[exit 3]"),
            other => panic!("{other:?}"),
        }
    }

    /// A process that leaves the group can't be stopped with it; the
    /// command still returns after a short grace, and says why.
    ///
    /// The job leaves through job control (`set -m` gives it a process
    /// group of its own) and names itself in a file before the command goes
    /// on. A `setsid` job raced the command: under load the group was
    /// stopped before it had left, and CI saw "stopped the background
    /// processes" instead.
    #[cfg(unix)]
    #[test]
    fn a_detached_process_holding_the_pipe_does_not_hang() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("pid");
        let cmd = format!(
            "set -m; bash -c 'echo $$ > {f}; exec sleep 30' & while [ ! -s {f} ]; do sleep 0.02; \
             done; echo detached",
            f = file.display()
        );
        let (r, took) = run(&cmd, 20);
        // Stopped by the id it wrote, not by a pattern.
        let _ = rustix::process::kill_process(pid_in(&file), rustix::process::Signal::KILL);
        assert!(took < Duration::from_secs(5), "hung for {took:?}");
        match r {
            Run::Ok(text) => {
                assert!(text.starts_with("detached\n"), "{text:?}");
                assert!(text.contains("stopped reading"), "{text:?}");
            }
            other => panic!("{other:?}"),
        }
    }

    /// A key's variable is kept out of a command's environment, whatever it
    /// is called. `cargo test` sets `CARGO_PKG_NAME` for this process, so it
    /// stands in for a key here: the command sees it until it is hidden.
    #[test]
    fn a_hidden_variable_does_not_reach_a_command() {
        let dir = tempfile::TempDir::new().unwrap();
        let cancel = crate::cancel::Cancel::new();
        let show = || match run_command(
            "echo \"[$CARGO_PKG_NAME]\"",
            dir.path(),
            Duration::from_secs(20),
            &cancel,
        )
        .unwrap()
        {
            Run::Ok(out) => out.trim().to_string(),
            other => panic!("{other:?}"),
        };
        assert_eq!(show(), "[ryter-core]");
        hide_env("CARGO_PKG_NAME");
        hide_env("  ");
        assert_eq!(show(), "[]");
    }
}
