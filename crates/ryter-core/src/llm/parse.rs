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
    /// OpenAI `/v1/responses` (SpaceXSI default).
    Responses,
    /// Anthropic `/v1/messages`.
    Messages,
}

/// Parse a full SSE document into deltas.
pub fn parse_sse(backend: Backend, body: &str) -> Result<Vec<StreamDelta>> {
    let (mut out, _) = parse_blocks(backend, body)?;
    out.push(StreamDelta::Done);
    Ok(out)
}

/// Parse SSE blocks into deltas, and say whether the provider said the
/// answer is over (`[DONE]`, `response.completed`, `message_stop`). Nothing
/// after that point is read. An error the provider sends inside the stream
/// is an `Err`: dropped, it ended the turn as a normal, empty reply.
pub fn parse_blocks(backend: Backend, body: &str) -> Result<(Vec<StreamDelta>, bool)> {
    let mut out = Vec::new();
    for (event, data) in sse_blocks(body) {
        if data == "[DONE]" {
            return Ok((out, true));
        }
        if data.is_empty() {
            continue;
        }
        let terminal = match backend {
            Backend::ChatCompletions => push_chat(&mut out, &data)?,
            Backend::Responses => push_responses(&mut out, event.as_deref(), &data)?,
            Backend::Messages => push_messages(&mut out, event.as_deref(), &data)?,
        };
        if terminal {
            return Ok((out, true));
        }
    }
    Ok((out, false))
}

/// The provider's own words for an error object: `type: message`.
fn error_text(e: &Value) -> String {
    let msg = e
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| e.as_str())
        .unwrap_or("");
    let kind = e
        .get("type")
        .or_else(|| e.get("code"))
        .map(|k| match k {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default();
    match (kind.is_empty(), msg.is_empty()) {
        (false, false) => format!("{kind}: {msg}"),
        (true, false) => msg.to_string(),
        (false, true) => kind,
        (true, true) => e.to_string(),
    }
}

fn mid_stream(e: &Value) -> Error {
    Error::Provider(format!("the provider failed mid-reply: {}", error_text(e)))
}

/// A finish that is neither done nor cut off at the output limit: the
/// provider stopped the reply itself. Carrying on as if it had finished left
/// the user a partial or empty answer with no reason given.
fn refused(reason: &str) -> Error {
    Error::Provider(match reason {
        "model_context_window_exceeded" | "context_length_exceeded" => {
            "the conversation is longer than the model's context window; /compact, then try again"
                .into()
        }
        other => format!("the provider stopped the reply: {other}"),
    })
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

fn push_chat(out: &mut Vec<StreamDelta>, data: &str) -> Result<bool> {
    let v: Value = serde_json::from_str(data)
        .map_err(|e| Error::Provider(format!("chat sse: {e}: {data}")))?;
    // OpenRouter reports an upstream failure as a chunk with `error`.
    if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
        return Err(mid_stream(e));
    }
    if let Some(u) = usage_from(&v["usage"]) {
        out.push(StreamDelta::Usage(u));
    }
    if let Some(c) = reported_cost(&v) {
        out.push(StreamDelta::ReportedCost(c));
    }
    // OpenRouter names the host on every chunk; the last one read stands.
    if let Some(h) = v.get("provider").and_then(Value::as_str) {
        if !h.is_empty() {
            out.push(StreamDelta::Host(h.to_string()));
        }
    }
    let Some(choice) = v
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|c| c.first())
    else {
        return Ok(false);
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
    match choice.get("finish_reason").and_then(Value::as_str) {
        Some("length") => out.push(StreamDelta::Truncated),
        Some("error") => {
            return Err(mid_stream(choice.get("error").unwrap_or(&Value::Null)));
        }
        Some(r @ ("content_filter" | "refusal" | "context_length_exceeded")) => {
            return Err(refused(r));
        }
        _ => {}
    }
    if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let id = call.get("id").and_then(Value::as_str).unwrap_or("");
            let func = &call["function"];
            let name = func.get("name").and_then(Value::as_str).unwrap_or("");
            // Some OpenAI-compatible servers send the arguments as an
            // object, not a JSON string.
            let arguments = match func.get("arguments") {
                Some(Value::String(s)) => s.clone(),
                Some(v @ Value::Object(_)) => v.to_string(),
                _ => String::new(),
            };
            out.push(StreamDelta::ToolCall {
                stream_key: call
                    .get("index")
                    .and_then(Value::as_u64)
                    .map(|i| i.to_string()),
                id: id.to_string(),
                name: name.to_string(),
                arguments,
            });
        }
    }
    Ok(false)
}

