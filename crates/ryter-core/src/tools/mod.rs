//! Built-in tools and the single permission gate.

mod bounded;
mod fs;
mod packages;
mod policy;
mod secret;
pub(crate) mod shell;
mod web;

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::error::Result;
use crate::llm::ToolSpec;
use crate::role::Role;

pub use fs::changed_lines;
pub(crate) use policy::{Effects, effects, on_this_machine, resolve};
pub use web::Search;
pub(crate) use web::TAVILY_KEY_VAR;

/// What `a` ("allow for this session") on this call's prompt would cover:
/// a key, and the words for it. `None` when the prompt must not offer it.
pub fn allow_scope(name: &str, args: &Value) -> Option<(String, String)> {
    match name {
        "write" | "search_replace" => Some(("edit".into(), "edits to files in the project".into())),
        "bash" => {
            let cmd = args.get("command").and_then(Value::as_str)?;
            if policy::destructive_command(cmd) {
                return None;
            }
            let scope = policy::command_scope(cmd)?;
            let label = if scope.contains("&&") || scope.contains(';') || scope.contains('|') {
                "this exact command".to_string()
            } else {
                format!("`{scope}` commands")
            };
            Some((format!("bash:{scope}"), label))
        }
        other => Some((other.to_string(), format!("{other} calls"))),
    }
}

/// A prompt a reflex key must not answer: destruction the undo may not
/// reach. The TUI takes only `y` for these.
pub fn strict_prompt(name: &str, args: &Value) -> bool {
    name == "bash"
        && args
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(policy::destructive_command)
}
pub(crate) use policy::own_host;
pub use policy::{Decision, decide, removes_stack_data};

/// How the build and test hats answer their own questions: `ask` a
/// person; `always`, a yes to everything inside the project; `yolo`, a
/// yes to everything that asks at all. What is refused is refused in all
/// three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolsMode {
    /// A person answers.
    #[default]
    Ask,
    /// Yes inside the project; outside it, and the project's `.env`,
    /// still ask.
    Always,
    /// Yes to every question.
    Yolo,
}

impl ToolsMode {
    /// `ask`, `always`, `yolo`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Always => "always",
            Self::Yolo => "yolo",
        }
    }

    /// The mode a word names, with the words people use for it.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "ask" | "off" => Some(Self::Ask),
            "always" | "auto" | "on" => Some(Self::Always),
            "yolo" | "full" => Some(Self::Yolo),
            _ => None,
        }
    }

    /// From the two flags a context carries.
    pub fn of(always_approve: bool, yolo: bool) -> Self {
        if yolo {
            Self::Yolo
        } else if always_approve {
            Self::Always
        } else {
            Self::Ask
        }
    }
}

/// Runtime context for a tool call.
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// Per-call sandbox; session storage outside notes/pages stays private.
    pub sandbox: Option<crate::sandbox::Scope>,
    /// Project root. All source paths must stay inside it.
    pub workspace: std::path::PathBuf,
    /// The session's notes folder: the one place the plan hat may write.
    pub notes_dir: std::path::PathBuf,
    /// Who is calling.
    pub role: Role,
    /// Treat Ask as Allow (deny still wins). Writes outside the project
    /// and to the project's `.env` still ask.
    pub always_approve: bool,
    /// Yolo: every question is a yes, those two included. What is refused
    /// stays refused.
    pub yolo: bool,
    /// The user's own rules for what asks.
    pub permissions: Arc<crate::permissions::Permissions>,
    /// Outbound MCP hub.
    pub mcp: Option<Arc<Mutex<crate::mcp::McpHub>>>,
    /// Optional lifecycle hooks.
    pub hooks: Option<Arc<crate::hooks::HookSet>>,
    /// Stop flag for the current turn (bash process groups, stream loops).
    pub cancel: Arc<crate::cancel::Cancel>,
    /// TUI permission / ask_user prompts. Headless is `None` (Ask fails closed).
    pub user_io: Option<crate::user_io::UserIo>,
    /// Kinds of action the user allowed for this session with `a` (edits
    /// to project files, `cargo test`, …); see [`allow_scope`].
    pub allowed: Arc<Mutex<std::collections::HashSet<String>>>,
    /// `[features] web`.
    pub web: bool,
    /// Where a running command's output goes as it arrives. `None` where
    /// only the result matters.
    pub live: Option<LiveOutput>,
    /// Where a shell command's relative paths start. The project's top,
    /// until a `cd` in the same command moves it.
    pub cwd: Cwd,
    /// Variables an earlier part of the same shell command set to a plain
    /// value (`B=http://localhost:8001`), so `$B` later can be read.
    pub vars: Vec<(String, String)>,
    /// The audit hat with no checkpoint to fall back on (a folder that is
    /// not a git repository): held to read-only commands for the turn.
    pub read_only: bool,
    /// Files this turn made (a `write` of a new file, a redirect, `touch`,
    /// `cp`, `mkdir`), which the turn's checkpoint does not hold: deleting
    /// one loses nothing, so it does not ask.
    pub created: Vec<std::path::PathBuf>,
    /// Places this turn moved a file of the user's to (`mv src/x new/`):
    /// never a free deletion, whatever else is known about them.
    pub kept: Vec<std::path::PathBuf>,
    /// Where `web_search` looks, with its key read once.
    pub search: web::Search,
}

impl ToolContext {
    /// Restrict tool/lifecycle work without constraining session record keeping.
    pub fn sandboxed<T: Send>(&self, run: impl FnOnce() -> Result<T> + Send) -> Result<T> {
        match &self.sandbox {
            Some(scope) => scope.run(&self.workspace, &self.notes_dir, run),
            None => run(),
        }
    }
}

/// The folder a shell command is in when one of its parts runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Cwd {
    /// The project's top: where every command starts.
    #[default]
    Project,
    /// A folder an earlier `cd` in the same command moved to.
    At(std::path::PathBuf),
    /// Somewhere the gate can't read: `cd "$DIR"`, `cd -`.
    Unknown,
}

/// Takes a running command's latest output lines.
pub type LiveSink = dyn Fn(&[String]) + Send + Sync;

/// A running command's latest output lines, as they arrive.
#[derive(Clone)]
pub struct LiveOutput(pub Arc<LiveSink>);

impl std::fmt::Debug for LiveOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LiveOutput")
    }
}

/// Marks a permission prompt for a write outside the project. The TUI shows
/// such prompts with a warning and no "allow all".
pub const OUTSIDE: &str = "· outside the project";
/// Appended to a tool name in a permission prompt for a write to the
/// project's own `.env`: a person answers every time.
pub const SECRET: &str = "· a secret file";

/// Result of `execute`.
#[derive(Debug, Clone)]
pub struct ToolOutput {
    /// Text the model sees.
    pub text: String,
    /// True when the tool failed.
    pub is_error: bool,
    /// What an edit did to the file, for the chat. Never sent to the model.
    pub diff: Option<crate::diff::FileDiff>,
}

/// Ceiling on one tool result, in bytes (~8k tokens at the bytes/4 estimate).
///
/// Every result is appended to the transcript and re-billed on every later
/// turn, so an uncapped `cat Cargo.lock` or `grep -r` costs for the rest of the
/// session and can trigger a compaction that discards the conversation.
pub const MAX_TOOL_OUTPUT_BYTES: usize = 32_000;

/// Keep the head and tail of an oversized result and say what was dropped.
///
/// Both ends matter: the head carries the command and the first hits, the tail
/// carries the summary line or the error a build ends with.
pub fn cap_output(text: String) -> String {
    if text.len() <= MAX_TOOL_OUTPUT_BYTES {
        return text;
    }
    let keep = MAX_TOOL_OUTPUT_BYTES / 2;
    let head_end = floor_boundary(&text, keep);
    let tail_start = ceil_boundary(&text, text.len() - keep);
    let dropped = tail_start - head_end;
    format!(
        "{}\n… {dropped} bytes elided; narrow the command or read a range …\n{}",
        &text[..head_end],
        &text[tail_start..]
    )
}

