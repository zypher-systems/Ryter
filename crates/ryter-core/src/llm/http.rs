//! Live HTTP provider.

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::time::Duration;

use crate::config::ConnectionConfig;
use crate::error::{Error, Result};
use crate::llm::parse::{Backend, parse_sse};
use crate::llm::{
    CompletionRequest, DeltaStream, ModelInfo, Provider, StreamDelta, ToolSpec, backend_for,
};

/// How long to wait for the connection itself.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a stream may go silent before it is considered dead.
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);
/// Attempts after the first for a retryable failure.
const MAX_RETRIES: u32 = 3;
/// First backoff step; doubles per attempt.
const BACKOFF_BASE: Duration = Duration::from_millis(500);
/// Ceiling on one backoff wait.
const BACKOFF_CAP: Duration = Duration::from_secs(20);

/// Statuses worth trying again: rate limits, overload, and gateway noise.
/// A 400 or 401 will not change on a second attempt.
fn is_retryable_status(code: u16) -> bool {
    matches!(code, 408 | 425 | 429 | 500 | 502 | 503 | 504 | 529)
}

/// Transport failures that are worth another attempt.
fn is_retryable_error(e: &reqwest::Error) -> bool {
    e.is_timeout() || e.is_connect() || e.is_request()
}

/// `Retry-After` in seconds, when the provider sent one.
fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let v = headers.get("retry-after")?.to_str().ok()?;
    v.trim()
        .parse::<u64>()
        .ok()
        .map(|s| Duration::from_secs(s.min(BACKOFF_CAP.as_secs())))
}

/// Exponential backoff with a little jitter, so parallel specialists that hit
/// the same rate limit do not retry in lockstep.
fn backoff(attempt: u32) -> Duration {
    let step = BACKOFF_BASE
        .saturating_mul(1u32 << attempt.min(5))
        .min(BACKOFF_CAP);
    let jitter = Duration::from_millis(u64::from(jitter_ms()));
    step.saturating_add(jitter)
}

/// Cheap jitter source; avoids taking a dependency on `rand` for 250ms.
fn jitter_ms() -> u16 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    (nanos % 250) as u16
}

/// reqwest-backed provider for SpaceXAI, OpenRouter, and generic endpoints.
pub struct HttpProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    backend: Backend,
    kind: String,
    http_referer: Option<String>,
    x_title: Option<String>,
}

impl HttpProvider {
    /// New client. Does not touch the network until `stream` / `list_models`.
    pub fn new(conn: &ConnectionConfig, api_key: String) -> Self {
        Self {
            client: reqwest::Client::builder()
                // A streamed turn has no useful total deadline: a long agentic
                // turn is legitimate, a stalled socket is not. Bound the gap
                // between chunks instead of the whole request.
                .connect_timeout(CONNECT_TIMEOUT)
                .read_timeout(IDLE_TIMEOUT)
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            base_url: conn.base_url.trim_end_matches('/').to_string(),
            api_key,
            backend: backend_for(conn),
            kind: conn.kind.clone(),
            http_referer: conn.http_referer.clone(),
            x_title: conn.x_title.clone(),
        }
    }

    fn headers(&self) -> Result<HeaderMap> {
        let mut h = HeaderMap::new();
        h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        match self.backend {
            Backend::Messages => {
                let key = HeaderValue::from_str(&self.api_key)
                    .map_err(|e| Error::Provider(e.to_string()))?;
                h.insert("x-api-key", key);
                h.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
            }
            _ => {
                let auth = format!("Bearer {}", self.api_key);
                h.insert(
                    AUTHORIZATION,
                    HeaderValue::from_str(&auth).map_err(|e| Error::Provider(e.to_string()))?,
                );
            }
        }
        if self.kind == "openrouter" {
            if let Some(r) = &self.http_referer {
                if let Ok(v) = HeaderValue::from_str(r) {
                    h.insert("HTTP-Referer", v);
                }
            }
            if let Some(t) = &self.x_title {
                if let Ok(v) = HeaderValue::from_str(t) {
                    h.insert("X-Title", v.clone());
                    h.insert("X-OpenRouter-Title", v);
                }
            }
        }
        Ok(h)
    }

