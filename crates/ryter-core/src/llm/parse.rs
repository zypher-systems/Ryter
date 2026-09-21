//! SSE parsers for the three wire protocols.

use serde_json::Value;

use crate::error::{Error, Result};
use crate::llm::StreamDelta;
use crate::spend::Usage;

/// Which HTTP API a connection speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// OpenAI `/v1/chat/completions` (OpenRouter, generic compat).
    ChatCompletions,
    /// OpenAI `/v1/responses` (SpaceXAI default).
    Responses,
    /// Anthropic `/v1/messages`.
    Messages,
}

/// Parse a full SSE document into deltas.
pub fn parse_sse(backend: Backend, body: &str) -> Result<Vec<StreamDelta>> {
    let mut out = Vec::new();
    for (event, data) in sse_blocks(body) {
        if data == "[DONE]" {
            out.push(StreamDelta::Done);
            continue;
        }
        if data.is_empty() {
            continue;
        }
        match backend {
            Backend::ChatCompletions => push_chat(&mut out, &data)?,
            Backend::Responses => push_responses(&mut out, event.as_deref(), &data)?,
            Backend::Messages => push_messages(&mut out, event.as_deref(), &data)?,
        }
    }
    if !out.iter().any(|d| matches!(d, StreamDelta::Done)) {
        out.push(StreamDelta::Done);
    }
    Ok(out)
}

fn sse_blocks(body: &str) -> Vec<(Option<String>, String)> {
    let mut blocks = Vec::new();
    let mut event = None;
    let mut data = String::new();
    for line in body.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            if event.is_some() || !data.is_empty() {
                blocks.push((event.take(), std::mem::take(&mut data)));
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("event:") {
            event = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.trim_start());
        }
    }
    if event.is_some() || !data.is_empty() {
        blocks.push((event, data));
    }
    blocks
}

fn push_chat(out: &mut Vec<StreamDelta>, data: &str) -> Result<()> {
    let v: Value = serde_json::from_str(data)
        .map_err(|e| Error::Provider(format!("chat sse: {e}: {data}")))?;
    if let Some(u) = usage_from(&v["usage"]) {
        out.push(StreamDelta::Usage(u));
    }
    if let Some(c) = reported_cost(&v) {
        out.push(StreamDelta::ReportedCost(c));
    }
    let Some(choice) = v
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|c| c.first())
    else {
        return Ok(());
    };
    let delta = &choice["delta"];
    if let Some(s) = delta.get("content").and_then(Value::as_str) {
        if !s.is_empty() {
            out.push(StreamDelta::Text(s.to_string()));
        }
    }
    if let Some(s) = delta
        .get("reasoning_content")
        .or_else(|| delta.get("reasoning"))
        .and_then(Value::as_str)
    {
        if !s.is_empty() {
            out.push(StreamDelta::Reasoning(s.to_string()));
        }
    }
    if choice.get("finish_reason").and_then(Value::as_str) == Some("length") {
        out.push(StreamDelta::Truncated);
    }
    if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let id = call.get("id").and_then(Value::as_str).unwrap_or("");
            let func = &call["function"];
            let name = func.get("name").and_then(Value::as_str).unwrap_or("");
            let arguments = func.get("arguments").and_then(Value::as_str).unwrap_or("");
            out.push(StreamDelta::ToolCall {
                id: id.to_string(),
                name: name.to_string(),
                arguments: arguments.to_string(),
            });
        }
    }
    Ok(())
}

