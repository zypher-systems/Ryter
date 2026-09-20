//! Built-in tools and the single permission gate.

mod fs;
mod policy;
mod shell;
mod web;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::error::Result;
use crate::llm::ToolSpec;
use crate::queue::TaskQueue;
use crate::role::Role;

pub use policy::{Decision, decide};

/// Runtime context for a tool call.
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// Project root. All source paths must stay inside it.
    pub workspace: std::path::PathBuf,
    /// Pass-note directory (writable for non-builders).
    pub notes_dir: std::path::PathBuf,
    /// Who is calling.
    pub role: Role,
    /// Treat Ask as Allow (deny still wins).
    pub always_approve: bool,
    /// Shared task queue (`todo_write`).
    pub queue: Arc<Mutex<TaskQueue>>,
    /// Outbound MCP hub.
    pub mcp: Option<Arc<Mutex<crate::mcp::McpHub>>>,
    /// Optional lifecycle hooks.
    pub hooks: Option<Arc<crate::hooks::HookSet>>,
    /// Stop flag for the current turn (bash process groups, stream loops).
    pub cancel: Arc<crate::cancel::Cancel>,
    /// TUI permission / ask_user prompts. Headless is `None` (Ask fails closed).
    pub user_io: Option<crate::user_io::UserIo>,
    /// Session-sticky Ask→Allow (`a` in the TUI).
    pub sticky_approve: Arc<AtomicBool>,
    /// `[features] web`.
    pub web: bool,
}

/// Result of `execute`.
#[derive(Debug, Clone)]
pub struct ToolOutput {
    /// Text the model sees.
    pub text: String,
    /// True when the tool failed.
    pub is_error: bool,
}

impl ToolOutput {
    fn ok(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: false,
        }
    }

    fn err(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: true,
        }
    }
}

/// Names of built-in tools.
pub const TOOL_NAMES: &[&str] = &[
    "read_file",
    "list_dir",
    "grep",
    "glob",
    "write",
    "search_replace",
    "bash",
    "todo_write",
];

/// JSON-schema tool specs for this role (what the model is offered).
pub fn specs_for(role: Role) -> Vec<ToolSpec> {
    specs_for_opts(role, false)
}

/// Like [`specs_for`], omitting web tools unless `web` is true.
pub fn specs_for_opts(role: Role, web: bool) -> Vec<ToolSpec> {
    tools_for(role)
        .iter()
        .filter(|n| web || (**n != "web_fetch" && **n != "web_search"))
        .filter_map(|n| spec(n))
        .collect()
}

fn spec(name: &str) -> Option<ToolSpec> {
    let (description, parameters) = match name {
        "read_file" => (
            "Read a file. Path is relative to the workspace.",
            json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
        ),
        "list_dir" => (
            "List a directory.",
            json!({"type":"object","properties":{"path":{"type":"string"}}}),
        ),
        "grep" => (
            "Search file contents.",
            json!({"type":"object","properties":{"pattern":{"type":"string"}},"required":["pattern"]}),
        ),
        "glob" => (
            "Find files by glob.",
            json!({"type":"object","properties":{"pattern":{"type":"string"}},"required":["pattern"]}),
        ),
        "write" => (
            "Write a file.",
            json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}),
        ),
        "search_replace" => (
            "Replace a unique string in a file.",
            json!({"type":"object","properties":{"path":{"type":"string"},"old_string":{"type":"string"},"new_string":{"type":"string"}},"required":["path","old_string","new_string"]}),
        ),
        "bash" => (
            "Run a shell command in the workspace.",
            json!({"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}),
        ),
        "todo_write" => (
            "Replace the task list. In Build, pending items become parallel builder jobs.",
            json!({"type":"object","properties":{"items":{"type":"array"}},"required":["items"]}),
        ),
        "search_tool" => (
            "Search connected MCP servers for tools.",
            json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}),
        ),
        "use_tool" => (
            "Call an MCP tool by catalog key (server__tool).",
            json!({"type":"object","properties":{"name":{"type":"string"},"arguments":{"type":"object"}},"required":["name"]}),
        ),
        "ask_user" => (
            "Ask the human a question. Use options for a short multiple-choice; omit options for free text.",
            json!({"type":"object","properties":{"question":{"type":"string"},"options":{"type":"array","items":{"type":"string"}}},"required":["question"]}),
        ),
        "web_fetch" => (
            "HTTP GET a URL and return text. Requires [features] web = true. No localhost or private IPs.",
            json!({"type":"object","properties":{"url":{"type":"string"}},"required":["url"]}),
        ),
        "web_search" => (
            "Search the public web. Requires [features] web = true.",
            json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}),
        ),
        _ => return None,
    };
    Some(ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        parameters,
    })
}

