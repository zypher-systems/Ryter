//! Outbound MCP client: spawn stdio servers, search_tool / use_tool.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};

use serde_json::{Value, json};

use crate::config::McpServerConfig;
use crate::error::{Error, Result};
use crate::mcp::rpc::{RpcRequest, RpcResponse};

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
}

/// Connected outbound servers.
#[derive(Debug, Default)]
pub struct McpHub {
    next_id: AtomicI64,
    servers: BTreeMap<String, LiveServer>,
    tools: Vec<RemoteTool>,
}

struct LiveServer {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl std::fmt::Debug for LiveServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveServer").finish_non_exhaustive()
    }
}

impl Drop for McpHub {
    fn drop(&mut self) {
        for (_, mut s) in std::mem::take(&mut self.servers) {
            let _ = s.child.kill();
        }
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
                continue;
            }
            if let Err(e) = hub.spawn(name, cfg) {
                eprintln!("mcp: skip {name}: {e}");
            }
        }
        Ok(hub)
    }

    fn spawn(&mut self, name: &str, cfg: &McpServerConfig) -> Result<()> {
        let mut cmd = Command::new(&cfg.command);
        cmd.args(&cfg.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // Do not inherit API keys unless the server config asks.
        cmd.env_remove("XAI_API_KEY");
        cmd.env_remove("OPENROUTER_API_KEY");
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
        let mut live = LiveServer {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        };
        let init = RpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: "initialize".into(),
            params: Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "ryter", "version": crate::VERSION }
            })),
        };
        let _ = rpc_roundtrip(&mut live, init)?;
        let notify = RpcRequest {
            jsonrpc: "2.0".into(),
            id: None,
            method: "notifications/initialized".into(),
            params: None,
        };
        live.stdin
            .write_all(notify.to_line().as_bytes())
            .map_err(|e| Error::Io(e.to_string()))?;
        let listed = rpc_roundtrip(
            &mut live,
            RpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(json!(2)),
                method: "tools/list".into(),
                params: None,
            },
        )?;
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
                });
            }
        }
        self.servers.insert(name.to_string(), live);
        let _ = self.next_id.fetch_add(3, Ordering::Relaxed);
        Ok(())
    }

    /// Catalog keys matching `query` (case-insensitive substring).
    pub fn search(&self, query: &str) -> Vec<&RemoteTool> {
        let q = query.to_ascii_lowercase();
        self.tools
            .iter()
            .filter(|t| {
                t.key.to_ascii_lowercase().contains(&q)
                    || t.description.to_ascii_lowercase().contains(&q)
            })
            .collect()
    }

    /// Call `server__tool` with JSON arguments.
    pub fn call(&mut self, key: &str, arguments: Value) -> Result<String> {
        let tool = self
            .tools
            .iter()
            .find(|t| t.key == key)
            .cloned()
            .ok_or_else(|| Error::Config(format!("unknown MCP tool {key}")))?;
        let live = self
            .servers
            .get_mut(&tool.server)
            .ok_or_else(|| Error::Config(format!("server {} not connected", tool.server)))?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let result = rpc_roundtrip(
            live,
            RpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(json!(id)),
                method: "tools/call".into(),
                params: Some(json!({
                    "name": tool.name,
                    "arguments": arguments
                })),
            },
        )?;
        if let Some(arr) = result.get("content").and_then(Value::as_array) {
            let text: Vec<&str> = arr
                .iter()
                .filter_map(|c| c.get("text").and_then(Value::as_str))
                .collect();
            return Ok(text.join("\n"));
        }
        Ok(result.to_string())
    }
}

fn rpc_roundtrip(live: &mut LiveServer, req: RpcRequest) -> Result<Value> {
    live.stdin
        .write_all(req.to_line().as_bytes())
        .map_err(|e| Error::Io(e.to_string()))?;
    live.stdin.flush().map_err(|e| Error::Io(e.to_string()))?;
    let mut line = String::new();
    live.stdout
        .read_line(&mut line)
        .map_err(|e| Error::Io(e.to_string()))?;
    let resp: RpcResponse =
        serde_json::from_str(&line).map_err(|e| Error::Io(format!("mcp reply: {e} {line}")))?;
    if let Some(err) = resp.error {
        return Err(Error::Config(format!("mcp {}: {}", err.code, err.message)));
    }
    Ok(resp.result.unwrap_or(json!({})))
}

/// Format search_tool output.
pub fn search_tools(hub: &McpHub, query: &str) -> String {
    let hits = hub.search(query);
    if hits.is_empty() {
        return "no MCP tools matched".into();
    }
    hits.iter()
        .map(|t| format!("{} — {}", t.key, t.description))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Call use_tool.
pub fn use_tool(hub: &mut McpHub, key: &str, arguments: Value) -> Result<String> {
    hub.call(key, arguments)
}
