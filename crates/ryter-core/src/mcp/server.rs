//! Inbound MCP server: Ryter as a tool other agents can call. Echo server for tests.

use std::io::{BufRead, Write};

use serde_json::{Value, json};

use crate::VERSION;
use crate::error::{Error, Result};
use crate::mcp::rpc::{RpcRequest, RpcResponse};
use crate::phase::Phase;

/// Snapshot for `ryter_status`.
#[derive(Debug, Clone, Default)]
pub struct StatusSnapshot {
    /// Phase name.
    pub phase: String,
    /// Model id.
    pub model: String,
    /// Connection name.
    pub connection: String,
    /// Session id.
    pub session: String,
    /// Last error, if any.
    pub last_error: String,
}

/// Host the inbound tools talk to (session + orchestrator).
pub trait InboundHost: Send + Sync {
    /// Run one user turn.
    fn prompt(&self, text: &str, phase: Option<Phase>) -> Result<String>;
    /// Status line.
    fn status(&self) -> StatusSnapshot;
    /// Spend summary (no secrets).
    fn spend(&self) -> String;
    /// Switch phase.
    fn set_phase(&self, phase: Phase, note: &str) -> Result<()>;
    /// Cancel in-flight work. Safe to call while [`prompt`](Self::prompt) is running.
    fn cancel(&self);
}

/// In-memory echo host (also used by `ryter mcp echo`).
#[derive(Debug, Default)]
pub struct EchoHost {
    last: std::sync::Mutex<String>,
}

impl EchoHost {
    /// Last `ryter_prompt` text.
    pub fn last(&self) -> String {
        self.last.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

impl InboundHost for EchoHost {
    fn prompt(&self, text: &str, _phase: Option<Phase>) -> Result<String> {
        if let Ok(mut g) = self.last.lock() {
            *g = text.to_string();
        }
        Ok(text.to_string())
    }
    fn status(&self) -> StatusSnapshot {
        StatusSnapshot {
            phase: "build".into(),
            model: "echo".into(),
            connection: "echo".into(),
            session: "echo".into(),
            last_error: String::new(),
        }
    }
    fn spend(&self) -> String {
        "$0.00".into()
    }
    fn set_phase(&self, _phase: Phase, _note: &str) -> Result<()> {
        Ok(())
    }
    fn cancel(&self) {}
}

/// Dispatch one MCP request. Notifications return `None`.
pub fn handle(host: &dyn InboundHost, req: &RpcRequest) -> Option<RpcResponse> {
    let id = req.id.clone();
    match req.method.as_str() {
        "notifications/initialized" => None,
        "notifications/cancelled" => {
            host.cancel();
            None
        }
        "initialize" => Some(RpcResponse::ok(
            id.unwrap_or(json!(null)),
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {}, "resources": {} },
                "serverInfo": { "name": "ryter", "version": VERSION }
            }),
        )),
        "ping" => Some(RpcResponse::ok(id.unwrap_or(json!(null)), json!({}))),
        "tools/list" => Some(RpcResponse::ok(
            id.unwrap_or(json!(null)),
            json!({ "tools": inbound_tools() }),
        )),
        "tools/call" => {
            let id = id.unwrap_or(json!(null));
            let params = req.params.clone().unwrap_or(json!({}));
            match call_tool(host, &params) {
                Ok(text) => Some(RpcResponse::ok(
                    id,
                    json!({ "content": [{ "type": "text", "text": text }] }),
                )),
                Err(e) => Some(RpcResponse::err(id, -32000, e.to_string())),
            }
        }
        "resources/list" => Some(RpcResponse::ok(
            id.unwrap_or(json!(null)),
            json!({
                "resources": [
                    { "uri": "ryter://session/transcript", "name": "transcript" },
                    { "uri": "ryter://session/spend", "name": "spend" }
                ]
            }),
        )),
        "resources/read" => {
            let id = id.unwrap_or(json!(null));
            let uri = req
                .params
                .as_ref()
                .and_then(|p| p.get("uri"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let text = match uri {
                "ryter://session/spend" => host.spend(),
                "ryter://session/transcript" => host.status().session,
                _ => format!("unknown resource {uri}"),
            };
            Some(RpcResponse::ok(
                id,
                json!({ "contents": [{ "uri": uri, "mimeType": "text/plain", "text": text }] }),
            ))
        }
        other => Some(RpcResponse::err(
            id.unwrap_or(json!(null)),
            -32601,
            format!("method not found: {other}"),
        )),
    }
}

fn inbound_tools() -> Value {
    json!([
        {
            "name": "ryter_prompt",
            "description": "Send a user turn to the Ryter orchestrator.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string" },
                    "phase": { "type": "string" }
                },
                "required": ["text"]
            }
        },
        {
            "name": "ryter_status",
            "description": "Phase, model, session id.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "ryter_spend",
            "description": "Spend summary. Never includes API keys.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "ryter_set_phase",
            "description": "Switch orchestrator phase.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "phase": { "type": "string" },
                    "note": { "type": "string" }
                },
                "required": ["phase"]
            }
        },
        {
            "name": "ryter_cancel",
            "description": "Cancel the in-flight turn.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "echo",
            "description": "Echo a message (test helper).",
            "inputSchema": {
                "type": "object",
                "properties": { "message": { "type": "string" } },
                "required": ["message"]
            }
        }
    ])
}

