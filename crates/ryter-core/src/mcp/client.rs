//! Outbound MCP client: spawn stdio servers, search_tool / use_tool.
//!
//! Each server's stdout is read by its own thread into a channel, so a call
//! can wait with a deadline and a cancel, and pick its reply out by id.
//! Reading the next line and calling it the reply hung forever on a server
//! that never answered, and a notification shifted every later result by one.

use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::cancel::Cancel;
use crate::config::McpServerConfig;
use crate::error::{Error, Result};
use crate::mcp::rpc::{RpcRequest, RpcResponse};

/// How long a server has to start and list its tools. `npx` fetching a
/// package on first run takes a while.
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a tool call may run when the server's config doesn't say.
pub const CALL_TIMEOUT_SECS: u64 = 120;
/// How much of a server's stderr is kept, for when it exits.
const STDERR_TAIL: usize = 2_000;

/// A tool discovered on a connected server.
#[derive(Debug, Clone)]
pub struct RemoteTool {
    /// `server__tool` catalog key.
    pub key: String,
    /// Server config name.
    pub server: String,
    /// Raw MCP tool name.
    pub name: String,
    /// Description.
    pub description: String,
    /// The tool's arguments, `name: type` (`?` when optional).
    pub args: Vec<String>,
}

/// Connected outbound servers.
#[derive(Debug, Default)]
pub struct McpHub {
    servers: BTreeMap<String, Arc<Mutex<LiveServer>>>,
    tools: Vec<RemoteTool>,
    /// Per-server outcome of the last connect: `connected (N tools)` or the error text.
    status: BTreeMap<String, String>,
}

/// One running server. Its lock is held for a call; the hub's is not, so a
/// slow server doesn't stall calls to the others.
pub struct LiveServer {
    name: String,
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    stderr: Arc<Mutex<VecDeque<u8>>>,
    /// The server's stderr reached its end: the tail is all it said.
    stderr_done: Arc<AtomicBool>,
    next_id: i64,
    timeout: Duration,
}