/// Largest char boundary at or below `at`.
fn floor_boundary(s: &str, at: usize) -> usize {
    let mut i = at.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Smallest char boundary at or above `at`.
fn ceil_boundary(s: &str, at: usize) -> usize {
    let mut i = at.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

impl ToolOutput {
    pub(crate) fn ok(text: impl Into<String>) -> Self {
        Self {
            text: cap_output(text.into()),
            is_error: false,
            diff: None,
        }
    }

    fn with_diff(mut self, diff: crate::diff::FileDiff) -> Self {
        self.diff = Some(diff);
        self
    }

    pub(crate) fn err(text: impl Into<String>) -> Self {
        Self {
            text: cap_output(text.into()),
            is_error: true,
            diff: None,
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
            "Read a file. Path is relative to the workspace. Long files come back \
             truncated; pass offset (1-based line) and limit to page through one.",
            json!({"type":"object","properties":{"path":{"type":"string"},"offset":{"type":"integer"},"limit":{"type":"integer"}},"required":["path"]}),
        ),
        "list_dir" => (
            "List a directory.",
            json!({"type":"object","properties":{"path":{"type":"string"}}}),
        ),
        "grep" => (
            "Search file contents with a regex. Respects .gitignore. Narrow with \
             path (a directory) and include (a glob such as \"*.rs\"). Use this to \
             find code before reading files, not bash grep.",
            json!({"type":"object","properties":{
                "pattern":{"type":"string","description":"Rust regex"},
                "path":{"type":"string","description":"directory to search, relative to the workspace"},
                "include":{"type":"string","description":"file glob, e.g. *.rs"},
                "case_insensitive":{"type":"boolean"}
            },"required":["pattern"]}),
        ),
        "glob" => (
            "Find files by glob relative to the workspace (\"src/**/*.rs\").",
            json!({"type":"object","properties":{"pattern":{"type":"string"}},"required":["pattern"]}),
        ),
        "write" => (
            "Create a new file, or replace one wholesale. To change part of an \
             existing file use search_replace: it is cheaper and cannot clobber \
             lines you did not mean to touch.",
            json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}),
        ),
        "search_replace" => (
            "Edit a file by replacing old_string with new_string. old_string must \
             match exactly once, so include enough surrounding lines to make it \
             unique. Read the file first.",
            json!({"type":"object","properties":{"path":{"type":"string"},"old_string":{"type":"string"},"new_string":{"type":"string"}},"required":["path","old_string","new_string"]}),
        ),
        "bash" => (
            "Run a shell command in the workspace. Default timeout 120s; pass \
             timeout_secs (max 600) for a long build or test run. Anything the \
             command starts in the background is stopped when it returns: to \
             check a server, start it, test it, and finish in the same command.",
            json!({"type":"object","properties":{"command":{"type":"string"},"timeout_secs":{"type":"integer"}},"required":["command"]}),
        ),
        "search_tool" => (
            "Search connected MCP servers for tools.",
            json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}),
        ),
        "use_tool" => (
            "Call an MCP tool by catalog key (server__tool).",
            json!({"type":"object","properties":{"name":{"type":"string"},"arguments":{"type":"object"}},"required":["name"]}),
        ),
        "load_skill" => (
            "Load a skill: instructions for a kind of work, listed under Skills in \
             your instructions. Load one before starting work it covers, then follow it. \
             A skill that lists files of its own: pass one as `file` to read it.",
            json!({"type":"object","properties":{
                "name":{"type":"string","description":"the skill's name, e.g. canvas"},
                "file":{"type":"string","description":"one of the skill's own files, as its list names it"}
            },"required":["name"]}),
        ),
        "show_page" => (
            "Show the user a page: one self-contained HTML file, with CSS and any \
             script inline and nothing loaded from the network. Pass the whole page \
             here: Ryter saves it outside the project and opens it for the user. Don't \
             write it, or any helper file for it, into the project. Showing a page with \
             the same title replaces it. Load the canvas skill before making one.",
            json!({"type":"object","properties":{
                "title":{"type":"string","description":"what the page is about; names its file"},
                "html":{"type":"string","description":"the whole page, starting <!doctype html>"}
            },"required":["title","html"]}),
        ),
        "update_rules" => (
            "Change the user's own rules for every project: RYTER.md in Ryter's home folder \
             (~/.ryter/RYTER.md, unless RYTER_HOME puts that folder elsewhere), shown in \
             your instructions under \"The user's rules\". Pass the whole file as it should \
             be after the change: every rule already there, word for word, with yours added, \
             changed or removed. Ryter shows the user the difference and asks before it \
             saves; they may say no. Load the rules skill first. A rule for this project \
             alone goes in the project's RYTER.md instead.",
            json!({"type":"object","properties":{
                "rules":{"type":"string","description":"the whole rules file, in Markdown, as it should be after the change"}
            },"required":["rules"]}),
        ),
        "file_audit" => (
            "File the audit, once, as your last call in the audit hat. `verdict` is pass or \
             fail. `summary` is one line. `findings` are what you checked, worst first, each \
             with `result` (pass, fail, not_reached), `title`, and for a failure `where` \
             (path:line), `detail` (what is wrong) and `saw` (what you ran and what came back). \
             `ran` lists the commands and tools you used. Ryter writes .ryter/audit.md and a \
             dated copy, shows the user the audit, and hands it to the build hat if they say \
             so. Don't write the audit into the chat instead.",
            json!({"type":"object","properties":{
                "verdict":{"type":"string","enum":["pass","fail"]},
                "summary":{"type":"string","description":"one line: what was found"},
                "findings":{"type":"array","items":{"type":"object","properties":{
                    "result":{"type":"string","enum":["pass","fail","not_reached"]},
                    "title":{"type":"string"},
                    "where":{"type":"string","description":"path:line, or where it was seen"},
                    "detail":{"type":"string","description":"what is wrong and why it matters"},
                    "saw":{"type":"string","description":"what you ran and what came back"}
                },"required":["result","title"]}},
                "ran":{"type":"array","items":{"type":"string"},"description":"the commands and tools you used"}
            },"required":["verdict","summary","findings"]}),
        ),
        "present_plan" => (
            "Show the user a plan to approve before any work on it starts. Use it whenever \
             the user asks for a plan, and before work that is more than a small change. \
             Write the plan in Markdown under these headings: Goal, Steps (numbered, each \
             small enough to check), Files, Risks, How to verify. The user reads it in a \
             panel and answers. Approve: the plan is saved in the project and you carry it \
             out in the build hat, in this same turn. Adjust: they say what to change; \
             revise the plan and present it again. Reject: nothing is saved. Never ask in \
             plain text whether a plan is acceptable: present it.",
            json!({"type":"object","properties":{
                "title":{"type":"string","description":"a few words: the panel's title and the file's name"},
                "plan":{"type":"string","description":"the plan, in Markdown"}
            },"required":["title","plan"]}),
        ),
        "record_decision" => (
            "Record one place where the work differs from the plan the user approved, and \
             why. Call it when the user tells you to leave out, add or change something the \
             plan says, and when the plan can't be followed as written and you take another \
             way to the same goal. One call for each difference, when it is decided, before \
             you build it. The entry goes in `.ryter/decisions.md` in the project, and a \
             review reads it: a difference recorded there is not held against the work. \
             Don't record work that follows the plan, or things the plan doesn't speak to.",
            json!({"type":"object","properties":{
                "title":{"type":"string","description":"a few words: what was decided"},
                "plan_said":{"type":"string","description":"what the plan says, with its step"},
                "built_instead":{"type":"string","description":"what is built instead"},
                "why":{"type":"string","description":"the reason, in a sentence; the user's own words when they decided"},
                "decided_by":{"type":"string","enum":["user","model"],"description":"`user` when they told you to; `model` when you chose"}
            },"required":["title","plan_said","built_instead","why","decided_by"]}),
        ),
        "propose_run" => (
            "Propose how this project runs, for the user to approve: the command that \
             starts it, an address that answers once it is up, its test commands, and the \
             command that stops it. Read the project first (its README, compose file, \
             package.json, Makefile, scripts). Use the commands the project itself uses. \
             The start command may return (`docker compose up -d --wait`) or stay in the \
             foreground (`npm run dev`): Ryter runs it and keeps it up. Approved, it is \
             saved as `.ryter/run.toml` and Ryter runs these commands with run_project, \
             without asking again. Leave out what the project doesn't have.",
            json!({"type":"object","properties":{
                "start":{"type":"string","description":"starts the product"},
                "ready":{"type":"string","description":"an http address on this machine that answers once it is up, e.g. http://localhost:8000/healthz"},
                "test":{"type":"array","items":{"type":"string"},"description":"the project's test and check commands, in order"},
                "stop":{"type":"string","description":"stops what start started"}
            }}),
        ),
        "run_project" => (
            "Run the project's own approved commands from `.ryter/run.toml`. `start` \
             starts the product and waits until it is up; it stays up after your turn. \
             `test` runs the project's test commands and returns their output. `stop` \
             stops what was started (the user usually does this, with /stop: leave the \
             product running when you finish). `status` says whether it is up. With no \
             run file yet, propose one with propose_run. The build and audit hats run \
             these; the plan hat starts nothing.",
            json!({"type":"object","properties":{
                "action":{"type":"string","enum":["start","test","stop","status"]}
            },"required":["action"]}),
        ),
        "request_hat" => (
            "Ask the user to switch your hat, e.g. to build once a plan is ready or once a \
             review found things to fix. They answer yes or no; on yes you continue in the \
             new hat in this same turn. Never ask in plain text whether to switch: the user \
             can't answer that from here.",
            json!({"type":"object","properties":{
                "hat":{"type":"string","enum":["build","plan","audit","scribe"]},
                "reason":{"type":"string","description":"one line the user sees, e.g. 'carry out the plan'"}
            },"required":["hat","reason"]}),
        ),
        "ask_user" => (
            "Ask the human a question. Use options for a short multiple-choice; omit options for free text.",
            json!({"type":"object","properties":{"question":{"type":"string"},"options":{"type":"array","items":{"type":"string"}}},"required":["question"]}),
        ),
        "check_package" => (
            "A dependency's latest release and its known security advisories, from its registry \
             (crates.io, npm, PyPI, the Go module proxy) and OSV.dev. Give the ecosystem, the \
             package name, and the version in use when you know it. For the plan and audit hats.",
            json!({"type":"object","properties":{
                "ecosystem":{"type":"string","enum":["crates","npm","pypi","go"]},
                "name":{"type":"string","description":"the package as its registry names it (`serde`, `@fastify/cookie`, `fastapi`, `github.com/gin-gonic/gin`)"},
                "version":{"type":"string","description":"the version in use, if any; advisories are then for it alone"}
            },"required":["ecosystem","name"]}),
        ),
        "web_fetch" => (
            "HTTP GET a public URL and return its text. For the plan and audit hats. No localhost \
             or private addresses.",
            json!({"type":"object","properties":{"url":{"type":"string"}},"required":["url"]}),
        ),
        "web_search" => (
            "Search the web through the user's configured provider ([search] in config: tavily or \
             searxng) and return titles, addresses and snippets. For the plan and audit hats. For \
             a dependency's version or advisories use check_package instead.",
            json!({"type":"object","properties":{"query":{"type":"string"},"max_results":{"type":"integer","description":"1 to 10; the configured number by default"}},"required":["query"]}),
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
        // One list for every hat, so switching hats never changes the tool
        // definitions (and never throws away the prompt cache). The gate
        // decides what each hat may run.
        Role::SoloPlan | Role::SoloBuild | Role::SoloAudit | Role::SoloScribe => &[
            "read_file",
            "list_dir",
            "grep",
            "glob",
            "write",
            "search_replace",
            "bash",
            "ask_user",
            "request_hat",
            "present_plan",
            "record_decision",
            "load_skill",
            "show_page",
            "update_rules",
            "propose_run",
            "run_project",
            "file_audit",
            "search_tool",
            "use_tool",
            "check_package",
            "web_fetch",
            "web_search",
        ],
        // A role from crew mode, which is gone: nothing runs as it.
        Role::Crew => &[],
    }
}