fn push_responses(out: &mut Vec<StreamDelta>, event: Option<&str>, data: &str) -> Result<bool> {
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
                stream_key: (!id.is_empty()).then(|| id.to_string()).or_else(|| {
                    v.get("output_index")
                        .and_then(Value::as_u64)
                        .map(|i| format!("index:{i}"))
                }),
                id: String::new(),
                name: String::new(),
                arguments: arguments.to_string(),
            });
        }
        "response.output_item.added" => {
            let item = &v["item"];
            if item.get("type").and_then(Value::as_str) == Some("function_call") {
                out.push(StreamDelta::ToolCall {
                    stream_key: item
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .or_else(|| {
                            v.get("output_index")
                                .and_then(Value::as_u64)
                                .map(|i| format!("index:{i}"))
                        }),
                    id: item
                        .get("call_id")
                        .or_else(|| item.get("id"))
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
            if let Some(c) = reported_cost(&v["response"]) {
                out.push(StreamDelta::ReportedCost(c));
            }
            match v["response"]["incomplete_details"]["reason"].as_str() {
                Some("max_output_tokens") => out.push(StreamDelta::Truncated),
                Some(r) if ty == "response.incomplete" => return Err(refused(r)),
                _ => {}
            }
            return Ok(true);
        }
        "response.failed" => {
            return Err(mid_stream(&v["response"]["error"]));
        }
        "error" => {
            return Err(mid_stream(v.get("error").unwrap_or(&v)));
        }
        _ => {
            if let Some(u) = usage_from(&v["response"]["usage"]).or_else(|| usage_from(&v["usage"]))
            {
                out.push(StreamDelta::Usage(u));
            }
        }
    }
    Ok(false)
}

fn push_messages(out: &mut Vec<StreamDelta>, event: Option<&str>, data: &str) -> Result<bool> {
    let v: Value = serde_json::from_str(data)
        .map_err(|e| Error::Provider(format!("messages sse: {e}: {data}")))?;
    let ty = event
        .or_else(|| v.get("type").and_then(Value::as_str))
        .unwrap_or("");
    match ty {
        // A tool call's id and name arrive here, once; the arguments follow as
        // id-less `input_json_delta` fragments. Without this arm Anthropic
        // models could never call a tool. Thinking / redacted_thinking open
        // here too so their signatures can be echoed after a tool round.
        "content_block_start" => {
            let block = &v["content_block"];
            let stream_key = v
                .get("index")
                .and_then(Value::as_u64)
                .map(|i| i.to_string());
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    out.push(StreamDelta::ToolCall {
                        stream_key,
                        id: block
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        name: block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        arguments: String::new(),
                    });
                }
                Some("thinking") => {
                    out.push(StreamDelta::ThinkingOpen {
                        stream_key,
                        redacted_data: None,
                    });
                    // A non-streamed block may already carry text / signature.
                    if let Some(s) = block.get("thinking").and_then(Value::as_str) {
                        if !s.is_empty() {
                            out.push(StreamDelta::Reasoning(s.to_string()));
                        }
                    }
                    if let Some(s) = block.get("signature").and_then(Value::as_str) {
                        if !s.is_empty() {
                            out.push(StreamDelta::ThinkingSignature {
                                stream_key: v
                                    .get("index")
                                    .and_then(Value::as_u64)
                                    .map(|i| i.to_string()),
                                signature: s.to_string(),
                            });
                        }
                    }
                }
                Some("redacted_thinking") => {
                    out.push(StreamDelta::ThinkingOpen {
                        stream_key,
                        redacted_data: Some(
                            block
                                .get("data")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                        ),
                    });
                }
                _ => {}
            }
        }
        "content_block_delta" => {
            let delta = &v["delta"];
            let stream_key = v
                .get("index")
                .and_then(Value::as_u64)
                .map(|i| i.to_string());
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
                Some("signature_delta") => {
                    if let Some(s) = delta.get("signature").and_then(Value::as_str) {
                        if !s.is_empty() {
                            out.push(StreamDelta::ThinkingSignature {
                                stream_key,
                                signature: s.to_string(),
                            });
                        }
                    }
                }
                Some("input_json_delta") => {
                    let arguments = delta
                        .get("partial_json")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    out.push(StreamDelta::ToolCall {
                        stream_key,
                        id: String::new(),
                        name: String::new(),
                        arguments: arguments.to_string(),
                    });
                }
                _ => {}
            }
        }
        "content_block_stop" => {
            // Close a thinking block. Tool and text stops need no signal:
            // tools finish via the accumulator, text is flush-on-next-block.
            // A stop with no open thinking is ignored by the echo accumulator.
            out.push(StreamDelta::ThinkingClose {
                stream_key: v
                    .get("index")
                    .and_then(Value::as_u64)
                    .map(|i| i.to_string()),
            });
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
            match v["delta"]["stop_reason"].as_str() {
                Some("max_tokens") => out.push(StreamDelta::Truncated),
                Some(r @ ("refusal" | "model_context_window_exceeded")) => {
                    return Err(refused(r));
                }
                _ => {}
            }
        }
        "message_stop" => return Ok(true),
        // `overloaded_error` and friends, sent after the answer began.
        "error" => return Err(mid_stream(v.get("error").unwrap_or(&v))),
        _ => {}
    }
    Ok(false)
}

