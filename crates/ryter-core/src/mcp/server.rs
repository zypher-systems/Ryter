//! Inbound MCP server: Ryter as a tool other agents can call. Echo server for tests.

use std::io::{BufRead, Write};

use serde_json::{Value, json};

use crate::VERSION;
use crate::error::{Error, Result};
use crate::mcp::rpc::{RpcRequest, RpcResponse};

/// Snapshot for `ryter_status`.
#[derive(Debug, Clone, Default)]
pub struct StatusSnapshot {
    /// Model id.
    pub model: String,
    /// Connection name.
    pub connection: String,
    /// Session id.
    pub session: String,
    /// Last error, if any.
    pub last_error: String,
}

/// Host the inbound tools talk to (the session and its agent).
pub trait InboundHost: Send + Sync {
    /// Prepare a reserved prompt before the dispatcher accepts cancellation.
    /// Hosts that reset a shared cancel flag must do it here, not in `prompt`.
    fn prepare_prompt(&self) {}
    /// Run one user turn: a normal user message, worked on in the build hat.
    fn prompt(&self, text: &str) -> Result<String>;
    /// Status line.
    fn status(&self) -> StatusSnapshot;
    /// Spend summary (no secrets).
    fn spend(&self) -> String;
    /// Cancel in-flight work. Safe to call while [`prompt`](Self::prompt) is running.
    fn cancel(&self);
    /// Cancel only this connection's reserved prompt on disconnect or write failure.
    /// Queued hosts should distinguish it from unrelated interactive work.
    fn cancel_prompt(&self) {
        self.cancel();
    }
}

// A host can be shared by many socket sessions. Reserve it before spawning a
// prompt so a rejected connection never owns (or cancels) somebody else's turn.
// The permit borrows the host, keeping its data address valid until release.
static PROMPT_OWNERS: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());

struct PromptPermit<'a> {
    host: &'a dyn InboundHost,
}

impl<'a> PromptPermit<'a> {
    fn acquire(host: &'a dyn InboundHost) -> Option<Self> {
        let key = host as *const dyn InboundHost as *const () as usize;
        let mut owners = PROMPT_OWNERS.lock().unwrap_or_else(|e| e.into_inner());
        if owners.contains(&key) {
            return None;
        }
        owners.push(key);
        Some(Self { host })
    }
}

impl Drop for PromptPermit<'_> {
    fn drop(&mut self) {
        let key = self.host as *const dyn InboundHost as *const () as usize;
        PROMPT_OWNERS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|owner| *owner != key);
    }
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
    fn prompt(&self, text: &str) -> Result<String> {
        if let Ok(mut g) = self.last.lock() {
            *g = text.to_string();
        }
        Ok(text.to_string())
    }
    fn status(&self) -> StatusSnapshot {
        StatusSnapshot {
            model: "echo".into(),
            connection: "echo".into(),
            session: "echo".into(),
            last_error: String::new(),
        }
    }
    fn spend(&self) -> String {
        "$0.00".into()
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
            "description": "Send a user message to Ryter. It is worked on in the build hat, in the project Ryter was started in.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string" }
                },
                "required": ["text"]
            }
        },
        {
            "name": "ryter_status",
            "description": "Model, connection, session id.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "ryter_spend",
            "description": "Spend summary. Never includes API keys.",
            "inputSchema": { "type": "object", "properties": {} }
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
            host.prompt(text)
        }
        "ryter_status" => {
            let s = host.status();
            Ok(format!(
                "model={} connection={} session={}",
                s.model, s.connection, s.session
            ))
        }
        "ryter_spend" => Ok(host.spend()),
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