/// Tools this role may be offered at all.
pub fn tools_for(role: Role) -> &'static [&'static str] {
    match role {
        Role::Orchestrator => &[
            "read_file",
            "list_dir",
            "grep",
            "glob",
            "write",
            "search_replace",
            "todo_write",
            "search_tool",
            "use_tool",
            "ask_user",
            "web_fetch",
            "web_search",
        ],
        Role::Planner | Role::Architect => &[
            "read_file",
            "list_dir",
            "grep",
            "glob",
            "todo_write",
            "write",
            "search_replace",
            "web_fetch",
            "web_search",
        ],
        Role::Builder => &[
            "read_file",
            "list_dir",
            "grep",
            "glob",
            "write",
            "search_replace",
            "bash",
            "todo_write",
            "web_fetch",
            "web_search",
        ],
        Role::Auditor => &[
            "read_file",
            "list_dir",
            "grep",
            "glob",
            "bash",
            "write",
            "search_replace",
        ],
    }
}

/// Run a built-in tool. Callers must have already applied [`decide`].
pub fn execute(name: &str, args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    match name {
        "read_file" => fs::read_file(args, ctx),
        "list_dir" => fs::list_dir(args, ctx),
        "grep" => fs::grep(args, ctx),
        "glob" => fs::glob_files(args, ctx),
        "write" => fs::write_file(args, ctx),
        "search_replace" => fs::search_replace(args, ctx),
        "bash" => shell::bash(args, ctx),
        "todo_write" => todo_write(args, ctx),
        "search_tool" => mcp_search(args, ctx),
        "use_tool" => mcp_use(args, ctx),
        "ask_user" => ask_user(args, ctx),
        "web_fetch" => web::web_fetch(args, ctx),
        "web_search" => web::web_search(args, ctx),
        other => Ok(ToolOutput::err(format!("unknown tool {other}"))),
    }
}

fn mcp_search(args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    let q = args.get("query").and_then(Value::as_str).unwrap_or("");
    let Some(hub) = &ctx.mcp else {
        return Ok(ToolOutput::ok("no MCP servers connected"));
    };
    let hub = hub
        .lock()
        .map_err(|e| crate::error::Error::Config(e.to_string()))?;
    Ok(ToolOutput::ok(crate::mcp::search_tools(&hub, q)))
}

fn mcp_use(args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| crate::error::Error::Config("use_tool: missing name".into()))?;
    let arguments = args.get("arguments").cloned().unwrap_or(json!({}));
    let Some(hub) = &ctx.mcp else {
        return Ok(ToolOutput::err("no MCP servers connected"));
    };
    let mut hub = hub
        .lock()
        .map_err(|e| crate::error::Error::Config(e.to_string()))?;
    match crate::mcp::use_tool(&mut hub, name, arguments) {
        Ok(t) => Ok(ToolOutput::ok(t)),
        Err(e) => Ok(ToolOutput::err(e.to_string())),
    }
}

fn ask_user(args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    let q = args
        .get("question")
        .and_then(Value::as_str)
        .ok_or_else(|| crate::error::Error::Config("ask_user: missing question".into()))?;
    let options: Vec<String> = args
        .get("options")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .take(8)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let Some(io) = &ctx.user_io else {
        return Ok(ToolOutput::err("ask_user needs the TUI"));
    };
    let answer = io.ask(q, options);
    if answer.trim().is_empty() {
        return Ok(ToolOutput::err("ask_user: no answer"));
    }
    Ok(ToolOutput::ok(answer))
}

fn todo_write(args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    let mut q = ctx
        .queue
        .lock()
        .map_err(|e| crate::error::Error::Config(e.to_string()))?;
    q.apply_todo(args)?;
    let summary = q
        .tasks
        .iter()
        .map(|t| format!("{:?} {} {}", t.status, t.id, t.title))
        .collect::<Vec<_>>()
        .join("\n");
    let n = q.tasks.len();
    Ok(ToolOutput::ok(format!("{n} tasks\n{summary}")))
}

