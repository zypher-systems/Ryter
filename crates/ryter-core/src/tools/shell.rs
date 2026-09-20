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
    let mut command = Command::new("bash");
    command
        .arg("-lc")
        .arg(cmd)
        .current_dir(&ctx.workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    if ctx.cancel.is_cancelled() {
        return Ok(ToolOutput::err("cancelled"));
    }
    let mut child = command.spawn().map_err(|e| Error::Config(e.to_string()))?;
    let pgid = child.id();
    ctx.cancel.register_pgid(pgid);
    let timeout = Duration::from_secs(30);
    let start = std::time::Instant::now();
    loop {
        if ctx.cancel.is_cancelled() {
            kill_pgid(pgid);
            let _ = child.kill();
            ctx.cancel.unregister_pgid(pgid);
            return Ok(ToolOutput::err("cancelled"));
        }
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > timeout => {
                kill_pgid(pgid);
                let _ = child.kill();
                ctx.cancel.unregister_pgid(pgid);
                return Ok(ToolOutput::err("bash: timed out after 30s"));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                ctx.cancel.unregister_pgid(pgid);
                return Err(Error::Config(e.to_string()));
            }
        }
    }
    ctx.cancel.unregister_pgid(pgid);
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
    if !out.status.success() {
        return Ok(ToolOutput::err(text));
    }
    Ok(ToolOutput::ok(text))
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