impl std::fmt::Debug for LiveServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveServer")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl Drop for LiveServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl McpHub {
    /// Empty hub (no servers).
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawn enabled servers. Env is filtered (no default API keys).
    pub fn connect(servers: &BTreeMap<String, McpServerConfig>) -> Result<Self> {
        let mut hub = Self::new();
        for (name, cfg) in servers {
            if !cfg.enabled {
                hub.status.insert(name.clone(), "disabled".into());
                continue;
            }
            match hub.spawn(name, cfg) {
                Ok(()) => {
                    let n = hub.tools.iter().filter(|t| t.server == *name).count();
                    hub.status
                        .insert(name.clone(), format!("connected · {n} tools"));
                }
                Err(e) => {
                    hub.status.insert(name.clone(), format!("error: {e}"));
                }
            }
        }
        Ok(hub)
    }

    /// Per-server status text from the last connect (`R-POP-58`).
    pub fn status(&self) -> &BTreeMap<String, String> {
        &self.status
    }

    fn spawn(&mut self, name: &str, cfg: &McpServerConfig) -> Result<()> {
        let mut cmd = Command::new(&cfg.command);
        cmd.args(&cfg.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Do not inherit API keys unless the server config asks: the
        // well-known ones, and every variable a key was read from.
        cmd.env_remove("XAI_API_KEY");
        cmd.env_remove("OPENROUTER_API_KEY");
        for var in crate::tools::shell::hidden_vars() {
            cmd.env_remove(var);
        }
        for (k, v) in &cfg.env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().map_err(|e| Error::Io(e.to_string()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Io("mcp stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Io("mcp stdout".into()))?;
        let (tx, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        // Drained so a chatty server never blocks on a full pipe; the tail
        // explains an exit.
        let stderr = Arc::new(Mutex::new(VecDeque::new()));
        let stderr_done = Arc::new(AtomicBool::new(true));
        if let Some(mut err) = child.stderr.take() {
            let tail = stderr.clone();
            let done = stderr_done.clone();
            done.store(false, Ordering::Release);
            std::thread::spawn(move || {
                let mut buf = [0u8; 1024];
                while let Ok(n) = err.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    if let Ok(mut t) = tail.lock() {
                        t.extend(&buf[..n]);
                        let over = t.len().saturating_sub(STDERR_TAIL);
                        t.drain(..over);
                    }
                }
                done.store(true, Ordering::Release);
            });
        }
        let mut live = LiveServer {
            name: name.to_string(),
            child,
            stdin,
            lines,
            stderr,
            stderr_done,
            next_id: 1,
            timeout: Duration::from_secs(cfg.timeout_secs.unwrap_or(CALL_TIMEOUT_SECS)),
        };
        live.request(
            "initialize",
            Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "ryter", "version": crate::VERSION }
            })),
            START_TIMEOUT,
            None,
        )?;
        live.send(&RpcRequest {
            jsonrpc: "2.0".into(),
            id: None,
            method: "notifications/initialized".into(),
            params: None,
        })?;
        let listed = live.request("tools/list", None, START_TIMEOUT, None)?;
        if let Some(arr) = listed.get("tools").and_then(Value::as_array) {
            for t in arr {
                let raw = t.get("name").and_then(Value::as_str).unwrap_or("");
                if raw.is_empty() {
                    continue;
                }
                self.tools.push(RemoteTool {
                    key: format!("{name}__{raw}"),
                    server: name.to_string(),
                    name: raw.to_string(),
                    description: t
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    args: arg_list(t.get("inputSchema")),
                });
            }
        }
        self.servers
            .insert(name.to_string(), Arc::new(Mutex::new(live)));
        Ok(())
    }

    /// Tools whose key or description has every word of `query`
    /// (case-insensitive). A whole-phrase match found nothing for
    /// "simulated logging" when the tool was `toggle-simulated-logging`.
    pub fn search(&self, query: &str) -> Vec<&RemoteTool> {
        let words: Vec<String> = query
            .split(|c: char| c.is_whitespace() || c == '-' || c == '_')
            .filter(|w| !w.is_empty())
            .map(str::to_ascii_lowercase)
            .collect();
        self.tools
            .iter()
            .filter(|t| {
                let hay = format!("{} {}", t.key, t.description).to_ascii_lowercase();
                words.iter().all(|w| hay.contains(w.as_str()))
            })
            .collect()
    }

    /// The tool behind `server__tool` and the server that runs it, so the
    /// call can be made without holding the hub.
    pub fn route(&self, key: &str) -> Result<(RemoteTool, Arc<Mutex<LiveServer>>)> {
        let tool = self
            .tools
            .iter()
            .find(|t| t.key == key)
            .cloned()
            .ok_or_else(|| Error::Mcp(format!("unknown MCP tool {key}; search_tool lists them")))?;
        let server =
            self.servers.get(&tool.server).cloned().ok_or_else(|| {
                Error::Mcp(format!("MCP server {} is not connected", tool.server))
            })?;
        Ok((tool, server))
    }

    /// Call `server__tool` with JSON arguments.
    pub fn call(&self, key: &str, arguments: Value) -> Result<String> {
        let (tool, server) = self.route(key)?;
        call_tool(&server, &tool, arguments, None)
    }
}

/// Run `tool` on `server`, until it answers, its timeout, or `cancel`.
pub fn call_tool(
    server: &Mutex<LiveServer>,
    tool: &RemoteTool,
    arguments: Value,
    cancel: Option<&Cancel>,
) -> Result<String> {
    let mut live = server
        .lock()
        .map_err(|_| Error::Mcp(format!("MCP server {} failed mid-call", tool.server)))?;
    let timeout = live.timeout;
    let result = live.request(
        "tools/call",
        Some(json!({ "name": tool.name, "arguments": arguments })),
        timeout,
        cancel,
    )?;
    let text = match result.get("content").and_then(Value::as_array) {
        Some(arr) => arr
            .iter()
            .filter_map(|c| c.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        None => result.to_string(),
    };
    // The tool ran and failed: the model should see it as a failure.
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        return Err(Error::Mcp(format!("{} failed: {text}", tool.key)));
    }
    Ok(text)
}