fn usage_from(v: &Value) -> Option<Usage> {
    if v.is_null() {
        return None;
    }
    let n = |k: &str| v.get(k).and_then(Value::as_u64);
    let input = n("prompt_tokens")
        .or_else(|| n("input_tokens"))
        .unwrap_or(0);
    let output = n("completion_tokens")
        .or_else(|| n("output_tokens"))
        .unwrap_or(0);
    let detail = |outer: &str, k: &str| v.get(outer).and_then(|d| d.get(k)).and_then(Value::as_u64);
    // Anthropic's `input_tokens` leaves out what was read from or written to
    // the cache; everyone else's prompt count includes it. Add them back so
    // `input_tokens` always means the whole prompt.
    let (read, write) = (
        n("cache_read_input_tokens"),
        n("cache_creation_input_tokens"),
    );
    let anthropic = read.is_some() || write.is_some();
    let (input, cached, cache_write) = if anthropic {
        let (read, write) = (read.unwrap_or(0), write.unwrap_or(0));
        (input + read + write, read, write)
    } else {
        let cached = detail("prompt_tokens_details", "cached_tokens")
            .or_else(|| detail("input_tokens_details", "cached_tokens"))
            // DeepSeek's own API.
            .or_else(|| n("prompt_cache_hit_tokens"))
            .unwrap_or(0);
        // OpenRouter and OpenAI (Responses) name the write the same way.
        let write = detail("prompt_tokens_details", "cache_write_tokens")
            .or_else(|| detail("input_tokens_details", "cache_write_tokens"))
            .unwrap_or(0);
        (input, cached, write)
    };
    if input == 0 && output == 0 && cached == 0 {
        return None;
    }
    Some(Usage {
        input_tokens: input,
        output_tokens: output,
        cached_tokens: cached,
        cache_write_tokens: cache_write,
    })
}