/// Run a built-in tool. Callers must have already applied [`decide`].
pub fn execute(name: &str, args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    ctx.sandboxed(|| execute_inner(name, args, ctx))
}

fn execute_inner(name: &str, args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    match name {
        "read_file" => fs::read_file(args, ctx),
        "list_dir" => fs::list_dir(args, ctx),
        "grep" => fs::grep(args, ctx),
        "glob" => fs::glob_files(args, ctx),
        "write" => fs::write_file(args, ctx),
        "search_replace" => fs::search_replace(args, ctx),
        "bash" => shell::bash(args, ctx),
        "search_tool" => mcp_search(args, ctx),
        "use_tool" => mcp_use(args, ctx),
        "ask_user" => ask_user(args, ctx),
        // The agent loop answers this itself: it changes who the agent is.
        "request_hat" | "present_plan" | "record_decision" | "load_skill" | "show_page"
        | "update_rules" | "propose_run" | "run_project" | "file_audit" => Ok(ToolOutput::err(
            format!("{name} is handled by the agent loop"),
        )),
        "check_package" => packages::check_package(args, ctx),
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
    // The hub is locked only to find the server: a slow call must not stall
    // `search_tool` or calls to other servers.
    let routed = hub
        .lock()
        .map_err(|e| crate::error::Error::Config(e.to_string()))?
        .route(name);
    let (tool, server) = match routed {
        Ok(r) => r,
        Err(e) => return Ok(ToolOutput::err(e.to_string())),
    };
    match crate::mcp::call_tool(&server, &tool, arguments, Some(&ctx.cancel)) {
        Ok(t) => Ok(ToolOutput::ok(t)),
        Err(crate::error::Error::Cancelled) => Ok(ToolOutput::err("cancelled")),
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
    let answer = io.ask(q, options, &ctx.cancel);
    if ctx.cancel.is_cancelled() {
        return Err(crate::error::Error::Cancelled);
    }
    if answer.trim().is_empty() {
        // "no answer" read as a glitch, and models asked again.
        return Ok(ToolOutput::err(
            "the user closed the question without answering. Don't ask it again: finish \
             what you can without it, or stop and wait for their next message",
        ));
    }
    Ok(ToolOutput::ok(answer))
}

/// What a reviewer may do with containers, for a refusal: a model refused
/// `docker compose up` otherwise concludes it has no Docker at all.
const CONTAINER_CHECKS: &str = " Docker is here, and tests and linters do run in the \
    project's containers: `docker compose run --rm <service> <test command>`, `docker compose \
    exec <service> <test command>`, and `docker compose ps` / `logs` to look. Building, \
    starting and stopping the stack are not yours.";

/// Decide then execute. Deny/Ask do not run. PreToolUse hooks can still deny.
pub fn gated_execute(name: &str, args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    ctx.sandboxed(|| gated_execute_inner(name, args, ctx))
}

fn gated_execute_inner(name: &str, args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    if ctx.cancel.is_cancelled() {
        return Ok(ToolOutput::err("cancelled"));
    }
    match decide(name, args, ctx) {
        Decision::Allow => run_with_hooks(name, args, ctx),
        // Yolo: every question is a yes, the rest of the machine and the
        // project's `.env` included. A refusal is still a refusal.
        Decision::Ask | Decision::AskOutside | Decision::AskSecret if ctx.yolo => {
            run_with_hooks(name, args, ctx)
        }
        // Outside the project: a person answers every time. "Allow all" and
        // --always-approve cover the project, not the rest of the machine.
        Decision::AskOutside => match &ctx.user_io {
            Some(io) => {
                let summary = crate::user_io::summary_args(name, args);
                match io.permission_with(
                    &format!("{name} {OUTSIDE}"),
                    &summary,
                    fs::preview(name, args, ctx),
                    &ctx.cancel,
                ) {
                    crate::user_io::Permission::Allow | crate::user_io::Permission::Always => {
                        run_with_hooks(name, args, ctx)
                    }
                    crate::user_io::Permission::Deny if ctx.cancel.is_cancelled() => {
                        Err(crate::error::Error::Cancelled)
                    }
                    crate::user_io::Permission::Deny => Ok(ToolOutput::err(format!(
                        "denied by user: {name} {summary} (outside the project). Work inside \
                         the project instead, or ask the user where it should go"
                    ))),
                }
            }
            None => Ok(ToolOutput::err(format!(
                "denied: {name} writes outside the project, which needs a person's yes each \
                 time, and nobody can be asked here (headless). --always-approve covers the \
                 project only. Work inside the project instead"
            ))),
        },
        // The project's own `.env`: a person answers every time, and what
        // is written is shown to them, never to the model.
        Decision::AskSecret => match &ctx.user_io {
            Some(io) => {
                let summary = crate::user_io::summary_args(name, args);
                match io.permission_with(
                    &format!("{name} {SECRET}"),
                    &summary,
                    fs::preview(name, args, ctx),
                    &ctx.cancel,
                ) {
                    crate::user_io::Permission::Allow | crate::user_io::Permission::Always => {
                        run_with_hooks(name, args, ctx)
                    }
                    crate::user_io::Permission::Deny if ctx.cancel.is_cancelled() => {
                        Err(crate::error::Error::Cancelled)
                    }
                    crate::user_io::Permission::Deny => Ok(ToolOutput::err(format!(
                        "denied by user: {name} {summary} (a secret file). Don't retry it: \
                         tell the user what the file needs and let them write it"
                    ))),
                }
            }
            None => Ok(ToolOutput::err(format!(
                "denied: {name} {} is a secret file, which needs a person's yes each time, \
                 and nobody can be asked here (headless). --always-approve doesn't cover it. \
                 Tell the user what the file needs",
                crate::user_io::summary_args(name, args)
            ))),
        },
        Decision::Ask
            if ctx.always_approve
                || allow_scope(name, args)
                    .is_some_and(|(key, _)| ctx.allowed.lock().is_ok_and(|a| a.contains(&key))) =>
        {
            run_with_hooks(name, args, ctx)
        }
        // The web is the looking hats': off, say where it is turned on;
        // on, say who may use it.
        Decision::Deny if matches!(name, "web_fetch" | "web_search") => {
            // `/settings` is the one place that turns it on: `settings.toml`
            // is applied after `config.toml`, so an edit there does nothing
            // against a saved no.
            Ok(ToolOutput::err(if !ctx.web {
                format!(
                    "denied: {name} is off ([features] web = false). Tell the user: /settings \
                     turns web on{}.",
                    if matches!(ctx.role, Role::SoloPlan | Role::SoloAudit) {
                        ""
                    } else {
                        ", and it is the plan and audit hats' tool"
                    }
                )
            } else {
                format!(
                    "denied: {name} is the plan and audit hats' tool; the {} hat does not use \
                     it. Tell the user, who can switch to plan or audit.",
                    ctx.role
                )
            }))
        }
        // The looking hats ask after dependencies; the hats that build and
        // document do not, and are told who does.
        Decision::Deny if name == "check_package" => Ok(ToolOutput::err(format!(
            "denied: check_package (a dependency's latest release and known advisories) is the \
             plan and audit hats' question; the {} hat does not ask it. Tell the user, who can \
             switch to plan or audit.",
            ctx.role
        ))),
        // Say which gate refused. "not allowed for <role>" on every denial
        // taught models a tool was forbidden when only the arguments were.
        Decision::Deny if !tools_for(ctx.role).contains(&name) => {
            Ok(ToolOutput::err(format!("denied: there is no {name} tool")))
        }
        // A hat that can't do this: say which one can, so the model tells the
        // user instead of hunting for a way round.
        Decision::Deny
            if matches!(
                ctx.role,
                Role::SoloPlan | Role::SoloAudit | Role::SoloScribe
            ) && matches!(name, "write" | "search_replace" | "bash")
                && !(name == "bash" && policy::bash_hint(args, ctx).is_some()) =>
        {
            // What the audit may do is said, or the model took "changes
            // nothing" for "writes nothing" while its shell commands wrote
            // probe scripts behind the checkpoint all along.
            if ctx.role == Role::SoloAudit && name != "bash" {
                return Ok(ToolOutput::err(
                    "denied: the audit's only file is audit.md (file_audit); its findings go \
                     there. A probe or a fixture it needs goes through bash (`cat > probe.sh`), \
                     in the project or /tmp: the checkpoint puts the tree back when the turn \
                     ends. A lasting change is the build hat's: tell the user, who can press \
                     Shift+Tab for it.",
                ));
            }
            if ctx.role == Role::SoloScribe && name != "bash" {
                return Ok(ToolOutput::err(
                    "denied: the scribe writes documentation (.md, .txt and their kind), \
                     not this file. Tell the user: Shift+Tab to the build hat for code.",
                ));
            }
            let key = if matches!(ctx.role, Role::SoloAudit | Role::SoloScribe) {
                "Shift+Tab"
            } else {
                "Tab"
            };
            // The looking hats refuse most commands because they look, not
            // because the command changes anything: a `GET` of the product
            // was once refused as one that did.
            if name == "bash" && matches!(ctx.role, Role::SoloPlan | Role::SoloScribe) {
                return Ok(ToolOutput::err(format!(
                    "denied: the {} hat only looks — it runs read-only commands, curl GET/HEAD \
                     to this project's own address, and a project program's --help or \
                     --version; tell the user, who can press {key} for build.",
                    ctx.role
                )));
            }
            Ok(ToolOutput::err(format!(
                "denied: the {} hat can't {} — tell the user; they can press {key} to switch to \
                 build.{}",
                ctx.role,
                if name == "bash" {
                    "run commands that change things"
                } else {
                    "edit files"
                },
                if name == "bash" && ctx.role == Role::SoloAudit && policy::names_containers(args) {
                    CONTAINER_CHECKS
                } else {
                    ""
                }
            )))
        }
        Decision::Deny if name == "bash" && policy::bash_hint(args, ctx).is_some() => {
            Ok(ToolOutput::err(format!(
                "denied: bash {} — {}",
                crate::user_io::summary_args(name, args),
                policy::bash_hint(args, ctx).unwrap_or_default()
            )))
        }
        Decision::Deny => Ok(ToolOutput::err(format!(
            "denied: {name} {} — the arguments are outside policy (missing or \
             out-of-workspace path, a secret file, or a blocked command). \
             Adjust the arguments rather than retrying the same call.",
            crate::user_io::summary_args(name, args)
        ))),
        Decision::Ask => match &ctx.user_io {
            Some(io) => {
                let summary = crate::user_io::summary_args(name, args);
                let scope = allow_scope(name, args);
                let answer = io.ask_tool(
                    crate::user_io::ToolAsk {
                        tool: name.to_string(),
                        summary: summary.clone(),
                        preview: fs::preview(name, args, ctx),
                        strict: strict_prompt(name, args),
                        scope: scope.as_ref().map(|(_, label)| label.clone()),
                        whole: false,
                        asks: (name == "bash")
                            .then(|| args.get("command").and_then(serde_json::Value::as_str))
                            .flatten()
                            .and_then(|c| policy::asking_segment(c, ctx)),
                    },
                    &ctx.cancel,
                );
                match answer {
                    crate::user_io::Permission::Allow => run_with_hooks(name, args, ctx),
                    // `a`: this kind of action, for the session. It used to
                    // allow everything, which is why it took two presses.
                    crate::user_io::Permission::Always => {
                        if let (Some((key, _)), Ok(mut a)) = (scope, ctx.allowed.lock()) {
                            a.insert(key);
                        }
                        run_with_hooks(name, args, ctx)
                    }
                    crate::user_io::Permission::Deny if ctx.cancel.is_cancelled() => {
                        Err(crate::error::Error::Cancelled)
                    }
                    // Said so the model doesn't ask again in other words.
                    crate::user_io::Permission::Deny => Ok(ToolOutput::err(format!(
                        "denied by user: {name} {summary}. Don't retry it or a variant of it: \
                         carry on without it, or ask the user what they'd like instead"
                    ))),
                }
            }
            None => Ok(ToolOutput::err(format!(
                "ask: {name} needs approval and nobody can be asked (headless). Tell the user: \
                 run with --always-approve to let it run, or use the TUI"
            ))),
        },
    }
}

fn run_with_hooks(name: &str, args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    // The gate already entered this exact session scope. Hooks inherit it;
    // do not create another worker or reopen grant directories from inside it.
    let mut scoped = ctx.clone();
    scoped.sandbox = None;
    with_hooks(name, args, &scoped, || execute_inner(name, args, ctx))
}

/// Run a tool the agent handles itself (`show_page`, `load_skill`,
/// `request_hat`) through the same hooks as every other tool: a
/// `PreToolUse` deny stops it, and `PostToolUse` sees its result. They used
/// to skip both, so a deny hook could not stop a page being written.
pub fn with_hooks(
    name: &str,
    args: &Value,
    ctx: &ToolContext,
    run: impl FnOnce() -> Result<ToolOutput>,
) -> Result<ToolOutput> {
    if let Some(hooks) = &ctx.hooks {
        if let crate::hooks::HookDecision::Deny(msg) =
            ctx.sandboxed(|| Ok(hooks.pre_tool(name, args, &ctx.workspace, ctx.role)))?
        {
            return Ok(ToolOutput::err(format!("hook denied: {msg}")));
        }
    }
    // A tool failing is information for the model — the file does not exist
    // yet, the path is a directory, the bytes are not text — not a reason to
    // end the task. On the first live crew run, builders reading a file they
    // were about to create died with "No such file or directory". Only a
    // cancel ends the task.
    let out = match run() {
        Ok(o) => o,
        Err(crate::error::Error::Cancelled) => return Err(crate::error::Error::Cancelled),
        Err(e) => ToolOutput::err(format!("{name} failed: {e}")),
    };
    if let Some(hooks) = &ctx.hooks {
        ctx.sandboxed(|| {
            hooks.post_tool(name, args, &out.text, &ctx.workspace, ctx.role);
            Ok(())
        })?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    pub(super) fn ctx(role: Role, root: &std::path::Path) -> ToolContext {
        let notes = root.join(".ryter-notes");
        std::fs::create_dir_all(&notes).unwrap();
        ToolContext {
            sandbox: None,
            live: None,
            workspace: root.to_path_buf(),
            notes_dir: notes,
            role,
            always_approve: false,
            yolo: false,
            permissions: Default::default(),
            mcp: None,
            hooks: None,
            cancel: crate::cancel::Cancel::new(),
            user_io: None,
            allowed: Default::default(),
            web: false,
            cwd: Default::default(),
            vars: Default::default(),
            // The audit hat with no checkpoint behind it (see policy's
            // `ctx_for`).
            read_only: role == Role::SoloAudit,
            created: Vec::new(),
            kept: Vec::new(),
            search: Default::default(),
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn sandboxed_gate_and_hooks_follow_the_current_notes() {
        use crate::sandbox::{
            SandboxProfile,
            tests::{fixture_scope, outside_scratch},
        };
        let ws = outside_scratch();
        let home = outside_scratch();
        let scope = fixture_scope(SandboxProfile::Workspace, home.path());
        let mut c = ctx(Role::SoloBuild, ws.path());
        c.always_approve = true;
        c.notes_dir = home.path().join("sessions/project/first/notes");
        if let Err(error) = scope.check(ws.path(), &c.notes_dir) {
            assert!(error.to_string().contains("Landlock is unavailable"));
            return;
        }
        c.sandbox = Some(scope);
        let secret = home.path().join("private");
        std::fs::write(&secret, "private\n").unwrap();
        let command = format!(
            "if IFS= read -r line < '{}'; then echo leaked; exit 2; fi; printf checked >> hooks.log",
            secret.display()
        );
        c.hooks = Some(Arc::new(crate::hooks::HookSet::from_config(&[
            crate::config::HookConfig {
                event: "PreToolUse".into(),
                command: Some(command.clone()),
                url: None,
                matcher: None,
            },
            crate::config::HookConfig {
                event: "PostToolUse".into(),
                command: Some(command),
                url: None,
                matcher: None,
            },
        ])));
        for id in ["first", "second", "first"] {
            c.notes_dir = home.path().join(format!("sessions/project/{id}/notes"));
            let path = c.notes_dir.join("written");
            let out = gated_execute("write", &json!({"path":path, "content":id}), &c).unwrap();
            assert!(!out.is_error, "{}", out.text);
            assert_eq!(std::fs::read_to_string(path).unwrap(), id);
        }
        assert_eq!(
            std::fs::read_to_string(ws.path().join("hooks.log")).unwrap(),
            "checked".repeat(6)
        );
    }

    #[test]
    fn the_plan_hat_writes_memory_not_source() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let c = ctx(Role::SoloPlan, dir.path());
        let road = json!({"path": "ROADMAP.md", "content": "# Roadmap\n"});
        assert_eq!(decide("write", &road, &c), Decision::Allow);
        let out = gated_execute("write", &road, &c).unwrap();
        assert!(!out.is_error);
        assert!(dir.path().join("ROADMAP.md").is_file());
        let src = json!({"path": "src/lib.rs", "content": "fn x() {}"});
        assert_eq!(decide("write", &src, &c), Decision::Deny);
    }

    #[test]
    fn the_plan_hat_cannot_write_source() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        let c = ctx(Role::SoloPlan, dir.path());
        let args = json!({"path": "src/lib.rs", "content": "fn x() {}"});
        assert_eq!(decide("write", &args, &c), Decision::Deny);
        let out = gated_execute("write", &args, &c).unwrap();
        assert!(out.is_error);
        assert!(!src.join("lib.rs").exists());
    }

    #[test]
    fn the_build_hat_writes_source_once_allowed() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let mut c = ctx(Role::SoloBuild, dir.path());
        let args = json!({"path": "src/lib.rs", "content": "fn x() {}"});
        // Edits run; `[permissions] edit = "ask"` makes them a question.
        assert_eq!(decide("write", &args, &c), Decision::Allow);
        c.permissions = Arc::new(crate::permissions::Permissions {
            edit: Some(crate::permissions::Answer::Ask),
            ..Default::default()
        });
        assert_eq!(decide("write", &args, &c), Decision::Ask);
        c.always_approve = true;
        let out = gated_execute("write", &args, &c).unwrap();
        assert!(!out.is_error);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
            "fn x() {}"
        );
    }

    #[test]
    fn the_plan_hat_writes_its_notes() {
        let dir = TempDir::new().unwrap();
        let c = ctx(Role::SoloPlan, dir.path());
        let note = c.notes_dir.join("plan.md");
        let args = json!({"path": note.to_string_lossy(), "content": "# plan\n"});
        assert_eq!(decide("write", &args, &c), Decision::Allow);
        gated_execute("write", &args, &c).unwrap();
        assert!(note.exists());
    }

    #[test]
    fn env_file_is_denied_even_for_the_build_hat() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".env"), "SECRET=1").unwrap();
        let c = ctx(Role::SoloBuild, dir.path());
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
        let mut c = ctx(Role::SoloBuild, dir.path());
        c.always_approve = true;
        c.hooks = Some(Arc::new(hooks));
        let args = json!({"path": "src/lib.rs", "content": "fn x() {}"});
        let out = gated_execute("write", &args, &c).unwrap();
        assert!(out.is_error);
        assert!(out.text.contains("hook denied"));
        assert!(!dir.path().join("src/lib.rs").exists());
    }

    #[test]
    fn reading_is_allowed() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("README.md"), "hello").unwrap();
        let c = ctx(Role::SoloPlan, dir.path());
        let out = gated_execute("read_file", &json!({"path": "README.md"}), &c).unwrap();
        assert!(!out.is_error);
        assert!(out.text.contains("hello"));
    }

    /// The scribe is told what it writes, and where code is written.
    #[test]
    fn a_scribe_write_of_code_says_where_code_goes() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let c = ctx(Role::SoloScribe, dir.path());
        let out = gated_execute(
            "write",
            &json!({"path": "src/main.rs", "content": "fn main() {}"}),
            &c,
        )
        .unwrap();
        assert!(
            out.is_error && out.text.contains("the scribe writes documentation"),
            "{out:?}"
        );
        assert!(!dir.path().join("src/main.rs").exists());
        let out = gated_execute(
            "write",
            &json!({"path": "docs/notes.md", "content": "# Notes\n"}),
            &c,
        )
        .unwrap();
        assert!(!out.is_error, "{out:?}");
        assert!(dir.path().join("docs/notes.md").exists());
        let out = gated_execute("bash", &json!({"command": "cargo build"}), &c).unwrap();
        assert!(out.is_error && out.text.contains("Shift+Tab"), "{out:?}");
    }

    #[test]
    fn web_tools_are_opt_in() {
        let names: Vec<_> = specs_for_opts(Role::SoloPlan, false)
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert!(names.contains(&"ask_user".into()));
        assert!(!names.contains(&"web_fetch".into()));
        let names: Vec<_> = specs_for_opts(Role::SoloPlan, true)
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert!(names.contains(&"web_fetch".into()));
        assert!(names.contains(&"web_search".into()));
        let dir = TempDir::new().unwrap();
        let mut c = ctx(Role::SoloPlan, dir.path());
        let out = execute("web_fetch", &json!({"url": "https://example.com"}), &c).unwrap();
        assert!(out.is_error);
        c.web = true;
        let out = execute("web_fetch", &json!({"url": "http://127.0.0.1/"}), &c).unwrap();
        assert!(out.is_error);
        assert!(out.text.contains("blocked"), "{out:?}");
    }

    /// No single result may dominate the window, and both ends survive.
    #[test]
    fn oversized_tool_output_is_capped_at_both_ends() {
        let dir = TempDir::new().unwrap();
        let c = ctx(Role::SoloBuild, dir.path());
        let big = "x".repeat(MAX_TOOL_OUTPUT_BYTES * 3);
        std::fs::write(dir.path().join("big.txt"), &big).unwrap();
        let out = execute("read_file", &json!({"path": "big.txt"}), &c).unwrap();
        assert!(
            out.text.len() < MAX_TOOL_OUTPUT_BYTES + 200,
            "capped length, got {}",
            out.text.len()
        );
        assert!(out.text.contains("elided"), "{}", out.text);
    }

    /// A multibyte file must not panic the head/tail split.
    #[test]
    fn capping_respects_char_boundaries() {
        let text = "é".repeat(MAX_TOOL_OUTPUT_BYTES);
        let capped = cap_output(text);
        assert!(capped.contains("elided"));
        assert!(capped.len() < MAX_TOOL_OUTPUT_BYTES + 200);
    }

    #[test]
    fn oversized_lines_have_bounded_results_and_do_not_hide_later_matches() {
        let dir = TempDir::new().unwrap();
        let c = ctx(Role::SoloBuild, dir.path());
        let path = dir.path().join("huge.txt");
        let mut file = std::fs::File::create(&path).unwrap();
        use std::io::Write;
        for _ in 0..1000 {
            file.write_all(&[b'x'; 8192]).unwrap();
        }
        file.write_all(b"\nneedle after giant line\n").unwrap();
        let first = execute("read_file", &json!({"path": "huge.txt", "limit": 1}), &c).unwrap();
        assert!(first.text.len() < MAX_TOOL_OUTPUT_BYTES + 256);
        assert!(first.text.contains("remaining bytes were skipped"));
        assert!(first.text.contains("offset 2"));
        let next = execute("read_file", &json!({"path": "huge.txt", "offset": 2}), &c).unwrap();
        assert!(next.text.contains("   2|needle after giant line"));
        let search = execute("grep", &json!({"pattern": "needle"}), &c).unwrap();
        assert!(search.text.contains("huge.txt:2:needle after giant line"));
        assert!(search.text.contains("skipped 1 lines over 64000 bytes"));
        let no_hit = execute("grep", &json!({"pattern": "not present"}), &c).unwrap();
        assert!(no_hit.text.contains("results may be incomplete"));
        let edit = execute(
            "write",
            &json!({"path": "huge.txt", "content": "replacement"}),
            &c,
        );
        assert!(edit.is_err());
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            8_192_000 + b"\nneedle after giant line\n".len() as u64
        );
    }

    #[test]
    fn read_file_pages_a_long_file() {
        let dir = TempDir::new().unwrap();
        let c = ctx(Role::SoloBuild, dir.path());
        let body: String = (1..=5_000).map(|i| format!("line {i}\n")).collect();
        std::fs::write(dir.path().join("long.txt"), body).unwrap();

        let first = execute("read_file", &json!({"path": "long.txt"}), &c).unwrap();
        assert!(first.text.contains("   1|line 1"));
        assert!(first.text.contains("2000|line 2000"));
        assert!(!first.text.contains("line 2001"));
        assert!(
            first.text.contains("offset 2001"),
            "must say how to continue: {}",
            first.text
        );

        let next = execute(
            "read_file",
            &json!({"path": "long.txt", "offset": 2001, "limit": 3}),
            &c,
        )
        .unwrap();
        assert!(next.text.contains("2001|line 2001"));
        assert!(next.text.contains("2003|line 2003"));
        assert!(!next.text.contains("line 2004"));

        // A short file is returned whole, with no continuation note.
        std::fs::write(dir.path().join("short.txt"), "a\nb\n").unwrap();
        let short = execute("read_file", &json!({"path": "short.txt"}), &c).unwrap();
        assert!(!short.text.contains("more lines"), "{}", short.text);
    }

    /// 30s was below a cold build, so the auditor could not run its own
    /// allowlist. The ceiling is per-command and clamped.
    #[test]
    fn bash_timeout_is_raisable_and_clamped() {
        let dir = TempDir::new().unwrap();
        let c = ctx(Role::SoloBuild, dir.path());
        let out = execute(
            "bash",
            &json!({"command": "sleep 2", "timeout_secs": 1}),
            &c,
        )
        .unwrap();
        assert!(out.is_error);
        assert!(out.text.contains("timed out after 1s"), "{}", out.text);
        // A command inside the default budget is unaffected.
        let ok = execute("bash", &json!({"command": "echo hi"}), &c).unwrap();
        assert!(!ok.is_error);
        assert_eq!(ok.text.trim(), "hi");
    }

    /// `-lc` sourced the user's profile on every call; `-c` does not.
    #[test]
    fn bash_is_not_a_login_shell() {
        let dir = TempDir::new().unwrap();
        let c = ctx(Role::SoloBuild, dir.path());
        let out = execute(
            "bash",
            &json!({"command": "shopt -q login_shell; echo $?"}),
            &c,
        )
        .unwrap();
        assert_eq!(out.text.trim(), "1", "should not be a login shell");
    }

    /// Scratch space outside the project is written without a question, in
    /// any hat, with nobody there to ask.
    #[test]
    fn scratch_space_is_written_without_asking() {
        let dir = TempDir::new().unwrap();
        // A folder of scratch space with an ordinary name: a hidden one
        // beside the project is where tools look for configuration.
        let outside = tempfile::Builder::new()
            .prefix("ryter-scratch-")
            .tempdir()
            .unwrap();
        for role in [Role::SoloBuild, Role::SoloPlan, Role::SoloAudit] {
            let target = outside.path().join(format!("{role}.txt"));
            let args = json!({"path": target.to_string_lossy(), "content": "hi"});
            let out = gated_execute("write", &args, &ctx(role, dir.path())).unwrap();
            assert!(!out.is_error, "{role:?}: {out:?}");
            assert_eq!(std::fs::read_to_string(&target).unwrap(), "hi");
        }
    }

    /// The project's own `.env` is written with a yes every time: an
    /// approved plan, "allow all" and --always-approve don't cover it,
    /// headless says so, and what was written is never read back.
    #[test]
    fn the_projects_env_is_written_with_a_yes_every_time() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".env.example"), "DB_PASSWORD=\n").unwrap();
        let args = json!({"path": ".env", "content": "DB_PASSWORD=localdev\n"});
        let mut c = ctx(Role::SoloBuild, dir.path());
        c.always_approve = true;
        c.allowed.lock().unwrap().insert("edit".into());
        let out = gated_execute("write", &args, &c).unwrap();
        assert!(out.is_error && out.text.contains("secret file"), "{out:?}");
        assert!(!dir.path().join(".env").exists());
        let (io, rx) = crate::user_io::UserIo::pair();
        c.user_io = Some(io);
        let asked = std::thread::spawn(move || match rx.recv().unwrap() {
            crate::user_io::UserRequest::Permission { tool, reply, .. } => {
                let _ = reply.send(crate::user_io::Permission::Allow);
                tool
            }
            _ => String::new(),
        });
        let out = gated_execute("write", &args, &c).unwrap();
        assert!(!out.is_error, "{out:?}");
        assert!(
            asked.join().unwrap().ends_with(SECRET),
            "the prompt says what"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".env")).unwrap(),
            "DB_PASSWORD=localdev\n"
        );
        // Written, not read: the result names the file and its length only.
        assert!(!out.text.contains("localdev"), "{out:?}");
        let read = gated_execute("read_file", &json!({"path": ".env"}), &c).unwrap();
        assert!(read.is_error, "{read:?}");
        // No other hat is asked: it is refused.
        let out = gated_execute("write", &args, &ctx(Role::SoloPlan, dir.path())).unwrap();
        assert!(out.is_error, "{out:?}");
    }

    /// "Allow all" and --always-approve don't reach the rest of the
    /// machine: headless says so, and a person is asked each time, told
    /// where.
    #[test]
    fn outside_writes_need_a_yes_every_time() {
        let dir = TempDir::new().unwrap();
        let args = json!({"path": "/opt/ryter-not-here/scratch.txt", "content": "hi"});
        let mut c = ctx(Role::SoloBuild, dir.path());
        c.always_approve = true;
        c.allowed.lock().unwrap().insert("edit".into());
        let out = gated_execute("write", &args, &c).unwrap();
        assert!(
            out.is_error && out.text.contains("outside the project"),
            "{out:?}"
        );
        let (io, rx) = crate::user_io::UserIo::pair();
        c.user_io = Some(io);
        let asked = std::thread::spawn(move || match rx.recv().unwrap() {
            crate::user_io::UserRequest::Permission { tool, reply, .. } => {
                let _ = reply.send(crate::user_io::Permission::Deny);
                tool
            }
            _ => String::new(),
        });
        let out = gated_execute("write", &args, &c).unwrap();
        assert!(
            out.is_error && out.text.contains("denied by user"),
            "{out:?}"
        );
        assert!(
            asked.join().unwrap().ends_with(OUTSIDE),
            "the prompt says where"
        );
        // No other hat is asked: it is refused.
        let out = gated_execute("write", &args, &ctx(Role::SoloPlan, dir.path())).unwrap();
        assert!(out.is_error, "{out:?}");
    }

    /// Inline code runs in the build hat. The review hat, refused it, is
    /// told which hats run it and what it may run itself; a shell handed a
    /// command as text is refused in both, and told why.
    #[test]
    fn inline_code_runs_in_the_build_hat_and_the_refusals_say_why() {
        let dir = TempDir::new().unwrap();
        let c = ctx(Role::SoloBuild, dir.path());
        let out =
            gated_execute("bash", &json!({ "command": "python3 -c 'print(6*7)'" }), &c).unwrap();
        assert!(!out.is_error && out.text.contains("42"), "{out:?}");
        let out = gated_execute("bash", &json!({ "command": "bash -c 'echo hi'" }), &c).unwrap();
        assert!(
            out.is_error && out.text.contains("shell handed a command"),
            "{out:?}"
        );
        let r = ctx(Role::SoloAudit, dir.path());
        let out =
            gated_execute("bash", &json!({ "command": "python3 -c 'print(1)'" }), &r).unwrap();
        assert!(out.is_error && out.text.contains("build hat"), "{out:?}");
        // Other refusals keep the general wording.
        let out = gated_execute("bash", &json!({ "command": "sudo ls" }), &c).unwrap();
        assert!(out.text.contains("outside policy"), "{out:?}");
    }

    /// A web tool refused says why: off, where it is turned on; on, who
    /// may use it.
    #[test]
    fn a_web_refusal_says_off_or_whose() {
        let dir = TempDir::new().unwrap();
        let mut c = ctx(Role::SoloBuild, dir.path());
        c.web = true;
        let out = gated_execute("web_search", &json!({"query": "x"}), &c).unwrap();
        assert!(
            out.is_error && out.text.contains("plan and audit hats' tool"),
            "{out:?}"
        );
        let mut c = ctx(Role::SoloPlan, dir.path());
        c.web = false;
        let out = gated_execute("web_fetch", &json!({"url": "https://example.com"}), &c).unwrap();
        assert!(
            out.is_error
                && out.text.contains("[features] web = false")
                && out.text.contains("/settings")
                && !out.text.contains("config.toml"),
            "{out:?}"
        );
        let mut c = ctx(Role::SoloBuild, dir.path());
        c.web = false;
        let out = gated_execute("web_search", &json!({"query": "x"}), &c).unwrap();
        assert!(
            out.text.contains("/settings") && out.text.contains("plan and audit hats' tool"),
            "{out:?}"
        );
    }

    /// The build and scribe hats are refused `check_package` with a note
    /// naming the hats that ask it, not a note about its arguments.
    #[test]
    fn a_package_check_from_the_wrong_hat_names_who_asks() {
        let dir = TempDir::new().unwrap();
        for role in [Role::SoloBuild, Role::SoloScribe] {
            let c = ctx(role, dir.path());
            let out = gated_execute(
                "check_package",
                &json!({"ecosystem": "npm", "name": "fastify"}),
                &c,
            )
            .unwrap();
            assert!(
                out.is_error && out.text.contains("plan and audit hats' question"),
                "{role:?}: {out:?}"
            );
        }
    }

    /// The audit hat's write outside its own files is refused with where
    /// its findings go, and the key that changes code.
    #[test]
    fn an_audit_write_is_pointed_at_its_file() {
        let dir = TempDir::new().unwrap();
        let mut c = ctx(Role::SoloAudit, dir.path());
        c.read_only = false;
        let out = gated_execute("write", &json!({"path": "src/a.rs", "content": "x"}), &c).unwrap();
        assert!(
            out.is_error && out.text.contains("only file is audit.md"),
            "{out:?}"
        );
        assert!(out.text.contains("Shift+Tab"), "{out:?}");
        let out = gated_execute(
            "write",
            &json!({"path": ".ryter/audit.md", "content": "# Audit"}),
            &c,
        )
        .unwrap();
        assert!(!out.is_error, "{out:?}");
        assert!(dir.path().join(".ryter/audit.md").exists());
    }

    /// Yolo: every question is a yes, the rest of the machine and the
    /// project's `.env` included; a refusal is still a refusal.
    #[test]
    fn yolo_answers_every_question_and_refuses_what_is_refused() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("gone.txt"), "x").unwrap();
        let outside = TempDir::new().unwrap();
        let mut c = ctx(Role::SoloBuild, dir.path());
        c.yolo = true;
        for (name, args) in [
            ("bash", json!({"command": "rm gone.txt"})),
            ("write", json!({"path": ".env", "content": "A=1\n"})),
            (
                "write",
                json!({"path": outside.path().join("x.txt").to_string_lossy(), "content": "hi"}),
            ),
        ] {
            let out = gated_execute(name, &args, &c).unwrap();
            assert!(!out.is_error, "{name}: {out:?}");
        }
        assert!(!dir.path().join("gone.txt").exists());
        assert!(dir.path().join(".env").exists());
        assert!(outside.path().join("x.txt").exists());
        for cmd in ["sudo ls", "cat .env", "cat ~/.ssh/id_rsa"] {
            let out = gated_execute("bash", &json!({"command": cmd}), &c).unwrap();
            assert!(out.is_error, "{cmd}: {out:?}");
        }
    }

    /// The hats that look refuse a command because they look, not because
    /// it changes anything: the refusal says what they may run, and which
    /// key reaches the build hat. A `GET` of the product was refused as a
    /// command that changes things.
    #[test]
    fn the_looking_hats_say_they_only_look() {
        let dir = TempDir::new().unwrap();
        for (role, key) in [(Role::SoloScribe, "Shift+Tab"), (Role::SoloPlan, "Tab")] {
            let out = gated_execute(
                "bash",
                &json!({ "command": ".venv/bin/tasks add x" }),
                &ctx(role, dir.path()),
            )
            .unwrap();
            assert!(out.is_error, "{out:?}");
            assert!(out.text.contains("hat only looks"), "{out:?}");
            assert!(out.text.contains("curl GET/HEAD"), "{out:?}");
            assert!(out.text.contains(key), "{out:?}");
            assert!(!out.text.contains("change things"), "{out:?}");
        }
    }

    /// A reviewer refused a container command is told what does run in
    /// containers. Told only "refused", an auditor reported that the
    /// machine had no Docker, and the user was told so.
    #[test]
    fn a_refused_container_command_says_what_does_run() {
        let dir = TempDir::new().unwrap();
        let reviewer = ctx(Role::SoloAudit, dir.path());
        for (cmd, containers) in [
            ("docker compose up -d --wait", true),
            ("cd app && podman-compose build", true),
            ("npm install", false),
        ] {
            let out = gated_execute("bash", &json!({ "command": cmd }), &reviewer).unwrap();
            assert!(out.is_error, "{cmd}: {out:?}");
            assert!(
                out.text.contains("the audit hat can't run commands"),
                "{out:?}"
            );
            assert_eq!(
                out.text.contains("docker compose run --rm <service>"),
                containers,
                "{cmd}: {out:?}"
            );
        }
        // What it may run still runs.
        let out = gated_execute("bash", &json!({ "command": "true" }), &reviewer).unwrap();
        assert!(!out.is_error, "{out:?}");
    }

    #[test]
    fn a_failing_tool_is_an_error_result_not_a_dead_task() {
        let dir = TempDir::new().unwrap();
        let c = ctx(Role::SoloBuild, dir.path());
        let missing = gated_execute("read_file", &json!({"path": "not/yet.py"}), &c).unwrap();
        assert!(missing.is_error, "{missing:?}");
        std::fs::write(dir.path().join("x.pyc"), [0xff_u8, 0xfe, 0x00, 0x01]).unwrap();
        let binary = gated_execute("read_file", &json!({"path": "x.pyc"}), &c).unwrap();
        assert!(
            binary.is_error && binary.text.contains("not a text file"),
            "{binary:?}"
        );
    }

    #[test]
    fn ask_is_fail_closed_without_tui() {
        let dir = TempDir::new().unwrap();
        let c = ctx(Role::SoloBuild, dir.path());
        let out = gated_execute(
            "bash",
            &json!({"command": "rm -rf /nonexistent-ryter-ask-fixture"}),
            &c,
        )
        .unwrap();
        assert!(out.is_error);
        assert!(out.text.contains("--always-approve"), "{out:?}");
    }

    #[test]
    fn permission_allow_from_user_io() {
        let dir = TempDir::new().unwrap();
        let (io, rx) = crate::user_io::UserIo::pair();
        let mut c = ctx(Role::SoloBuild, dir.path());
        c.user_io = Some(io);
        let worker = std::thread::spawn(move || {
            gated_execute(
                "bash",
                &json!({"command": "rm -rf /nonexistent-ryter-ask-fixture"}),
                &c,
            )
            .unwrap()
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
        let c = ctx(Role::SoloPlan, dir.path());
        let out = gated_execute("ask_user", &json!({"question": "ok?"}), &c).unwrap();
        assert!(out.is_error);
        assert!(out.text.contains("TUI"), "{out:?}");
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;
    use serde_json::json;

    fn edit(root: &std::path::Path, old: &str, new: &str) -> ToolOutput {
        let c = tests::ctx(Role::SoloBuild, root);
        execute(
            "search_replace",
            &json!({"path": "f.txt", "old_string": old, "new_string": new}),
            &c,
        )
        .unwrap()
    }

    /// A miss says where to look, a repeat says which lines, a no-op is
    /// refused: each gives the model a way forward instead of a retry.
    #[test]
    fn edits_that_cannot_apply_say_why_and_where() {
        let dir = tempfile::TempDir::new().unwrap();
        let f = dir.path().join("f.txt");
        std::fs::write(&f, "fn a() {\n    one();\n}\nfn b() {\n    one();\n}\n").unwrap();
        let out = edit(dir.path(), "fn a() {\n  one();\n}", "fn a() {}");
        assert!(out.is_error);
        assert!(out.text.contains("first line is at line 1"), "{}", out.text);
        let out = edit(dir.path(), "    one();", "    two();");
        assert!(
            out.text.contains("matched 2 times (lines 2, 5)"),
            "{}",
            out.text
        );
        let out = edit(dir.path(), "fn b()", "fn b()");
        assert!(
            out.is_error && out.text.contains("the same"),
            "{}",
            out.text
        );
        let out = edit(dir.path(), "nothing like this", "x");
        assert!(out.text.contains("read it again"), "{}", out.text);
    }

    /// The person approving an edit sees the change it would make, whole:
    /// the prompt's one-line summary is cut at 160 characters.
    #[test]
    fn an_edit_asks_with_the_change_it_would_make() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "keep\nold\n").unwrap();
        let mut c = tests::ctx(Role::SoloBuild, dir.path());
        c.permissions = Arc::new(crate::permissions::Permissions {
            edit: Some(crate::permissions::Answer::Ask),
            ..Default::default()
        });
        let (io, rx) = crate::user_io::UserIo::pair();
        c.user_io = Some(io);
        let long: String = (0..40)
            .map(|i| format!("line {i} of the new text\n"))
            .collect();
        let args = json!({"path": "f.txt", "old_string": "old\n", "new_string": long});
        let t = std::thread::spawn(move || match rx.recv().unwrap() {
            crate::user_io::UserRequest::Permission { preview, reply, .. } => {
                let _ = reply.send(crate::user_io::Permission::Deny);
                preview
            }
            _ => None,
        });
        let out = gated_execute("search_replace", &args, &c).unwrap();
        assert!(out.is_error, "denied");
        let d = t.join().unwrap().expect("a preview");
        assert_eq!((d.added, d.removed), (40, 1));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "keep\nold\n"
        );
    }

    /// `read_file` shows a CRLF file without its `\r`; an edit quoting what
    /// it showed still applies, and the file keeps its line endings.
    #[test]
    fn an_edit_quoted_from_a_crlf_file_applies_and_keeps_crlf() {
        let dir = tempfile::TempDir::new().unwrap();
        let f = dir.path().join("f.txt");
        std::fs::write(&f, "one\r\ntwo\r\nthree\r\n").unwrap();
        let out = edit(dir.path(), "one\ntwo\n", "one\n2\n");
        assert!(!out.is_error, "{}", out.text);
        assert_eq!(
            std::fs::read_to_string(&f).unwrap(),
            "one\r\n2\r\nthree\r\n"
        );
        let d = out.diff.expect("an edit carries its diff");
        assert_eq!((d.added, d.removed, d.path.as_str()), (1, 1, "f.txt"));
    }

    /// A project reached through a symlink still shows relative paths.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_project_shows_relative_paths() {
        let real = tempfile::TempDir::new().unwrap();
        let links = tempfile::TempDir::new().unwrap();
        let link = links.path().join("ws");
        std::os::unix::fs::symlink(real.path(), &link).unwrap();
        std::fs::write(real.path().join("f.txt"), "a\n").unwrap();
        let out = edit(&link, "a\n", "b\n");
        assert_eq!(out.diff.expect("a diff").path, "f.txt");
    }
}