impl LiveServer {
    fn send(&mut self, msg: &impl serde::Serialize) -> Result<()> {
        let mut line = serde_json::to_string(msg).map_err(|e| Error::Io(e.to_string()))?;
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|()| self.stdin.flush())
            .map_err(|_| self.gone())
    }

    /// The server's stdout closed: say so, with what it last wrote to stderr.
    fn gone(&mut self) -> Error {
        // Its stdout closing can reach us before the reader has the last of
        // its stderr, which says why it stopped ("token expired"). Give the
        // reader a moment to finish; a server that closed stdout but lives on
        // doesn't hold the error up for long.
        let until = Instant::now() + Duration::from_millis(500);
        while Instant::now() < until
            && !(self.stderr_done.load(Ordering::Acquire)
                && self.child.try_wait().is_ok_and(|s| s.is_some()))
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        let status = match self.child.try_wait() {
            Ok(Some(s)) => format!(" ({s})"),
            _ => String::new(),
        };
        let tail = self
            .stderr
            .lock()
            .map(|t| String::from_utf8_lossy(&t.iter().copied().collect::<Vec<u8>>()).into_owned())
            .unwrap_or_default();
        let tail = tail.trim();
        let said = if tail.is_empty() {
            String::new()
        } else {
            format!(": {}", tail.lines().last().unwrap_or(tail))
        };
        Error::Mcp(format!(
            "MCP server {} exited{status}{said}. `/mcp` reconnects it.",
            self.name
        ))
    }

    /// Send a request and wait for the reply with its id. Notifications and
    /// replies to earlier, abandoned requests are skipped; requests from the
    /// server are answered, since some servers wait on them.
    fn request(
        &mut self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
        cancel: Option<&Cancel>,
    ) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&RpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(id)),
            method: method.into(),
            params,
        })?;
        let deadline = Instant::now() + timeout;
        loop {
            if cancel.is_some_and(Cancel::is_cancelled) {
                self.abandon(id, "cancelled");
                return Err(Error::Cancelled);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                self.abandon(id, "timed out");
                return Err(Error::Mcp(format!(
                    "MCP server {} didn't answer {method} within {}s. If it needs \
                     longer, set `timeout_secs` under [servers.{}] in ~/.ryter/mcp.toml.",
                    self.name,
                    timeout.as_secs(),
                    self.name,
                )));
            }
            let line = match self
                .lines
                .recv_timeout(left.min(Duration::from_millis(100)))
            {
                Ok(l) => l,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return Err(self.gone()),
            };
            // Servers that log to stdout break the protocol; skip their noise.
            let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };
            if let Some(m) = msg.get("method").and_then(Value::as_str) {
                if let Some(rid) = msg.get("id") {
                    let reply = if m == "ping" {
                        RpcResponse::ok(rid.clone(), json!({}))
                    } else {
                        RpcResponse::err(rid.clone(), -32601, format!("ryter doesn't support {m}"))
                    };
                    self.send(&reply)?;
                }
                continue;
            }
            if msg.get("id") != Some(&json!(id)) {
                continue;
            }
            let resp: RpcResponse = serde_json::from_value(msg).map_err(|e| {
                Error::Mcp(format!("MCP server {} sent a bad reply: {e}", self.name))
            })?;
            if let Some(err) = resp.error {
                return Err(Error::Mcp(format!(
                    "MCP server {} refused {method}: {} ({})",
                    self.name, err.message, err.code
                )));
            }
            return Ok(resp.result.unwrap_or(json!({})));
        }
    }

    /// Tell the server Ryter stopped waiting on `id`. Its late reply, if any,
    /// is skipped by the id check.
    fn abandon(&mut self, id: i64, reason: &str) {
        let _ = self.send(&RpcRequest {
            jsonrpc: "2.0".into(),
            id: None,
            method: "notifications/cancelled".into(),
            params: Some(json!({ "requestId": id, "reason": reason })),
        });
    }
}

