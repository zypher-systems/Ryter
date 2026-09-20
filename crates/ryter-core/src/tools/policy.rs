//! Single permission gate.

use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::role::Role;
use crate::tools::{ToolContext, tools_for};

/// Outcome of the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Run now.
    Allow,
    /// Prompt the user (headless: fail closed).
    Ask,
    /// Do not run.
    Deny,
}

/// Authorize `name` / `args` for this context.
pub fn decide(name: &str, args: &Value, ctx: &ToolContext) -> Decision {
    if !tools_for(ctx.role).contains(&name) {
        return Decision::Deny;
    }
    match name {
        "write" | "search_replace" => decide_write(name, args, ctx),
        "read_file" | "list_dir" => decide_read(args, ctx),
        "bash" => decide_bash(args, ctx),
        "grep" | "glob" | "todo_write" | "search_tool" | "use_tool" | "ask_user" => Decision::Allow,
        "web_fetch" | "web_search" => {
            if ctx.web {
                Decision::Allow
            } else {
                Decision::Deny
            }
        }
        _ => Decision::Deny,
    }
}

fn decide_read(args: &Value, ctx: &ToolContext) -> Decision {
    let Some(path) = arg_path(args) else {
        return Decision::Deny;
    };
    match resolve(ctx, &path) {
        None => Decision::Deny,
        Some(p) if is_secret(&p, ctx) => Decision::Deny,
        Some(_) => Decision::Allow,
    }
}

fn decide_write(name: &str, args: &Value, ctx: &ToolContext) -> Decision {
    let Some(path) = arg_path(args) else {
        return Decision::Deny;
    };
    let Some(resolved) = resolve(ctx, &path) else {
        return Decision::Deny;
    };
    if is_secret(&resolved, ctx) {
        return Decision::Deny;
    }
    if is_under(&resolved, &ctx.notes_dir) {
        return Decision::Allow;
    }
    if crate::memory::is_memory_file(&ctx.workspace, &resolved) {
        return Decision::Allow;
    }
    if ctx.role.writes_source() {
        if name == "search_replace" {
            return Decision::Allow;
        }
        return Decision::Allow;
    }
    Decision::Deny
}

fn decide_bash(args: &Value, ctx: &ToolContext) -> Decision {
    let Some(cmd) = args.get("command").and_then(Value::as_str) else {
        return Decision::Deny;
    };
    match ctx.role {
        Role::Builder => {
            if is_destructive(cmd) {
                Decision::Ask
            } else {
                Decision::Allow
            }
        }
        Role::Auditor => {
            if is_test_or_lint(cmd) {
                Decision::Allow
            } else {
                Decision::Deny
            }
        }
        Role::Orchestrator | Role::Planner | Role::Architect => {
            if is_readonly_shell(cmd) {
                Decision::Allow
            } else {
                Decision::Deny
            }
        }
    }
}

fn arg_path(args: &Value) -> Option<String> {
    args.get("path")
        .or_else(|| args.get("target_file"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Resolve a user path against the workspace (or an absolute notes path).
pub fn resolve(ctx: &ToolContext, raw: &str) -> Option<PathBuf> {
    let p = Path::new(raw);
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        ctx.workspace.join(p)
    };
    let abs = normalize(&joined);
    if is_under(&abs, &ctx.workspace) || is_under(&abs, &ctx.notes_dir) {
        Some(abs)
    } else {
        None
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn is_under(path: &Path, root: &Path) -> bool {
    let path = normalize(path);
    let root = normalize(root);
    path == root || path.starts_with(&root)
}

pub(crate) fn is_secret(path: &Path, ctx: &ToolContext) -> bool {
    let rel = path.strip_prefix(&ctx.workspace).unwrap_or(path);
    let name = rel
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name == ".env" || name.ends_with(".pem") || name.ends_with(".key") {
        return true;
    }
    let s = rel
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    s.contains("/.ssh/")
        || s.contains("credential")
        || s.contains("/.ryter/")
        || s.ends_with(".env")
}

fn is_destructive(cmd: &str) -> bool {
    let c = cmd.to_ascii_lowercase();
    c.contains("rm -rf")
        || c.contains("git push --force")
        || c.contains("mkfs")
        || c.contains(" dd ")
}

fn is_test_or_lint(cmd: &str) -> bool {
    let t = cmd.trim();
    t.starts_with("cargo test")
        || t.starts_with("cargo clippy")
        || t.starts_with("cargo fmt")
        || t.starts_with("cargo build")
        || t.starts_with("npm test")
        || t.starts_with("pnpm test")
        || t.starts_with("pytest")
        || t.starts_with("go test")
}

fn is_readonly_shell(cmd: &str) -> bool {
    let t = cmd.trim();
    let first = t.split_whitespace().next().unwrap_or("");
    matches!(
        first,
        "git" | "ls" | "cat" | "wc" | "file" | "which" | "head" | "tail"
    ) && !t.contains('|')
        && !t.contains('>')
        && !t.contains("rm ")
}