    fn endpoint(&self) -> String {
        match self.backend {
            Backend::Responses => format!("{}/responses", self.base_url),
            Backend::Messages => format!("{}/messages", self.base_url),
            Backend::ChatCompletions => format!("{}/chat/completions", self.base_url),
        }
    }

    fn body(&self, req: &CompletionRequest) -> Value {
        match self.backend {
            Backend::ChatCompletions => chat_body(req),
            Backend::Responses => responses_body(req),
            Backend::Messages => messages_body(req),
        }
    }
}

#[async_trait]
impl Provider for HttpProvider {
    async fn stream(&self, req: CompletionRequest) -> Result<DeltaStream> {
        let headers = self.headers()?;
        let body = self.body(&req);
        let mut last = String::new();
        // Only the opening request is retried. Once deltas have been handed to
        // the caller, a retry would duplicate text they already have.
        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                tokio::time::sleep(backoff(attempt - 1)).await;
            }
            let sent = self
                .client
                .post(self.endpoint())
                .headers(headers.clone())
                .json(&body)
                .send()
                .await;
            let resp = match sent {
                Ok(r) => r,
                Err(e) if is_retryable_error(&e) && attempt < MAX_RETRIES => {
                    last = e.to_string();
                    continue;
                }
                Err(e) => return Err(Error::Provider(e.to_string())),
            };
            let status = resp.status();
            if status.is_success() {
                return Ok(Box::pin(sse_delta_stream(
                    self.backend,
                    resp.bytes_stream(),
                )));
            }
            let wait = retry_after(resp.headers());
            let text = resp.text().await.unwrap_or_default();
            last = format!("http {status}: {text}");
            if !is_retryable_status(status.as_u16()) || attempt == MAX_RETRIES {
                return Err(Error::Provider(last));
            }
            if let Some(w) = wait {
                tokio::time::sleep(w).await;
            }
        }
        Err(Error::Provider(format!(
            "{last} (after {MAX_RETRIES} retries)"
        )))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let url = format!("{}/models", self.base_url);
        let resp = self
            .client
            .get(url)
            .headers(self.headers()?)
            .send()
            .await
            .map_err(|e| Error::Provider(e.to_string()))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| Error::Provider(e.to_string()))?;
        if !status.is_success() {
            return Err(Error::Provider(format!("http {status}: {text}")));
        }
        parse_models_json(&text)
    }
}

fn sse_delta_stream(
    backend: Backend,
    byte_stream: impl Stream<Item = std::result::Result<Bytes, reqwest::Error>> + Send + Unpin + 'static,
) -> impl Stream<Item = Result<StreamDelta>> + Send {
    futures_util::stream::unfold(
        StreamState {
            backend,
            byte_stream,
            buf: String::new(),
            pending: VecDeque::new(),
            done: false,
        },
        |mut st| async move {
            loop {
                if let Some(d) = st.pending.pop_front() {
                    return Some((d, st));
                }
                if st.done {
                    return None;
                }
                match st.byte_stream.next().await {
                    Some(Ok(chunk)) => {
                        st.buf
                            .push_str(&String::from_utf8_lossy(&chunk).replace("\r\n", "\n"));
                        drain_sse(&mut st);
                    }
                    Some(Err(e)) => {
                        st.done = true;
                        return Some((Err(Error::Provider(e.to_string())), st));
                    }
                    None => {
                        if !st.buf.trim().is_empty() {
                            drain_rest(&mut st);
                        }
                        st.done = true;
                        if let Some(d) = st.pending.pop_front() {
                            return Some((d, st));
                        }
                        return None;
                    }
                }
            }
        },
    )
}

struct StreamState<S> {
    backend: Backend,
    byte_stream: S,
    buf: String,
    pending: VecDeque<Result<StreamDelta>>,
    done: bool,
}

fn drain_sse<S>(st: &mut StreamState<S>) {
    while let Some(idx) = st.buf.find("\n\n") {
        let block = st.buf[..idx + 2].to_string();
        st.buf = st.buf[idx + 2..].to_string();
        push_block(st, &block);
    }
}

fn drain_rest<S>(st: &mut StreamState<S>) {
    let rest = std::mem::take(&mut st.buf);
    if !rest.trim().is_empty() {
        push_block(st, &rest);
    }
}