/// One JSON-RPC line session. Blocking host requests run on bounded workers;
/// cancellation and authentication stay on the connection thread. Hosts must
/// cooperate with cancellation and reject overlapping turns across connections.
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
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::time::Duration;

    const MAX_PENDING: usize = 8;
    let (line_tx, line_rx) = mpsc::sync_channel::<String>(64);
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
    let send = |writer: &mut W, resp: RpcResponse| -> Result<()> {
        writer
            .write_all(resp.to_line().as_bytes())
            .and_then(|()| writer.flush())
            .map_err(|e| Error::Io(e.to_string()))
    };
    // Keep the permit outside the scope: error paths cancel and join the
    // old worker before another connection can claim the same host.
    let mut permit = None;
    let result = std::thread::scope(|scope| {
        let (done_tx, done_rx) = mpsc::channel();
        let mut workers: Vec<std::thread::ScopedJoinHandle<'_, ()>> = Vec::new();
        let mut authed = tokens.is_empty();
        let mut pending = 0;
        let mut prompting = false;
        let mut closed = false;
        let result = (|| {
            loop {
                while let Ok((prompt, response)) = done_rx.try_recv() {
                    pending -= 1;
                    if prompt {
                        prompting = false;
                        permit = None;
                    }
                    if let Some(resp) = response {
                        send(&mut writer, resp)?;
                    }
                }
                let mut i = 0;
                while i < workers.len() {
                    if workers[i].is_finished() {
                        workers
                            .swap_remove(i)
                            .join()
                            .map_err(|_| Error::Io("MCP request worker panicked".into()))?;
                    } else {
                        i += 1;
                    }
                }
                if closed {
                    if pending == 0 {
                        return Ok(());
                    }
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                }
                let line = match line_rx.recv_timeout(Duration::from_millis(20)) {
                    Ok(line) => line,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => {
                        closed = true;
                        if prompting {
                            host.cancel_prompt();
                        }
                        continue;
                    }
                };
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
                if !authed {
                    let got = req.params.as_ref().and_then(|p| {
                        p.get("token")
                            .or_else(|| p.get("bearer"))
                            .and_then(Value::as_str)
                    });
                    if req.method == "initialize"
                        && got.is_some_and(|g| tokens.iter().any(|t| token_eq(t, g)))
                    {
                        authed = true;
                    } else {
                        if let Some(id) = req.id.clone() {
                            send(&mut writer, RpcResponse::err(id, -32001, "unauthorized"))?;
                        }
                        if req.method == "initialize" {
                            return Err(Error::Config("unauthorized".into()));
                        }
                        continue;
                    }
                }
                let prompt = is_prompt(&req);
                // Only host-dependent methods can block. Keep cancel, ping,
                // discovery and initialization available even at capacity.
                let blocking = req.method == "resources/read"
                    || (req.method == "tools/call" && !is_cancel_call(&req));
                if !blocking {
                    if let Some(resp) = handle(host, &req) {
                        send(&mut writer, resp)?;
                    }
                } else if pending >= MAX_PENDING || (prompt && prompting) {
                    if let Some(id) = req.id {
                        send(
                            &mut writer,
                            RpcResponse::err(
                                id,
                                -32000,
                                "busy; retry after the current request finishes",
                            ),
                        )?;
                    }
                } else {
                    if prompt {
                        permit = PromptPermit::acquire(host);
                        if permit.is_none() {
                            if let Some(id) = req.id {
                                send(
                                    &mut writer,
                                    RpcResponse::err(
                                        id,
                                        -32000,
                                        "busy; a prompt is already running",
                                    ),
                                )?;
                            }
                            continue;
                        }
                        host.prepare_prompt();
                    }
                    pending += 1;
                    prompting |= prompt;
                    let done_tx = done_tx.clone();
                    workers.push(scope.spawn(move || {
                        let response = handle(host, &req);
                        let _ = done_tx.send((prompt, response));
                    }));
                }
            }
        })();
        if prompting {
            host.cancel_prompt();
        }
        result
    });
    drop(permit);
    result
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

    /// This host deliberately shares the prompt lock with status/spend, like
    /// the old CLI host. A blocked observer must not hold up cancellation.
    struct BlockingHost {
        agent: std::sync::Mutex<()>,
        started: std::sync::mpsc::Sender<()>,
        stopped: std::sync::mpsc::Sender<()>,
        release: std::sync::mpsc::Sender<()>,
        wait: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }

    impl InboundHost for BlockingHost {
        fn prompt(&self, _: &str) -> Result<String> {
            let _agent = self.agent.lock().unwrap();
            self.started.send(()).unwrap();
            // A failed regression releases itself instead of hanging the suite.
            self.wait
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(3))
                .map_err(|_| Error::Config("cancellation did not arrive".into()))?;
            Ok("stopped".into())
        }
        fn status(&self) -> StatusSnapshot {
            let _agent = self.agent.lock().unwrap();
            StatusSnapshot::default()
        }
        fn spend(&self) -> String {
            let _agent = self.agent.lock().unwrap();
            "$0.00".into()
        }
        fn cancel(&self) {
            let _ = self.stopped.send(());
            let _ = self.release.send(());
        }
    }

    fn tool(id: u64, name: &str) -> RpcRequest {
        RpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(id)),
            method: "tools/call".into(),
            params: Some(json!({"name": name, "arguments": {"text": "go"}})),
        }
    }

    #[test]
    fn cancellation_survives_blocked_observers_overlapping_prompts_and_disconnect() {
        use std::net::{Shutdown, TcpListener, TcpStream};
        use std::sync::mpsc;
        use std::time::Duration;
        for ending in ["ryter_cancel", "notification", "disconnect"] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let server = listener.accept().unwrap().0;
            let (started, started_rx) = mpsc::channel();
            let (stopped, stopped_rx) = mpsc::channel();
            let (release, wait) = mpsc::channel();
            let host = std::sync::Arc::new(BlockingHost {
                agent: Default::default(),
                started,
                stopped,
                release,
                wait: std::sync::Mutex::new(wait),
            });
            let owner_host = host.clone();
            let worker = std::thread::spawn(move || {
                serve_session(
                    server.try_clone().unwrap(),
                    server,
                    owner_host.as_ref(),
                    &[],
                )
            });
            client
                .write_all(tool(1, "ryter_prompt").to_line().as_bytes())
                .unwrap();
            started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            let mut second = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            second
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let second_server = listener.accept().unwrap().0;
            let rejected = std::thread::spawn(move || {
                serve_session(
                    second_server.try_clone().unwrap(),
                    second_server,
                    host.as_ref(),
                    &[],
                )
            });
            second
                .write_all(tool(99, "ryter_prompt").to_line().as_bytes())
                .unwrap();
            second.shutdown(Shutdown::Write).unwrap();
            let mut second_reply = String::new();
            std::io::Read::read_to_string(&mut second, &mut second_reply).unwrap();
            rejected.join().unwrap().unwrap();
            let response: RpcResponse = serde_json::from_str(second_reply.trim()).unwrap();
            assert_eq!(response.id, json!(99));
            assert!(response.error.unwrap().message.contains("busy"));
            assert!(
                stopped_rx.try_recv().is_err(),
                "rejected client cancelled the owner's prompt"
            );

            for req in [
                tool(2, "ryter_status"),
                tool(3, "ryter_spend"),
                tool(4, "ryter_prompt"),
            ] {
                client.write_all(req.to_line().as_bytes()).unwrap();
            }
            let mut reader = std::io::BufReader::new(client.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let busy: RpcResponse = serde_json::from_str(&line).unwrap();
            assert_eq!(busy.id, json!(4));
            assert!(busy.error.unwrap().message.contains("busy"));
            // Fill every worker with blocked resource reads; cancellation must
            // still get through once the dispatcher starts returning busy.
            for id in 5..14 {
                let req = RpcRequest {
                    jsonrpc: "2.0".into(),
                    id: Some(json!(id)),
                    method: "resources/read".into(),
                    params: Some(json!({"uri": "ryter://session/spend"})),
                };
                client.write_all(req.to_line().as_bytes()).unwrap();
            }
            match ending {
                "disconnect" => client.shutdown(Shutdown::Write).unwrap(),
                "notification" => client
                    .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\"}\n")
                    .unwrap(),
                _ => client
                    .write_all(tool(14, "ryter_cancel").to_line().as_bytes())
                    .unwrap(),
            }
            let cancelled = stopped_rx.recv_timeout(Duration::from_secs(1));
            if ending != "disconnect" {
                client.shutdown(Shutdown::Write).unwrap();
            }
            let result = worker.join().unwrap();
            assert!(cancelled.is_ok(), "{ending} was blocked");
            result.unwrap();
            let mut rest = String::new();
            std::io::Read::read_to_string(&mut reader, &mut rest).unwrap();
            let replies: Vec<RpcResponse> = rest
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect();
            assert!(
                replies
                    .iter()
                    .any(|r| r.id == json!(1) && r.result.is_some())
            );
        }
    }

    #[test]
    fn authentication_precedes_cancel_and_worker_dispatch() {
        let host = EchoHost::default();
        let input = format!(
            "{}{}",
            tool(1, "ryter_prompt").to_line(),
            RpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(json!(2)),
                method: "initialize".into(),
                params: Some(json!({"token": "wrong"}))
            }
            .to_line()
        );
        let mut output = Vec::new();
        assert!(
            serve_session(
                std::io::Cursor::new(input),
                &mut output,
                &host,
                &["fixture".into()]
            )
            .is_err()
        );
        assert_eq!(host.last(), "");
        assert!(String::from_utf8(output).unwrap().lines().all(|line| {
            serde_json::from_str::<RpcResponse>(line)
                .unwrap()
                .error
                .unwrap()
                .code
                == -32001
        }));
    }

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
        assert!(text.contains("model=echo"));
        assert!(!text.contains("phase"));
    }
}
