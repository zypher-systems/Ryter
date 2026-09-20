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
                .timeout(Duration::from_secs(600))
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
        let resp = self
            .client
            .post(self.endpoint())
            .headers(self.headers()?)
            .json(&self.body(&req))
            .send()
            .await
            .map_err(|e| Error::Provider(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Provider(format!("http {status}: {text}")));
        }
        let backend = self.backend;
        let byte_stream = resp.bytes_stream();
        Ok(Box::pin(sse_delta_stream(backend, byte_stream)))
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

fn responses_body(req: &CompletionRequest) -> Value {
    let mut input = Vec::new();
    for m in &req.messages {
        input.push(json!({"role": m.role, "content": m.content}));
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
        body["tools"] = json!(tools_openai(&req.tools));
    }
    body
}

fn messages_body(req: &CompletionRequest) -> Value {
    let mut messages = Vec::new();
    for m in &req.messages {
        if m.role == "system" {
            continue;
        }
        messages.push(json!({"role": m.role, "content": m.content}));
    }
    let mut body = json!({
        "model": req.model,
        "messages": messages,
        "stream": true,
        "max_tokens": req.max_tokens.unwrap_or(8192),
    });
    if let Some(sys) = &req.system {
        body["system"] = json!(sys);
    }
    if !req.tools.is_empty() {
        body["tools"] = json!(tools_anthropic(&req.tools));
    }
    body
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
