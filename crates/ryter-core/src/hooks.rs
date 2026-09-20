//! PreToolUse / PostToolUse / SessionStart / Handoff hooks (command or HTTP).

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::config::HookConfig;
use crate::phase::Phase;
use crate::role::Role;

const TIMEOUT: Duration = Duration::from_secs(5);

/// Which lifecycle event a hook listens for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    /// Before a tool runs. Exit 2 / HTTP 403 denies.
    PreToolUse,
    /// After a tool runs. Cannot deny.
    PostToolUse,
    /// New session. Exit 2 denies starting work.
    SessionStart,
    /// Before a phase handoff. Exit 2 denies the switch.
    Handoff,
}

impl HookEvent {
    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "pretooluse" | "pre_tool_use" => Some(Self::PreToolUse),
            "posttooluse" | "post_tool_use" => Some(Self::PostToolUse),
            "sessionstart" | "session_start" => Some(Self::SessionStart),
            "handoff" => Some(Self::Handoff),
            _ => None,
        }
    }

    /// Canonical config name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
            Self::SessionStart => "SessionStart",
            Self::Handoff => "Handoff",
        }
    }

    /// Events the `/hooks` add menu offers.
    pub fn all() -> &'static [Self] {
        &[
            Self::PreToolUse,
            Self::PostToolUse,
            Self::SessionStart,
            Self::Handoff,
        ]
    }
}

/// Allow or deny from a hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookDecision {
    /// Continue.
    Allow,
    /// Stop this action. Reason is shown to the model / user.
    Deny(String),
}

/// One configured hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hook {
    /// Event name.
    pub event: HookEvent,
    /// Shell command (`/bin/sh -c`).
    pub command: Option<String>,
    /// HTTP POST URL.
    pub url: Option<String>,
    /// Optional glob on the tool name (Pre/Post only).
    pub matcher: Option<String>,
}

/// Loaded hooks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookSet {
    /// In config order.
    pub hooks: Vec<Hook>,
}

impl HookSet {
    /// Build from TOML rows. Unknown events are skipped.
    pub fn from_config(items: &[HookConfig]) -> Self {
        let hooks = items
            .iter()
            .filter_map(|c| {
                let event = HookEvent::parse(&c.event)?;
                if c.command.is_none() && c.url.is_none() {
                    return None;
                }
                Some(Hook {
                    event,
                    command: c.command.clone(),
                    url: c.url.clone(),
                    matcher: c.matcher.clone(),
                })
            })
            .collect();
        Self { hooks }
    }

    /// One-line listing for `/hooks`.
    pub fn summary(&self) -> String {
        if self.hooks.is_empty() {
            return "no hooks configured".into();
        }
        let mut s = String::from("hooks:\n");
        for h in &self.hooks {
            s.push_str("  ");
            s.push_str(h.event.as_str());
            if let Some(c) = &h.command {
                s.push_str("  command=");
                s.push_str(c);
            }
            if let Some(u) = &h.url {
                s.push_str("  url=");
                s.push_str(u);
            }
            if let Some(m) = &h.matcher {
                s.push_str("  matcher=");
                s.push_str(m);
            }
            s.push('\n');
        }
        s
    }

    /// PreToolUse. First deny wins.
    pub fn pre_tool(&self, tool: &str, args: &Value, cwd: &Path, role: Role) -> HookDecision {
        self.run_matching(
            HookEvent::PreToolUse,
            tool,
            json!({
                "event": "PreToolUse",
                "tool": tool,
                "arguments": args,
                "cwd": cwd.to_string_lossy(),
                "role": role.as_str(),
            }),
            cwd,
            true,
        )
    }

    /// PostToolUse. Errors ignored.
    pub fn post_tool(&self, tool: &str, args: &Value, output: &str, cwd: &Path, role: Role) {
        let _ = self.run_matching(
            HookEvent::PostToolUse,
            tool,
            json!({
                "event": "PostToolUse",
                "tool": tool,
                "arguments": args,
                "output": output,
                "cwd": cwd.to_string_lossy(),
                "role": role.as_str(),
            }),
            cwd,
            false,
        );
    }