fn push_block<S>(st: &mut StreamState<S>, block: &str) {
    match parse_sse(st.backend, block) {
        Ok(mut ds) => {
            if matches!(ds.last(), Some(StreamDelta::Done)) {
                ds.pop();
            }
            for d in ds {
                st.pending.push_back(Ok(d));
            }
        }
        Err(e) => st.pending.push_back(Err(e)),
    }
}

fn chat_body(req: &CompletionRequest) -> Value {
    let mut messages = Vec::new();
    if let Some(sys) = &req.system {
        messages.push(json!({"role": "system", "content": sys}));
    }
    for m in &req.messages {
        let mut obj = json!({"role": m.role, "content": m.content});
        if let Some(id) = &m.tool_call_id {
            obj["tool_call_id"] = json!(id);
        }
        if let Some(calls) = &m.tool_calls {
            obj["tool_calls"] = json!(
                calls
                    .iter()
                    .map(|c| json!({
                        "id": c.id,
                        "type": "function",
                        "function": { "name": c.name, "arguments": c.arguments }
                    }))
                    .collect::<Vec<_>>()
            );
        }
        messages.push(obj);
    }
    let mut body = json!({
        "model": req.model,
        "messages": messages,
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    if let Some(max) = req.max_tokens {
        body["max_tokens"] = json!(max);
    }
    if !req.tools.is_empty() {
        body["tools"] = json!(tools_openai(&req.tools));
    }
    body
}

/// Responses API body.
///
/// Tool calls and their results are their own `input` items (`function_call` /
/// `function_call_output`), not fields on a message, and there is no `tool`
/// role. Flattening them into `{role, content}` silently drops the agentic
/// loop, so every call is emitted as an item keyed by `call_id`.
fn responses_body(req: &CompletionRequest) -> Value {
    let mut input = Vec::new();
    for m in &req.messages {
        if m.role == "tool" {
            input.push(json!({
                "type": "function_call_output",
                "call_id": m.tool_call_id.clone().unwrap_or_default(),
                "output": m.content,
            }));
            continue;
        }
        // An assistant turn that only called tools carries no text.
        if !m.content.is_empty() || m.role != "assistant" {
            input.push(json!({"role": m.role, "content": m.content}));
        }
        for c in m.tool_calls.iter().flatten() {
            input.push(json!({
                "type": "function_call",
                "call_id": c.id,
                "name": c.name,
                "arguments": c.arguments,
            }));
        }
    }
    let mut body = json!({
        "model": req.model,
        "input": input,
        "stream": true,
    });
    if let Some(sys) = &req.system {
        body["instructions"] = json!(sys);
    }
    if let Some(max) = req.max_tokens {
        body["max_output_tokens"] = json!(max);
    }
    if !req.tools.is_empty() {
        body["tools"] = json!(tools_responses(&req.tools));
    }
    body
}

/// Anthropic Messages body.
///
/// Tool use and tool results are content blocks, not message fields: the call
/// is a `tool_use` block on the assistant turn, the result is a `tool_result`
/// block on a *user* turn. There is no `tool` role, `system` is top-level, and
/// an empty content list is rejected.
fn messages_body(req: &CompletionRequest) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    // Messages has no system role. Specialists carry their role prompt as a
    // `system` message, so dropping those silently ran every specialist on
    // this backend without its prompt. Fold them into the top-level field.
    let mut system: Vec<&str> = req.system.iter().map(String::as_str).collect();
    for m in &req.messages {
        match m.role.as_str() {
            "system" => {
                if !m.content.trim().is_empty() {
                    system.push(&m.content);
                }
            }
            "tool" => {
                let block = json!({
                    "type": "tool_result",
                    "tool_use_id": m.tool_call_id.clone().unwrap_or_default(),
                    "content": m.content,
                });
                // Results for one assistant turn share a single user turn.
                match messages.last_mut() {
                    Some(prev) if is_tool_result_turn(prev) => {
                        if let Some(arr) = prev["content"].as_array_mut() {
                            arr.push(block);
                        }
                    }
                    _ => messages.push(json!({"role": "user", "content": [block]})),
                }
            }
            "assistant" => {
                let mut blocks = Vec::new();
                if !m.content.is_empty() {
                    blocks.push(json!({"type": "text", "text": m.content}));
                }
                for c in m.tool_calls.iter().flatten() {
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": c.id,
                        "name": c.name,
                        "input": serde_json::from_str::<Value>(&c.arguments)
                            .unwrap_or_else(|_| json!({})),
                    }));
                }
                if !blocks.is_empty() {
                    messages.push(json!({"role": "assistant", "content": blocks}));
                }
            }
            _ => messages.push(json!({"role": m.role, "content": m.content})),
        }
    }
    let mut body = json!({
        "model": req.model,
        "messages": messages,
        "stream": true,
        "max_tokens": req.max_tokens.unwrap_or(8192),
    });
    // The system prompt and tool schemas are byte-identical on every turn of an
    // agent loop, so without a cache breakpoint they are re-billed each time.
    // `cached_tokens` was already parsed and shown; nothing ever asked for it.
    if !system.is_empty() {
        body["system"] = json!([{
            "type": "text",
            "text": system.join("\n\n"),
            "cache_control": { "type": "ephemeral" },
        }]);
    }
    if !req.tools.is_empty() {
        let mut tools = tools_anthropic(&req.tools);
        // One breakpoint covers everything before it, so it goes on the last
        // tool: system + all tools become the cached prefix.
        if let Some(last) = tools.as_array_mut().and_then(|a| a.last_mut()) {
            last["cache_control"] = json!({ "type": "ephemeral" });
        }
        body["tools"] = tools;
    }
    body
}