fn push_responses(out: &mut Vec<StreamDelta>, event: Option<&str>, data: &str) -> Result<()> {
    let v: Value = serde_json::from_str(data)
        .map_err(|e| Error::Provider(format!("responses sse: {e}: {data}")))?;
    let ty = event
        .or_else(|| v.get("type").and_then(Value::as_str))
        .unwrap_or("");
    match ty {
        "response.output_text.delta" | "response.text.delta" => {
            if let Some(s) = v.get("delta").and_then(Value::as_str) {
                if !s.is_empty() {
                    out.push(StreamDelta::Text(s.to_string()));
                }
            }
        }
        "response.reasoning_text.delta" => {
            if let Some(s) = v.get("delta").and_then(Value::as_str) {
                if !s.is_empty() {
                    out.push(StreamDelta::Reasoning(s.to_string()));
                }
            }
        }
        "response.function_call_arguments.delta" => {
            let id = v
                .get("item_id")
                .or_else(|| v.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let arguments = v.get("delta").and_then(Value::as_str).unwrap_or("");
            out.push(StreamDelta::ToolCall {
                id: id.to_string(),
                name: String::new(),
                arguments: arguments.to_string(),
            });
        }
        "response.output_item.added" => {
            let item = &v["item"];
            if item.get("type").and_then(Value::as_str) == Some("function_call") {
                out.push(StreamDelta::ToolCall {
                    id: item
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    name: item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    arguments: String::new(),
                });
            }
        }
        "response.completed" | "response.incomplete" => {
            if let Some(u) = usage_from(&v["response"]["usage"]) {
                out.push(StreamDelta::Usage(u));
            }
            if v["response"]["incomplete_details"]["reason"].as_str() == Some("max_output_tokens") {
                out.push(StreamDelta::Truncated);
            }
        }
        _ => {
            if let Some(u) = usage_from(&v["response"]["usage"]).or_else(|| usage_from(&v["usage"]))
            {
                out.push(StreamDelta::Usage(u));
            }
        }
    }
    Ok(())
}

fn push_messages(out: &mut Vec<StreamDelta>, event: Option<&str>, data: &str) -> Result<()> {
    let v: Value = serde_json::from_str(data)
        .map_err(|e| Error::Provider(format!("messages sse: {e}: {data}")))?;
    let ty = event
        .or_else(|| v.get("type").and_then(Value::as_str))
        .unwrap_or("");
    match ty {
        "content_block_delta" => {
            let delta = &v["delta"];
            match delta.get("type").and_then(Value::as_str) {
                Some("text_delta") => {
                    if let Some(s) = delta.get("text").and_then(Value::as_str) {
                        if !s.is_empty() {
                            out.push(StreamDelta::Text(s.to_string()));
                        }
                    }
                }
                Some("thinking_delta") => {
                    if let Some(s) = delta.get("thinking").and_then(Value::as_str) {
                        if !s.is_empty() {
                            out.push(StreamDelta::Reasoning(s.to_string()));
                        }
                    }
                }
                Some("input_json_delta") => {
                    let arguments = delta
                        .get("partial_json")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    out.push(StreamDelta::ToolCall {
                        id: String::new(),
                        name: String::new(),
                        arguments: arguments.to_string(),
                    });
                }
                _ => {}
            }
        }
        "message_start" => {
            if let Some(u) = usage_from(&v["message"]["usage"]) {
                out.push(StreamDelta::Usage(u));
            }
        }
        "message_delta" => {
            if let Some(u) = usage_from(&v["usage"]) {
                out.push(StreamDelta::Usage(u));
            }
            if v["delta"]["stop_reason"].as_str() == Some("max_tokens") {
                out.push(StreamDelta::Truncated);
            }
        }
        _ => {}
    }
    Ok(())
}

fn usage_from(v: &Value) -> Option<Usage> {
    if v.is_null() {
        return None;
    }
    let input = v
        .get("prompt_tokens")
        .or_else(|| v.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = v
        .get("completion_tokens")
        .or_else(|| v.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cached = v
        .get("prompt_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .or_else(|| {
            v.get("input_tokens_details")
                .and_then(|d| d.get("cached_tokens"))
        })
        .or_else(|| v.get("cache_read_input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if input == 0 && output == 0 && cached == 0 {
        return None;
    }
    Some(Usage {
        input_tokens: input,
        output_tokens: output,
        cached_tokens: cached,
    })
}

fn reported_cost(v: &Value) -> Option<f64> {
    v.get("usage")
        .and_then(|u| u.get("cost").or_else(|| u.get("total_cost")))
        .and_then(Value::as_f64)
        .or_else(|| v.get("cost").and_then(Value::as_f64))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A turn cut off at the output ceiling must be distinguishable from a
    /// finished one; otherwise the agent loop reports a partial answer as
    /// success.
    #[test]
    fn each_backend_reports_truncation() {
        let cases: &[(Backend, &str)] = &[
            (
                Backend::ChatCompletions,
                r#"data: {"choices":[{"delta":{"content":"half"},"finish_reason":"length"}]}

"#,
            ),
            (
                Backend::Responses,
                r#"event: response.incomplete
data: {"response":{"incomplete_details":{"reason":"max_output_tokens"}}}

"#,
            ),
            (
                Backend::Messages,
                r#"event: message_delta
data: {"delta":{"stop_reason":"max_tokens"}}

"#,
            ),
        ];
        for (backend, sse) in cases {
            let deltas = parse_sse(*backend, sse).expect("parse");
            assert!(
                deltas.contains(&StreamDelta::Truncated),
                "{backend:?} missed truncation: {deltas:?}"
            );
        }
    }

    /// A normal stop must not be flagged.
    #[test]
    fn a_normal_stop_is_not_truncation() {
        let cases: &[(Backend, &str)] = &[
            (
                Backend::ChatCompletions,
                r#"data: {"choices":[{"delta":{"content":"all"},"finish_reason":"stop"}]}

"#,
            ),
            (
                Backend::Messages,
                r#"event: message_delta
data: {"delta":{"stop_reason":"end_turn"}}

"#,
            ),
        ];
        for (backend, sse) in cases {
            let deltas = parse_sse(*backend, sse).expect("parse");
            assert!(
                !deltas.contains(&StreamDelta::Truncated),
                "{backend:?} false positive: {deltas:?}"
            );
        }
    }
}