fn call_tool(host: &dyn InboundHost, params: &Value) -> Result<String> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    match name {
        "echo" => Ok(args
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()),
        "ryter_prompt" => {
            let text = args
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Config("ryter_prompt: missing text".into()))?;
            let phase = args
                .get("phase")
                .and_then(Value::as_str)
                .map(|s| s.parse())
                .transpose()?;
            host.prompt(text, phase)
        }
        "ryter_status" => {
            let s = host.status();
            Ok(format!(
                "phase={} model={} connection={} session={}",
                s.phase, s.model, s.connection, s.session
            ))
        }
        "ryter_spend" => Ok(host.spend()),
        "ryter_set_phase" => {
            let phase: Phase = args
                .get("phase")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Config("missing phase".into()))?
                .parse()?;
            let note = args.get("note").and_then(Value::as_str).unwrap_or("");
            host.set_phase(phase, note)?;
            Ok(format!("phase {phase}"))
        }
        "ryter_cancel" => {
            host.cancel();
            Ok("cancelled".into())
        }
        other => Err(Error::Config(format!("unknown tool {other}"))),
    }
}

/// Serve MCP on stdio until stdin closes. Logs go to stderr.
pub fn serve_inbound(host: &dyn InboundHost) -> Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    serve_session(stdin, stdout, host, &[])
}