/// `duration: number`, `steps?: number` from a tool's JSON Schema. Without
/// them the model could only guess a tool's arguments.
fn arg_list(schema: Option<&Value>) -> Vec<String> {
    let Some(props) = schema
        .and_then(|s| s.get("properties"))
        .and_then(Value::as_object)
    else {
        return Vec::new();
    };
    let required: Vec<&str> = schema
        .and_then(|s| s.get("required"))
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    props
        .iter()
        .map(|(name, p)| {
            let ty = p.get("type").and_then(Value::as_str).unwrap_or("any");
            let opt = if required.contains(&name.as_str()) {
                ""
            } else {
                "?"
            };
            format!("{name}{opt}: {ty}")
        })
        .collect()
}

/// Format search_tool output.
pub fn search_tools(hub: &McpHub, query: &str) -> String {
    let hits = hub.search(query);
    if hits.is_empty() {
        return "no MCP tools matched".into();
    }
    hits.iter()
        .map(|t| {
            if t.args.is_empty() {
                format!("{} — {}", t.key, t.description)
            } else {
                format!(
                    "{} — {}\n  arguments: {}",
                    t.key,
                    t.description,
                    t.args.join(", ")
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Call use_tool.
pub fn use_tool(hub: &McpHub, key: &str, arguments: Value) -> Result<String> {
    hub.call(key, arguments)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server scripted in `sh`: it prints `out` (one JSON message per
    /// line, `SLEEP n` to pause) and copies what Ryter sends to `seen`.
    fn server(dir: &std::path::Path, out: &[&str], timeout_secs: Option<u64>) -> McpServerConfig {
        let seen = dir.join("seen");
        // A background job's stdin is /dev/null unless it is handed another fd.
        let mut script = format!("exec 3<&0\ncat <&3 > '{}' &\n", seen.display());
        for l in out {
            match l.strip_prefix("SLEEP ") {
                Some(n) => script.push_str(&format!("sleep {n}\n")),
                None => script.push_str(&format!("printf '%s\\n' '{l}'\n")),
            }
        }
        script.push_str("sleep 30\n");
        McpServerConfig {
            command: "sh".into(),
            args: vec!["-c".into(), script],
            enabled: true,
            env: BTreeMap::new(),
            timeout_secs,
        }
    }

    const INIT: &str =
        r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{}}}"#;
    const LIST: &str = r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo","description":"says it back","inputSchema":{"type":"object","properties":{"message":{"type":"string"},"loud":{"type":"boolean"}},"required":["message"]}}]}}"#;

    fn hub(out: &[&str], timeout_secs: Option<u64>) -> (tempfile::TempDir, McpHub) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = server(dir.path(), out, timeout_secs);
        let hub = McpHub::connect(&[("fake".to_string(), cfg)].into()).unwrap();
        assert_eq!(
            hub.status()["fake"],
            "connected · 1 tools",
            "{:?}",
            hub.status()
        );
        (dir, hub)
    }

    /// A reply is the message with the request's id: log lines,
    /// notifications, a stale reply, and a server's own request come first.
    #[test]
    fn the_reply_is_the_one_with_the_request_id() {
        let (dir, hub) = hub(
            &[
                "starting up...",
                INIT,
                r#"{"jsonrpc":"2.0","method":"notifications/message","params":{"level":"info","data":"ready"}}"#,
                LIST,
                r#"{"jsonrpc":"2.0","method":"notifications/progress","params":{"progress":1}}"#,
                r#"{"jsonrpc":"2.0","id":99,"result":{"content":[{"type":"text","text":"stale"}]}}"#,
                r#"{"jsonrpc":"2.0","id":"s1","method":"ping"}"#,
                r#"{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"hello"}]}}"#,
            ],
            None,
        );
        assert_eq!(use_tool(&hub, "fake__echo", json!({})).unwrap(), "hello");
        assert_eq!(
            search_tools(&hub, "echo"),
            "fake__echo — says it back\n  arguments: loud?: boolean, message: string"
        );
        // The ping was answered (the script's `cat` may lag a moment).
        let answered = r#""id":"s1","result":{}"#;
        let mut seen = String::new();
        for _ in 0..40 {
            seen = std::fs::read_to_string(dir.path().join("seen")).unwrap_or_default();
            if seen.contains(answered) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(seen.contains(answered), "{seen}");
    }

    /// A server that never answers used to hang the turn for good.
    #[test]
    fn a_silent_server_times_out() {
        let (_dir, hub) = hub(&[INIT, LIST], Some(1));
        let t = Instant::now();
        let err = use_tool(&hub, "fake__echo", json!({})).unwrap_err();
        assert!(t.elapsed() < Duration::from_secs(5));
        assert!(
            err.to_string()
                .contains("didn't answer tools/call within 1s"),
            "{err}"
        );
        assert!(
            err.to_string()
                .contains("[servers.fake] in ~/.ryter/mcp.toml"),
            "{err}"
        );
    }

    #[test]
    fn cancel_stops_the_wait() {
        let (_dir, hub) = hub(&[INIT, LIST], None);
        let (tool, server) = hub.route("fake__echo").unwrap();
        let cancel = Cancel::new();
        let c = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            c.cancel();
        });
        let t = Instant::now();
        let err = call_tool(&server, &tool, json!({}), Some(&cancel)).unwrap_err();
        assert!(matches!(err, Error::Cancelled), "{err}");
        assert!(t.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn a_server_that_exits_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = server(dir.path(), &[INIT, LIST], None);
        cfg.args[1] = format!(
            "printf '%s\\n%s\\n' '{INIT}' '{LIST}'; read _; read _; read _; echo 'token expired' >&2; exit 2"
        );
        let hub = McpHub::connect(&[("fake".to_string(), cfg)].into()).unwrap();
        let err = use_tool(&hub, "fake__echo", json!({}))
            .unwrap_err()
            .to_string();
        assert!(err.contains("MCP server fake exited"), "{err}");
        assert!(err.contains("token expired"), "{err}");
    }

    /// The reason a server gives on its way out reaches the error even
    /// when it writes it after its stdout has closed. CI caught the reader
    /// behind the exit, and the reason was lost.
    #[test]
    fn a_server_that_exits_is_heard_to_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = server(dir.path(), &[INIT, LIST], None);
        cfg.args[1] = format!(
            "printf '%s\\n%s\\n' '{INIT}' '{LIST}'; read _; read _; read _; exec 1>&-; \
             sleep 0.2; echo 'token expired' >&2; exit 2"
        );
        let hub = McpHub::connect(&[("fake".to_string(), cfg)].into()).unwrap();
        let err = use_tool(&hub, "fake__echo", json!({}))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("exited (exit status: 2): token expired"),
            "{err}"
        );
    }

    /// `isError` is the tool failing, not a result.
    #[test]
    fn a_tool_error_is_an_error() {
        let (_dir, hub) = hub(
            &[
                INIT,
                LIST,
                r#"{"jsonrpc":"2.0","id":3,"result":{"isError":true,"content":[{"type":"text","text":"no such repo"}]}}"#,
            ],
            None,
        );
        let err = use_tool(&hub, "fake__echo", json!({}))
            .unwrap_err()
            .to_string();
        assert!(err.contains("no such repo"), "{err}");
    }

    #[test]
    fn search_matches_every_word_anywhere() {
        let mut hub = McpHub::new();
        hub.tools.push(RemoteTool {
            key: "everything__toggle-simulated-logging".into(),
            server: "everything".into(),
            name: "toggle-simulated-logging".into(),
            description: "Toggles simulated, random-leveled logging on or off.".into(),
            args: Vec::new(),
        });
        for q in [
            "simulated logging",
            "everything logging",
            "Toggle",
            "logging-toggle",
        ] {
            assert_eq!(hub.search(q).len(), 1, "{q}");
        }
        assert!(hub.search("simulated weather").is_empty());
    }
}