    /// SessionStart.
    pub fn session_start(&self, cwd: &Path, phase: Phase) -> HookDecision {
        self.run_event(
            HookEvent::SessionStart,
            json!({
                "event": "SessionStart",
                "cwd": cwd.to_string_lossy(),
                "phase": phase.as_str(),
            }),
            cwd,
            true,
        )
    }

    /// Handoff (before the phase switch).
    pub fn handoff(&self, from: Phase, to: Phase, note: &str, cwd: &Path) -> HookDecision {
        self.run_event(
            HookEvent::Handoff,
            json!({
                "event": "Handoff",
                "from": from.as_str(),
                "to": to.as_str(),
                "note": note,
                "cwd": cwd.to_string_lossy(),
            }),
            cwd,
            true,
        )
    }

    fn run_matching(
        &self,
        event: HookEvent,
        tool: &str,
        payload: Value,
        cwd: &Path,
        can_deny: bool,
    ) -> HookDecision {
        for h in &self.hooks {
            if h.event != event {
                continue;
            }
            if !matcher_ok(h.matcher.as_deref(), tool) {
                continue;
            }
            match fire(h, &payload, cwd, can_deny) {
                HookDecision::Deny(m) => return HookDecision::Deny(m),
                HookDecision::Allow => {}
            }
        }
        HookDecision::Allow
    }

    fn run_event(
        &self,
        event: HookEvent,
        payload: Value,
        cwd: &Path,
        can_deny: bool,
    ) -> HookDecision {
        for h in &self.hooks {
            if h.event != event {
                continue;
            }
            match fire(h, &payload, cwd, can_deny) {
                HookDecision::Deny(m) => return HookDecision::Deny(m),
                HookDecision::Allow => {}
            }
        }
        HookDecision::Allow
    }
}

fn matcher_ok(matcher: Option<&str>, tool: &str) -> bool {
    let Some(m) = matcher.map(str::trim).filter(|s| !s.is_empty()) else {
        return true;
    };
    if m == "*" {
        return true;
    }
    glob::Pattern::new(m)
        .map(|p| p.matches(tool))
        .unwrap_or(m == tool)
}

fn fire(h: &Hook, payload: &Value, cwd: &Path, can_deny: bool) -> HookDecision {
    let body = payload.to_string();
    if let Some(cmd) = &h.command {
        match run_command(cmd, &body, cwd) {
            Ok((2, text)) if can_deny => {
                let reason = text.trim();
                return HookDecision::Deny(if reason.is_empty() {
                    "hook denied".into()
                } else {
                    reason.to_string()
                });
            }
            Err(e) if can_deny => return HookDecision::Deny(e),
            _ => {}
        }
    } else if let Some(url) = &h.url {
        match run_http(url, payload) {
            Ok((status, text)) if can_deny && matches!(status, 401 | 403 | 409) => {
                let reason = text.trim();
                return HookDecision::Deny(if reason.is_empty() {
                    format!("hook HTTP {status}")
                } else {
                    reason.to_string()
                });
            }
            Err(e) if can_deny => return HookDecision::Deny(e),
            _ => {}
        }
    }
    HookDecision::Allow
}

fn run_command(command: &str, input: &str, cwd: &Path) -> Result<(i32, String), String> {
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .envs(filtered_env())
        .spawn()
        .map_err(|e| e.to_string())?;
    let pid = child.id();
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
        let _ = stdin.write_all(b"\n");
    }
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(TIMEOUT) {
        Ok(Ok(out)) => {
            let code = out.status.code().unwrap_or(1);
            let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
            if text.trim().is_empty() {
                text = String::from_utf8_lossy(&out.stderr).into_owned();
            }
            Ok((code, text))
        }
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => {
            let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
            Err("hook timed out".into())
        }
    }
}

fn run_http(url: &str, payload: &Value) -> Result<(u16, String), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(url)
        .json(payload)
        .send()
        .map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let text = resp.text().unwrap_or_default();
    Ok((status, text))
}