/// True when `msg` is a user turn built only of `tool_result` blocks, so a
/// sibling result from the same assistant turn can join it.
fn is_tool_result_turn(msg: &Value) -> bool {
    msg.get("role").and_then(Value::as_str) == Some("user")
        && msg
            .get("content")
            .and_then(Value::as_array)
            .is_some_and(|a| {
                !a.is_empty()
                    && a.iter()
                        .all(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
            })
}

/// Responses advertises tools flat, not nested under `function`.
fn tools_responses(tools: &[ToolSpec]) -> Value {
    tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "name": t.name,
                "description": t.description,
                "parameters": t.parameters,
            })
        })
        .collect()
}

fn tools_openai(tools: &[ToolSpec]) -> Value {
    tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                }
            })
        })
        .collect()
}

fn tools_anthropic(tools: &[ToolSpec]) -> Value {
    tools
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.parameters,
            })
        })
        .collect()
}

fn parse_models_json(text: &str) -> Result<Vec<ModelInfo>> {
    let v: Value =
        serde_json::from_str(text).map_err(|e| Error::Provider(format!("models json: {e}")))?;
    let data = v
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Provider("models: missing data[]".into()))?;
    Ok(data
        .iter()
        .filter_map(|m| {
            let id = m.get("id").and_then(Value::as_str)?;
            let context_length = m
                .get("context_length")
                .or_else(|| m.get("context_window"))
                .and_then(Value::as_u64);
            let pricing = m.get("pricing");
            let input_per_million = pricing
                .and_then(|p| p.get("prompt").or_else(|| p.get("input")))
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<f64>().ok())
                .map(|per_token| per_token * 1_000_000.0);
            let output_per_million = pricing
                .and_then(|p| p.get("completion").or_else(|| p.get("output")))
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<f64>().ok())
                .map(|per_token| per_token * 1_000_000.0);
            Some(ModelInfo {
                id: id.to_string(),
                context_length,
                input_per_million,
                output_per_million,
                connection: None,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::llm::{AssistantToolCall, Message};

    fn msg(role: &str, content: &str) -> Message {
        Message {
            role: role.into(),
            content: content.into(),
            tool_call_id: None,
            tool_calls: None,
        }
    }

    fn call(id: &str, name: &str, arguments: &str) -> AssistantToolCall {
        AssistantToolCall {
            id: id.into(),
            name: name.into(),
            arguments: arguments.into(),
        }
    }

    /// user → assistant(tool_call) → tool result → assistant(text).
    fn tool_loop() -> CompletionRequest {
        CompletionRequest {
            model: "m".into(),
            system: Some("sys".into()),
            messages: vec![
                msg("user", "read it"),
                Message {
                    tool_calls: Some(vec![call("call_1", "read_file", r#"{"path":"a.rs"}"#)]),
                    ..msg("assistant", "")
                },
                Message {
                    tool_call_id: Some("call_1".into()),
                    ..msg("tool", "fn main() {}")
                },
                msg("assistant", "done"),
            ],
            tools: vec![ToolSpec {
                name: "read_file".into(),
                description: "read".into(),
                parameters: json!({"type": "object"}),
            }],
            max_tokens: Some(64),
        }
    }

    /// Every backend must round-trip the call id, name, arguments, and result.
    /// Dropping any of them breaks the agent loop on the second iteration.
    #[test]
    fn every_backend_round_trips_a_tool_loop() {
        let req = tool_loop();
        for (label, body) in [
            ("chat_completions", chat_body(&req)),
            ("responses", responses_body(&req)),
            ("messages", messages_body(&req)),
        ] {
            let wire = serde_json::to_string(&body).expect("serialize");
            // Messages parses arguments into an object, so match on the
            // argument's content rather than the raw JSON string.
            for needle in ["call_1", "read_file", "path", "a.rs", "fn main() {}"] {
                assert!(wire.contains(needle), "{label} lost {needle}: {wire}");
            }
        }
    }

    #[test]
    fn responses_emits_call_items_and_flat_tools() {
        let body = responses_body(&tool_loop());
        let input = body["input"].as_array().expect("input array");
        let kinds: Vec<&str> = input
            .iter()
            .map(|i| {
                i.get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_else(|| i["role"].as_str().unwrap_or("?"))
            })
            .collect();
        // The text-free assistant turn collapses into its `function_call`.
        assert_eq!(
            kinds,
            ["user", "function_call", "function_call_output", "assistant"]
        );
        assert_eq!(input[1]["call_id"], "call_1");
        assert_eq!(input[2]["call_id"], "call_1");
        assert_eq!(input[2]["output"], "fn main() {}");
        // Responses has no `tool` role and no nested `function` object.
        assert!(!kinds.contains(&"tool"));
        assert_eq!(body["tools"][0]["name"], "read_file");
        assert!(body["tools"][0].get("function").is_none());
    }

    #[test]
    fn messages_emits_tool_use_and_tool_result_blocks() {
        let body = messages_body(&tool_loop());
        let ms = body["messages"].as_array().expect("messages array");
        let roles: Vec<&str> = ms.iter().map(|m| m["role"].as_str().unwrap()).collect();
        // The tool result becomes a user turn; there is no `tool` role.
        assert_eq!(roles, ["user", "assistant", "user", "assistant"]);
        assert_eq!(ms[1]["content"][0]["type"], "tool_use");
        assert_eq!(ms[1]["content"][0]["id"], "call_1");
        // `input` is an object, not the raw argument string.
        assert_eq!(ms[1]["content"][0]["input"]["path"], "a.rs");
        assert_eq!(ms[2]["content"][0]["type"], "tool_result");
        assert_eq!(ms[2]["content"][0]["tool_use_id"], "call_1");
        // System is a cacheable text block now, not a bare string.
        assert_eq!(body["system"][0]["text"], "sys");
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        // One breakpoint on the last tool covers system + every tool.
        assert_eq!(
            body["tools"].as_array().unwrap().last().unwrap()["cache_control"]["type"],
            "ephemeral"
        );
    }

    /// Parallel calls answer into one user turn (Anthropic rejects a bare
    /// `tool_result` turn per result).
    #[test]
    fn messages_merges_parallel_tool_results() {
        let req = CompletionRequest {
            model: "m".into(),
            system: None,
            messages: vec![
                msg("user", "both"),
                Message {
                    tool_calls: Some(vec![call("c1", "grep", "{}"), call("c2", "glob", "{}")]),
                    ..msg("assistant", "")
                },
                Message {
                    tool_call_id: Some("c1".into()),
                    ..msg("tool", "hit one")
                },
                Message {
                    tool_call_id: Some("c2".into()),
                    ..msg("tool", "hit two")
                },
            ],
            tools: vec![],
            max_tokens: None,
        };
        let body = messages_body(&req);
        let ms = body["messages"].as_array().unwrap();
        assert_eq!(ms.len(), 3, "results should share one user turn: {ms:?}");
        assert_eq!(ms[1]["content"].as_array().unwrap().len(), 2);
        assert_eq!(ms[2]["content"].as_array().unwrap().len(), 2);
        assert_eq!(ms[2]["content"][1]["tool_use_id"], "c2");
    }

    /// An empty content list is rejected by the API, and `system` is a
    /// top-level field rather than a message.
    #[test]
    fn messages_drops_empty_assistant_and_lifts_system_messages() {
        let req = CompletionRequest {
            model: "m".into(),
            system: None,
            messages: vec![
                msg("system", "You are a Ryter builder."),
                msg("user", "hi"),
                msg("assistant", ""),
            ],
            tools: vec![],
            max_tokens: None,
        };
        let body = messages_body(&req);
        let ms = body["messages"].as_array().unwrap();
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0]["role"], "user");
        // The specialist's role prompt must survive, as the system field.
        assert_eq!(body["system"][0]["text"], "You are a Ryter builder.");
    }

    /// Unparseable arguments must not abort the request.
    #[test]
    fn messages_tolerates_truncated_tool_arguments() {
        let req = CompletionRequest {
            model: "m".into(),
            system: None,
            messages: vec![Message {
                tool_calls: Some(vec![call("c1", "grep", "{\"pattern\": ")]),
                ..msg("assistant", "")
            }],
            tools: vec![],
            max_tokens: None,
        };
        let body = messages_body(&req);
        assert_eq!(body["messages"][0]["content"][0]["input"], json!({}));
    }

    /// A rate limit or an overloaded provider is the most common failure in a
    /// BYOK harness; a client error is not worth a second attempt.
    #[test]
    fn retryable_statuses_are_the_transient_ones() {
        for code in [408, 425, 429, 500, 502, 503, 504, 529] {
            assert!(is_retryable_status(code), "{code} should retry");
        }
        for code in [200, 400, 401, 403, 404, 413, 422] {
            assert!(!is_retryable_status(code), "{code} should not retry");
        }
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        let waits: Vec<Duration> = (0..8).map(backoff).collect();
        assert!(waits[0] >= BACKOFF_BASE);
        assert!(waits[3] > waits[0], "should grow: {waits:?}");
        for w in &waits {
            assert!(
                *w <= BACKOFF_CAP + Duration::from_millis(250),
                "capped, got {w:?}"
            );
        }
    }

    #[test]
    fn retry_after_header_is_honored_and_clamped() {
        let mut h = HeaderMap::new();
        h.insert("retry-after", HeaderValue::from_static("3"));
        assert_eq!(retry_after(&h), Some(Duration::from_secs(3)));
        // A hostile or absurd value must not park the turn for an hour.
        let mut h = HeaderMap::new();
        h.insert("retry-after", HeaderValue::from_static("99999"));
        assert_eq!(retry_after(&h), Some(BACKOFF_CAP));
        // A date form is not parsed; fall back to our own backoff.
        let mut h = HeaderMap::new();
        h.insert(
            "retry-after",
            HeaderValue::from_static("Wed, 21 Oct 2026 07:28:00 GMT"),
        );
        assert_eq!(retry_after(&h), None);
        assert_eq!(retry_after(&HeaderMap::new()), None);
    }

    #[test]
    fn parse_openrouter_models_list() {
        let json = include_str!("../../fixtures/openrouter_models.json");
        let models = parse_models_json(json).unwrap();
        assert_eq!(models[0].id, "anthropic/claude-sonnet-4.6");
        assert_eq!(models[0].context_length, Some(200000));
    }

    #[tokio::test]
    #[ignore = "set RYTER_LIVE=1 and XAI_API_KEY or OPENROUTER_API_KEY"]
    async fn live_list_models() {
        if std::env::var("RYTER_LIVE").as_deref() != Ok("1") {
            return;
        }
        let cfg = crate::config::Config::default();
        let (name, env) = if std::env::var("XAI_API_KEY").is_ok() {
            ("spacexai", "XAI_API_KEY")
        } else {
            ("openrouter", "OPENROUTER_API_KEY")
        };
        let key = std::env::var(env).expect("live key");
        let conn = &cfg.connections[name];
        let p = HttpProvider::new(conn, key);
        let models = p.list_models().await.expect("list_models");
        assert!(!models.is_empty(), "provider returned no models");
    }
}
