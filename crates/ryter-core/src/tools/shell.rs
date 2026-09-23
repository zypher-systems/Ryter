//! Shell tool.

use serde_json::Value;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::cancel::kill_group;
use crate::error::{Error, Result};
use crate::tools::{ToolContext, ToolOutput};

pub fn bash(args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    let cmd = args
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Config("bash: missing command".into()))?;
    let timeout = Duration::from_secs(timeout_secs(args));
    Ok(
        match run_command(cmd, &ctx.workspace, timeout, &ctx.cancel)? {
            Run::Ok(text) => ToolOutput::ok(text),
            Run::Failed(text) => ToolOutput::err(text),
            Run::Cancelled => ToolOutput::err("cancelled"),
            Run::TimedOut => ToolOutput::err(format!(
                "bash: timed out after {}s; pass a larger timeout_secs if the \
             command needs it",
                timeout.as_secs()
            )),
        },
    )
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
/// timeout kill everything it started. Shared by the `bash` tool and by the
/// merge gate's configured checks, which are not model-chosen and so do not go
/// through the permission gate.
pub fn run_command(
    cmd: &str,
    cwd: &std::path::Path,
    timeout: Duration,
    cancel: &crate::cancel::Cancel,
) -> Result<Run> {
    let mut command = Command::new("bash");
    command
        // `-c`, not `-lc`: a login shell sources the user's profile on every
        // tool call, which is slow and lets a stray `echo` corrupt the output.
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
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
    let status = loop {
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
/// 30s was below a cold `cargo test` or `npm install`, which made the auditor's
/// own allowlist unrunnable. The model can raise it per command up to the cap.
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
    use crate::queue::TaskQueue;
    use crate::role::Role;
    use crate::tools::ToolContext;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    #[test]
    fn cancel_kills_sleep() {
        let dir = TempDir::new().unwrap();
        let cancel = Cancel::new();
        let ctx = ToolContext {
            workspace: dir.path().to_path_buf(),
            notes_dir: dir.path().to_path_buf(),
            role: Role::Builder,
            always_approve: true,
            queue: Arc::new(Mutex::new(TaskQueue::open(dir.path().join("tasks.json")))),
            mcp: None,
            hooks: None,
            cancel: cancel.clone(),
            user_io: None,
            sticky_approve: Arc::new(std::sync::atomic::AtomicBool::new(false)),
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

    /// A server left running with `&` holds the output pipe open. The command
    /// must still return, with its output, and the server must be stopped.
    #[test]
    fn a_background_process_does_not_hang_the_command() {
        let marker = format!("ryter-bg-{}", std::process::id());
        let cmd = format!("(exec -a {marker} sleep 30 &) && echo started");
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
        let gone = (0..100).any(|_| {
            let alive = Command::new("pgrep")
                .args(["-f", &marker])
                .output()
                .unwrap();
            alive.stdout.is_empty() || {
                std::thread::sleep(Duration::from_millis(20));
                false
            }
        });
        assert!(gone, "left running");
    }

    /// Output past the pipe buffer (64 KiB) must not stall until the timeout.
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
    fn ordinary_commands_are_unchanged() {
        assert_eq!(run("echo hi", 5).0, Run::Ok("hi\n".into()));
        match run("echo out; echo err >&2; exit 3", 5).0 {
            Run::Failed(text) => assert_eq!(text, "out\n\nerr\n[exit 3]"),
            other => panic!("{other:?}"),
        }
    }

    /// A process that leaves the group (`setsid`) can't be stopped with it;
    /// the command still returns after a short grace, and says why.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_detached_process_holding_the_pipe_does_not_hang() {
        let marker = format!("ryter-detached-{}", std::process::id());
        let cmd = format!("setsid bash -c 'exec -a {marker} sleep 30' & echo detached");
        let (r, took) = run(&cmd, 20);
        let _ = Command::new("pkill").args(["-f", &marker]).status();
        assert!(took < Duration::from_secs(5), "hung for {took:?}");
        match r {
            Run::Ok(text) => {
                assert!(text.starts_with("detached\n"), "{text:?}");
                assert!(text.contains("stopped reading"), "{text:?}");
            }
            other => panic!("{other:?}"),
        }
    }
}