/// Decide then execute. Deny/Ask do not run. PreToolUse hooks can still deny.
pub fn gated_execute(name: &str, args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    if ctx.cancel.is_cancelled() {
        return Ok(ToolOutput::err("cancelled"));
    }
    match decide(name, args, ctx) {
        Decision::Allow => run_with_hooks(name, args, ctx),
        Decision::Ask if ctx.always_approve || ctx.sticky_approve.load(Ordering::SeqCst) => {
            run_with_hooks(name, args, ctx)
        }
        Decision::Deny => Ok(ToolOutput::err(format!(
            "denied: {name} is not allowed for {}",
            ctx.role
        ))),
        Decision::Ask => match &ctx.user_io {
            Some(io) => {
                let summary = crate::user_io::summary_args(name, args);
                match io.permission(name, &summary) {
                    crate::user_io::Permission::Allow => run_with_hooks(name, args, ctx),
                    crate::user_io::Permission::Always => {
                        ctx.sticky_approve.store(true, Ordering::SeqCst);
                        run_with_hooks(name, args, ctx)
                    }
                    crate::user_io::Permission::Deny => {
                        Ok(ToolOutput::err(format!("denied by user: {name} {summary}")))
                    }
                }
            }
            None => Ok(ToolOutput::err(format!(
                "ask: {name} requires approval (no TUI)"
            ))),
        },
    }
}