/// What the provider itself says the request cost, in USD: OpenRouter's
/// `usage.cost`; SpaceXSI's `usage.cost_in_usd_ticks` (ten billion to the
/// dollar) or `cost_in_nano_usd`.
fn reported_cost(v: &Value) -> Option<f64> {
    let usage = v.get("usage");
    usage
        .and_then(|u| u.get("cost").or_else(|| u.get("total_cost")))
        .and_then(Value::as_f64)
        .or_else(|| {
            usage
                .and_then(|u| u.get("cost_in_usd_ticks"))
                .and_then(Value::as_f64)
                .map(|t| t / 1e10)
        })
        .or_else(|| {
            usage
                .and_then(|u| u.get("cost_in_nano_usd"))
                .and_then(Value::as_f64)
                .map(|n| n / 1e9)
        })
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

    /// Anthropic counts the prompt in three fields; `input_tokens` is only the
    /// part that was neither read from nor written to the cache. The usage
    /// Ryter keeps is the whole prompt, with reads and writes marked.
    #[test]
    fn anthropic_cache_counts_are_part_of_the_prompt() {
        let sse = r#"event: message_start
data: {"message":{"usage":{"input_tokens":12,"cache_read_input_tokens":9000,"cache_creation_input_tokens":800,"output_tokens":1}}}

event: message_delta
data: {"delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":40}}

"#;
        let usage = parse_sse(Backend::Messages, sse)
            .unwrap()
            .into_iter()
            .filter_map(|d| match d {
                StreamDelta::Usage(u) => Some(u),
                _ => None,
            })
            .fold(Usage::default(), Usage::merge);
        assert_eq!(
            usage,
            Usage {
                input_tokens: 9812,
                output_tokens: 40,
                cached_tokens: 9000,
                cache_write_tokens: 800,
            }
        );
    }

    /// OpenAI's Responses usage names the write the way OpenRouter does;
    /// from GPT-5.6 on a write is billed at five quarters, so it is kept.
    #[test]
    fn openai_cache_writes_are_counted() {
        let sse = "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":1000,\"output_tokens\":5,\"input_tokens_details\":{\"cached_tokens\":600,\"cache_write_tokens\":300}}}}\n\n";
        let deltas = parse_sse(Backend::Responses, sse).unwrap();
        let u = deltas
            .into_iter()
            .find_map(|d| match d {
                StreamDelta::Usage(u) => Some(u),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            (u.input_tokens, u.cached_tokens, u.cache_write_tokens),
            (1000, 600, 300)
        );
    }

    /// SpaceXSI says what the request cost, in ticks of a ten-billionth
    /// of a dollar (or nano-dollars); that figure beats any table.
    #[test]
    fn the_providers_own_cost_is_read_in_ticks_and_nano_dollars() {
        let cost = |sse: &str| {
            parse_sse(Backend::Responses, sse)
                .unwrap()
                .into_iter()
                .find_map(|d| match d {
                    StreamDelta::ReportedCost(c) => Some(c),
                    _ => None,
                })
        };
        let ticks = "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":5,\"cost_in_usd_ticks\":1636040000}}}\n\n";
        assert!((cost(ticks).unwrap() - 0.163604).abs() < 1e-12);
        let nano = "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":5,\"cost_in_nano_usd\":163604000}}}\n\n";
        assert!((cost(nano).unwrap() - 0.163604).abs() < 1e-12);
        let none = "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":5}}}\n\n";
        assert_eq!(cost(none), None);
        // The chat wire too (OpenRouter's `cost`, or a tick count there).
        let chat = "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"cost_in_usd_ticks\":5000000000}}\n\ndata: [DONE]\n\n";
        let c = parse_sse(Backend::ChatCompletions, chat)
            .unwrap()
            .into_iter()
            .find_map(|d| match d {
                StreamDelta::ReportedCost(c) => Some(c),
                _ => None,
            })
            .unwrap();
        assert!((c - 0.5).abs() < 1e-12);
    }

    /// OpenRouter names the host that served the call on every chunk. The
    /// host decides the price, so it is read; another wire names none.
    #[test]
    fn the_host_that_served_the_call_is_read_off_the_chunks() {
        let hosts = |backend, sse: &str| -> Vec<String> {
            parse_sse(backend, sse)
                .unwrap()
                .into_iter()
                .filter_map(|d| match d {
                    StreamDelta::Host(h) => Some(h),
                    _ => None,
                })
                .collect()
        };
        let chat = "data: {\"provider\":\"Baseten\",\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n\
                    data: {\"provider\":\"Baseten\",\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"cost\":0.002}}\n\n\
                    data: [DONE]\n\n";
        assert_eq!(
            hosts(Backend::ChatCompletions, chat),
            vec!["Baseten".to_string(), "Baseten".to_string()]
        );
        let plain = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n";
        assert!(hosts(Backend::ChatCompletions, plain).is_empty());
        let empty = "data: {\"provider\":\"\",\"choices\":[]}\n\ndata: [DONE]\n\n";
        assert!(hosts(Backend::ChatCompletions, empty).is_empty());
    }

    /// OpenAI-style counts already include cached tokens; DeepSeek names its
    /// own field.
    #[test]
    fn other_cache_counts_are_read_as_given() {
        for (sse, cached) in [
            (
                r#"data: {"choices":[],"usage":{"prompt_tokens":100,"completion_tokens":5,"prompt_tokens_details":{"cached_tokens":60}}}

"#,
                60,
            ),
            (
                r#"data: {"choices":[],"usage":{"prompt_tokens":100,"completion_tokens":5,"prompt_cache_hit_tokens":70,"prompt_cache_miss_tokens":30}}

"#,
                70,
            ),
        ] {
            let u = parse_sse(Backend::ChatCompletions, sse)
                .unwrap()
                .into_iter()
                .find_map(|d| match d {
                    StreamDelta::Usage(u) => Some(u),
                    _ => None,
                })
                .unwrap();
            assert_eq!((u.input_tokens, u.cached_tokens), (100, cached));
        }
    }
}