/// Compare a bearer token without leaking its length or first difference
/// through timing. Not a big win over a local socket, but it costs one line.
fn token_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// One JSON-RPC line session. `ryter_prompt` runs on a helper thread so the same
/// connection can send `ryter_cancel` / `notifications/cancelled` while it is in flight.
pub fn serve_session<R, W>(
    reader: R,
    mut writer: W,
    host: &dyn InboundHost,
    tokens: &[String],
) -> Result<()>
where
    R: std::io::Read + Send + 'static,
    W: Write,
{
    let (line_tx, line_rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let reader = std::io::BufReader::new(reader);
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    if line_tx.send(l).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    let mut authed = tokens.is_empty();
    while let Ok(line) = line_rx.recv() {
        if line.trim().is_empty() {
            continue;
        }
        let req: RpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("mcp: bad json: {e}");
                continue;
            }
        };
        if !tokens.is_empty() && !authed {
            if req.method == "initialize" {
                let got = req.params.as_ref().and_then(|p| {
                    p.get("token")
                        .or_else(|| p.get("bearer"))
                        .and_then(Value::as_str)
                });
                let ok = got.is_some_and(|g| tokens.iter().any(|t| token_eq(t, g)));
                if !ok {
                    if let Some(id) = req.id.clone() {
                        let resp = RpcResponse::err(id, -32001, "unauthorized");
                        writer
                            .write_all(resp.to_line().as_bytes())
                            .map_err(|e| Error::Io(e.to_string()))?;
                        writer.flush().map_err(|e| Error::Io(e.to_string()))?;
                    }
                    return Err(Error::Config("unauthorized".into()));
                }
                authed = true;
            } else {
                if let Some(id) = req.id.clone() {
                    let resp = RpcResponse::err(id, -32001, "unauthorized");
                    writer
                        .write_all(resp.to_line().as_bytes())
                        .map_err(|e| Error::Io(e.to_string()))?;
                    writer.flush().map_err(|e| Error::Io(e.to_string()))?;
                }
                continue;
            }
        }
        if is_prompt(&req) {
            let args = req
                .params
                .clone()
                .unwrap_or(json!({}))
                .get("arguments")
                .cloned()
                .unwrap_or(json!({}));
            let id = req.id.clone().unwrap_or(json!(null));
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            // Prompt may block; cancel arrives on `line_rx`.
            let text = args
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let phase = args
                .get("phase")
                .and_then(Value::as_str)
                .map(|s| s.parse::<Phase>().ok())
                .unwrap_or(None);
            std::thread::scope(|s| {
                s.spawn(|| {
                    let r = host.prompt(&text, phase);
                    let _ = done_tx.send(r);
                });
                loop {
                    match done_rx.try_recv() {
                        Ok(result) => {
                            let resp = match result {
                                Ok(text) => RpcResponse::ok(
                                    id.clone(),
                                    json!({ "content": [{ "type": "text", "text": text }] }),
                                ),
                                Err(e) => RpcResponse::err(id.clone(), -32000, e.to_string()),
                            };
                            if writer.write_all(resp.to_line().as_bytes()).is_err() {
                                return;
                            }
                            let _ = writer.flush();
                            return;
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            match line_rx.recv_timeout(std::time::Duration::from_millis(20)) {
                                Ok(line) => {
                                    if let Ok(extra) = serde_json::from_str::<RpcRequest>(&line) {
                                        if extra.method == "notifications/cancelled"
                                            || is_cancel_call(&extra)
                                        {
                                            host.cancel();
                                        }
                                        if let Some(resp) = handle(host, &extra) {
                                            let _ = writer.write_all(resp.to_line().as_bytes());
                                            let _ = writer.flush();
                                        }
                                    }
                                }
                                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                                    let _ = done_rx.recv();
                                    return;
                                }
                            }
                        }
                    }
                }
            });
            continue;
        }
        if let Some(resp) = handle(host, &req) {
            writer
                .write_all(resp.to_line().as_bytes())
                .map_err(|e| Error::Io(e.to_string()))?;
            writer.flush().map_err(|e| Error::Io(e.to_string()))?;
        }
    }
    Ok(())
}

fn is_prompt(req: &RpcRequest) -> bool {
    if req.method != "tools/call" {
        return false;
    }
    req.params
        .as_ref()
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        == Some("ryter_prompt")
}

fn is_cancel_call(req: &RpcRequest) -> bool {
    if req.method != "tools/call" {
        return false;
    }
    req.params
        .as_ref()
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        == Some("ryter_cancel")
}

/// Echo-only MCP server (tests / `ryter mcp echo`).
pub fn serve_echo() -> Result<()> {
    let host = EchoHost::default();
    serve_inbound(&host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::rpc::RpcRequest;

    #[test]
    fn list_and_echo() {
        let host = EchoHost::default();
        let list = handle(
            &host,
            &RpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(json!(1)),
                method: "tools/list".into(),
                params: None,
            },
        )
        .unwrap();
        let tools = list.result.unwrap();
        let names: Vec<_> = tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert!(names.contains(&"ryter_prompt"));
        assert!(names.contains(&"ryter_spend"));
        let call = handle(
            &host,
            &RpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(json!(2)),
                method: "tools/call".into(),
                params: Some(json!({"name":"echo","arguments":{"message":"hi"}})),
            },
        )
        .unwrap();
        assert_eq!(call.result.unwrap()["content"][0]["text"], "hi");
    }

    #[test]
    fn prompt_and_status() {
        let host = EchoHost::default();
        let r = handle(
            &host,
            &RpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(json!(3)),
                method: "tools/call".into(),
                params: Some(json!({"name":"ryter_prompt","arguments":{"text":"hello"}})),
            },
        )
        .unwrap();
        assert_eq!(r.result.unwrap()["content"][0]["text"], "hello");
        let s = handle(
            &host,
            &RpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(json!(4)),
                method: "tools/call".into(),
                params: Some(json!({"name":"ryter_status","arguments":{}})),
            },
        )
        .unwrap();
        let result = s.result.unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("phase=build"));
    }
}