fn run_with_hooks(name: &str, args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    if let Some(hooks) = &ctx.hooks {
        if let crate::hooks::HookDecision::Deny(msg) =
            hooks.pre_tool(name, args, &ctx.workspace, ctx.role)
        {
            return Ok(ToolOutput::err(format!("hook denied: {msg}")));
        }
    }
    let out = execute(name, args, ctx)?;
    if let Some(hooks) = &ctx.hooks {
        hooks.post_tool(name, args, &out.text, &ctx.workspace, ctx.role);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn ctx(role: Role, root: &std::path::Path) -> ToolContext {
        let notes = root.join(".ryter-notes");
        std::fs::create_dir_all(&notes).unwrap();
        ToolContext {
            workspace: root.to_path_buf(),
            notes_dir: notes,
            role,
            always_approve: false,
            queue: Arc::new(Mutex::new(crate::queue::TaskQueue::open(
                root.join("tasks.json"),
            ))),
            mcp: None,
            hooks: None,
            cancel: crate::cancel::Cancel::new(),
            user_io: None,
            sticky_approve: Arc::new(AtomicBool::new(false)),
            web: false,
        }
    }

    #[test]
    fn orchestrator_can_write_roadmap_not_src() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let c = ctx(Role::Orchestrator, dir.path());
        let road = json!({"path": "ROADMAP.md", "content": "# Roadmap\n"});
        assert_eq!(decide("write", &road, &c), Decision::Allow);
        let out = gated_execute("write", &road, &c).unwrap();
        assert!(!out.is_error);
        assert!(dir.path().join("ROADMAP.md").is_file());
        let src = json!({"path": "src/lib.rs", "content": "fn x() {}"});
        assert_eq!(decide("write", &src, &c), Decision::Deny);
    }

    #[test]
    fn orchestrator_cannot_write_src() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        let c = ctx(Role::Orchestrator, dir.path());
        let args = json!({"path": "src/lib.rs", "content": "fn x() {}"});
        assert_eq!(decide("write", &args, &c), Decision::Deny);
        let out = gated_execute("write", &args, &c).unwrap();
        assert!(out.is_error);
        assert!(!src.join("lib.rs").exists());
    }

    #[test]
    fn builder_can_write_src() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let c = ctx(Role::Builder, dir.path());
        let args = json!({"path": "src/lib.rs", "content": "fn x() {}"});
        assert_eq!(decide("write", &args, &c), Decision::Allow);
        let out = gated_execute("write", &args, &c).unwrap();
        assert!(!out.is_error);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
            "fn x() {}"
        );
    }

    #[test]
    fn todo_write_feeds_queue() {
        let dir = TempDir::new().unwrap();
        let c = ctx(Role::Orchestrator, dir.path());
        let out = gated_execute("todo_write", &json!({"items":["a","b"]}), &c).unwrap();
        assert!(!out.is_error);
        let q = c.queue.lock().unwrap();
        assert_eq!(q.tasks.len(), 2);
        assert_eq!(q.tasks[0].title, "a");
    }

    #[test]
    fn planner_can_write_notes_only() {
        let dir = TempDir::new().unwrap();
        let c = ctx(Role::Planner, dir.path());
        let note = c.notes_dir.join("plan.md");
        let args = json!({"path": note.to_string_lossy(), "content": "# plan\n"});
        assert_eq!(decide("write", &args, &c), Decision::Allow);
        gated_execute("write", &args, &c).unwrap();
        assert!(note.exists());
    }

    #[test]
    fn env_file_is_denied_even_for_builder() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".env"), "SECRET=1").unwrap();
        let c = ctx(Role::Builder, dir.path());
        let args = json!({"path": ".env"});
        assert_eq!(decide("read_file", &args, &c), Decision::Deny);
    }

    #[test]
    fn pre_tool_hook_can_deny_write() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let script = dir.path().join("deny.sh");
        std::fs::write(&script, "#!/bin/sh\necho blocked-by-hook\nexit 2\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let hooks = crate::hooks::HookSet::from_config(&[crate::config::HookConfig {
            event: "PreToolUse".into(),
            command: Some(script.to_string_lossy().into_owned()),
            url: None,
            matcher: Some("write".into()),
        }]);
        let mut c = ctx(Role::Builder, dir.path());
        c.hooks = Some(Arc::new(hooks));
        let args = json!({"path": "src/lib.rs", "content": "fn x() {}"});
        let out = gated_execute("write", &args, &c).unwrap();
        assert!(out.is_error);
        assert!(out.text.contains("hook denied"));
        assert!(!dir.path().join("src/lib.rs").exists());
    }

    #[test]
    fn orchestrator_read_is_allowed() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("README.md"), "hello").unwrap();
        let c = ctx(Role::Orchestrator, dir.path());
        let out = gated_execute("read_file", &json!({"path": "README.md"}), &c).unwrap();
        assert!(!out.is_error);
        assert!(out.text.contains("hello"));
    }

    #[test]
    fn web_tools_are_opt_in() {
        let names: Vec<_> = specs_for_opts(Role::Orchestrator, false)
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert!(names.contains(&"ask_user".into()));
        assert!(!names.contains(&"web_fetch".into()));
        let names: Vec<_> = specs_for_opts(Role::Orchestrator, true)
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert!(names.contains(&"web_fetch".into()));
        assert!(names.contains(&"web_search".into()));
        let dir = TempDir::new().unwrap();
        let mut c = ctx(Role::Orchestrator, dir.path());
        let out = execute("web_fetch", &json!({"url": "https://example.com"}), &c).unwrap();
        assert!(out.is_error);
        c.web = true;
        let out = execute("web_fetch", &json!({"url": "http://127.0.0.1/"}), &c).unwrap();
        assert!(out.is_error);
        assert!(out.text.contains("blocked"), "{out:?}");
    }

    #[test]
    fn ask_is_fail_closed_without_tui() {
        let dir = TempDir::new().unwrap();
        let c = ctx(Role::Builder, dir.path());
        let out = gated_execute("bash", &json!({"command": "rm -rf doomed"}), &c).unwrap();
        assert!(out.is_error);
        assert!(out.text.contains("no TUI"), "{out:?}");
    }

    #[test]
    fn permission_allow_from_user_io() {
        let dir = TempDir::new().unwrap();
        let (io, rx) = crate::user_io::UserIo::pair();
        let mut c = ctx(Role::Builder, dir.path());
        c.user_io = Some(io);
        let worker = std::thread::spawn(move || {
            gated_execute("bash", &json!({"command": "rm -rf doomed"}), &c).unwrap()
        });
        match rx.recv_timeout(std::time::Duration::from_secs(2)) {
            Ok(crate::user_io::UserRequest::Permission { reply, .. }) => {
                reply.send(crate::user_io::Permission::Allow).unwrap();
            }
            other => panic!("expected permission prompt, got {other:?}"),
        }
        let out = worker.join().unwrap();
        assert!(!out.is_error, "{out:?}");
    }

    #[test]
    fn ask_user_needs_tui() {
        let dir = TempDir::new().unwrap();
        let c = ctx(Role::Orchestrator, dir.path());
        let out = gated_execute("ask_user", &json!({"question": "ok?"}), &c).unwrap();
        assert!(out.is_error);
        assert!(out.text.contains("TUI"), "{out:?}");
    }
}