#[cfg(test)]
mod allow_tests {
    use super::*;
    use serde_json::json;

    /// `a` allows the kind of action it named, for the session, and nothing
    /// else: it used to allow every later call, destructive ones included.
    #[test]
    fn allow_for_the_session_covers_only_what_it_named() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut c = tests::ctx(Role::SoloBuild, dir.path());
        // `mkdir` runs by default; a rule of the user's makes it a question.
        let mut rules = crate::permissions::Permissions::default();
        rules
            .bash
            .insert("mkdir *".into(), crate::permissions::Answer::Ask);
        c.permissions = Arc::new(rules);
        let (io, rx) = crate::user_io::UserIo::pair();
        c.user_io = Some(io);
        let answers = std::thread::spawn(move || {
            let mut asked = Vec::new();
            while let Ok(req) = rx.recv() {
                if let crate::user_io::UserRequest::Permission {
                    summary,
                    strict,
                    scope,
                    reply,
                    ..
                } = req
                {
                    asked.push((summary.clone(), strict, scope.clone()));
                    let _ = reply.send(if scope.is_some() {
                        crate::user_io::Permission::Always
                    } else {
                        crate::user_io::Permission::Deny
                    });
                }
            }
            asked
        });
        let run = |cmd: &str| gated_execute("bash", &json!({"command": cmd}), &c).unwrap();
        run("mkdir build");
        run("mkdir -p build");
        run("mkdir other");
        run("rm -rf other");
        drop(c);
        let asked = answers.join().unwrap();
        let summaries: Vec<&str> = asked.iter().map(|(s, _, _)| s.as_str()).collect();
        // The second `mkdir build` is covered by the first `a`; `mkdir
        // other` is another kind; `rm` asks, strict, with no `a`.
        assert_eq!(
            summaries,
            vec!["mkdir build", "mkdir other", "rm -rf other"],
            "{asked:?}"
        );
        assert_eq!(asked[0].2.as_deref(), Some("`mkdir build` commands"));
        assert!(asked[2].1, "rm is strict");
        assert_eq!(asked[2].2, None, "no allow-for-session on destruction");
    }

    #[test]
    fn scopes_name_what_a_would_allow() {
        assert_eq!(
            allow_scope("bash", &json!({"command": "cargo test --workspace"})),
            Some(("bash:cargo test".into(), "`cargo test` commands".into()))
        );
        assert_eq!(
            allow_scope("search_replace", &json!({"path": "a.rs"})).map(|s| s.1),
            Some("edits to files in the project".into())
        );
        assert_eq!(
            allow_scope("bash", &json!({"command": "rm -rf target"})),
            None
        );
        assert!(strict_prompt("bash", &json!({"command": "git clean -fdx"})));
        assert!(!strict_prompt("bash", &json!({"command": "cargo test"})));
    }
}