fn filtered_env() -> Vec<(String, String)> {
    const KEEP: &[&str] = &[
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "TERM",
        "SHELL",
        "TMPDIR",
        "TMP",
        "TEMP",
        "PWD",
        "XDG_RUNTIME_DIR",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "RYTER_HOME",
        "USERNAME",
    ];
    std::env::vars()
        .filter(|(k, _)| KEEP.iter().any(|keep| k.eq_ignore_ascii_case(keep)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    fn write_script(dir: &Path, name: &str, body: &str) -> String {
        let p = dir.join(name);
        fs::write(&p, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        }
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn pre_tool_exit_2_denies() {
        let dir = TempDir::new().unwrap();
        let script = write_script(dir.path(), "deny.sh", "#!/bin/sh\necho no-bash\nexit 2\n");
        let set = HookSet::from_config(&[HookConfig {
            event: "PreToolUse".into(),
            command: Some(script),
            url: None,
            matcher: Some("bash".into()),
        }]);
        let d = set.pre_tool("bash", &json!({"command": "ls"}), dir.path(), Role::Builder);
        assert!(
            matches!(d, HookDecision::Deny(ref s) if s.contains("no-bash")),
            "{d:?}"
        );
        let allow = set.pre_tool("write", &json!({"path": "x"}), dir.path(), Role::Builder);
        assert_eq!(allow, HookDecision::Allow);
    }

    #[test]
    fn pre_tool_exit_0_allows() {
        let dir = TempDir::new().unwrap();
        let script = write_script(dir.path(), "ok.sh", "#!/bin/sh\nexit 0\n");
        let set = HookSet::from_config(&[HookConfig {
            event: "PreToolUse".into(),
            command: Some(script),
            url: None,
            matcher: None,
        }]);
        assert_eq!(
            set.pre_tool("read_file", &json!({}), dir.path(), Role::Orchestrator),
            HookDecision::Allow
        );
    }

    #[test]
    fn handoff_can_deny() {
        let dir = TempDir::new().unwrap();
        let script = write_script(
            dir.path(),
            "stop.sh",
            "#!/bin/sh\necho stay-in-plan\nexit 2\n",
        );
        let set = HookSet::from_config(&[HookConfig {
            event: "Handoff".into(),
            command: Some(script),
            url: None,
            matcher: None,
        }]);
        let d = set.handoff(Phase::Plan, Phase::Build, "skip", dir.path());
        assert!(matches!(d, HookDecision::Deny(s) if s.contains("stay-in-plan")));
    }

    #[test]
    fn post_tool_exit_2_does_not_deny() {
        let dir = TempDir::new().unwrap();
        let script = write_script(dir.path(), "p.sh", "#!/bin/sh\nexit 2\n");
        let set = HookSet::from_config(&[HookConfig {
            event: "PostToolUse".into(),
            command: Some(script),
            url: None,
            matcher: None,
        }]);
        set.post_tool(
            "read_file",
            &json!({}),
            "ok",
            dir.path(),
            Role::Orchestrator,
        );
    }

    #[test]
    fn http_403_denies_pre_tool() {
        use std::io::{Read, Write as _};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf);
                let body = b"nope";
                let resp = format!(
                    "HTTP/1.1 403 Forbidden\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s.write_all(resp.as_bytes());
                let _ = s.write_all(body);
            }
        });
        let set = HookSet::from_config(&[HookConfig {
            event: "PreToolUse".into(),
            command: None,
            url: Some(format!("http://{addr}/hook")),
            matcher: None,
        }]);
        let d = set.pre_tool("bash", &json!({}), Path::new("/tmp"), Role::Builder);
        assert!(
            matches!(d, HookDecision::Deny(ref s) if s.contains("nope")),
            "{d:?}"
        );
    }

    #[test]
    fn summary_lists_hooks() {
        let set = HookSet::from_config(&[HookConfig {
            event: "SessionStart".into(),
            command: Some("true".into()),
            url: None,
            matcher: None,
        }]);
        assert!(set.summary().contains("SessionStart"));
    }
}
