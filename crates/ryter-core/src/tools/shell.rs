//! Shell tool.

use serde_json::Value;
use std::process::{Command, Stdio};
use std::time::Duration;

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
    let start = std::time::Instant::now();
    loop {
        if cancel.is_cancelled() {
            kill_pgid(pgid);
            let _ = child.kill();
            cancel.unregister_pgid(pgid);
            return Ok(Run::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > timeout => {
                kill_pgid(pgid);
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
    }
    cancel.unregister_pgid(pgid);
    let out = child
        .wait_with_output()
        .map_err(|e| Error::Config(e.to_string()))?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(&out.stderr));
    }
    Ok(if out.status.success() {
        Run::Ok(text)
    } else {
        // The exit code tells the model (and the chat) how it failed.
        let how = match out.status.code() {
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

fn kill_pgid(pgid: u32) {
    if pgid == 0 {
        return;
    }
    let _ = Command::new("kill")
        .args(["-KILL", &format!("-{pgid}")])
        .status();
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
}
