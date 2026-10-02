//! Deterministic transcript compaction and `/context` accounting.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::llm::Message;

/// How full the model window is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextReport {
    /// Heuristic token count (bytes / 4).
    pub tokens: u64,
    /// Model window used for the percentage.
    pub window: u64,
    /// `tokens / window` as 0–100 (capped).
    pub pct: u8,
    /// Transcript messages.
    pub messages: usize,
    /// True when this report follows a compact pass.
    pub compacted: bool,
}

impl ContextReport {
    /// One-line `/context` listing.
    pub fn render(&self) -> String {
        let flag = if self.compacted { "  compacted" } else { "" };
        format!(
            "context  {}%  ~{}/{} tok  {} messages{flag}",
            self.pct, self.tokens, self.window, self.messages
        )
    }
}

/// Context window: grok-4.6 is 500k; unknown is 200k.
pub fn window_for(model: &str) -> u64 {
    match model {
        "grok-4.6" | "grok-4.6-latest" => 500_000,
        _ => 200_000,
    }
}

/// Compact token count (`0`, `12k`, `500k`, `1.2M`).
pub fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        if n % 1_000_000 == 0 {
            format!("{}M", n / 1_000_000)
        } else {
            format!("{:.1}M", n as f64 / 1_000_000.0)
        }
    } else if n >= 1000 {
        if n % 1000 == 0 {
            format!("{}k", n / 1000)
        } else {
            format!("{:.1}k", n as f64 / 1000.0)
        }
    } else {
        n.to_string()
    }
}

/// Sidebar context: `0/500k`.
pub fn format_context_used(used: u64, window: u64) -> String {
    format!("{}/{}", format_tokens(used), format_tokens(window))
}

/// Auto-compact when estimated tokens reach this fraction of the window.
pub const AUTO_COMPACT_PCT: u8 = 85;

/// Keep this many trailing user turns (and everything after the cut).
pub const KEEP_USER_TURNS: usize = 4;

/// Cheap token estimate: UTF-8 bytes / 4, matching common harness heuristics.
pub fn estimate_tokens(system: &str, messages: &[Message]) -> u64 {
    let mut n = system.len();
    for m in messages {
        n += m.role.len();
        n += m.content.len();
        if let Some(id) = &m.tool_call_id {
            n += id.len();
        }
        if let Some(calls) = &m.tool_calls {
            for c in calls {
                n += c.name.len() + c.arguments.len() + c.id.len();
            }
        }
    }
    u64::try_from(n).unwrap_or(u64::MAX).div_ceil(4)
}

/// Per-contributor token estimates for the `/context` inspector.
///
/// Rows: `system prompt`, `user messages`, `assistant text`, `tool calls`,
/// `tool output`. Zero rows are omitted. Same bytes/4 heuristic as
/// [`estimate_tokens`].
pub fn breakdown(system: &str, messages: &[Message]) -> Vec<(String, u64)> {
    let mut sys = system.len();
    let mut user = 0usize;
    let mut assistant = 0usize;
    let mut calls = 0usize;
    let mut tool_out = 0usize;
    for m in messages {
        match m.role.as_str() {
            "user" => user += m.role.len() + m.content.len(),
            "assistant" => assistant += m.role.len() + m.content.len(),
            "tool" => {
                tool_out += m.role.len() + m.content.len();
                if let Some(id) = &m.tool_call_id {
                    tool_out += id.len();
                }
            }
            _ => sys += m.role.len() + m.content.len(),
        }
        if let Some(cs) = &m.tool_calls {
            for c in cs {
                calls += c.name.len() + c.arguments.len() + c.id.len();
            }
        }
    }
    let tok = |n: usize| u64::try_from(n).unwrap_or(u64::MAX).div_ceil(4);
    [
        ("system prompt", sys),
        ("user messages", user),
        ("assistant text", assistant),
        ("tool calls", calls),
        ("tool output", tool_out),
    ]
    .into_iter()
    .filter(|(_, n)| *n > 0)
    .map(|(k, n)| (k.to_string(), tok(n)))
    .collect()
}

/// Build a report for `/context`.
pub fn report(
    system: &str,
    messages: &[Message],
    model: &str,
    window_override: u64,
) -> ContextReport {
    let window = if window_override > 0 {
        window_override
    } else {
        window_for(model)
    };
    let tokens = estimate_tokens(system, messages);
    let pct = if window == 0 {
        0
    } else {
        ((tokens.saturating_mul(100)) / window).min(100) as u8
    };
    ContextReport {
        tokens,
        window,
        pct,
        messages: messages.len(),
        compacted: false,
    }
}

/// Input, tool schemas and requested output allowance, using one estimate for
/// admission and the context inspector. The estimate is not a model tokenizer.
pub fn request_tokens(
    system: &str,
    messages: &[Message],
    tools: &[crate::llm::ToolSpec],
    output: u32,
) -> u64 {
    let schemas = serde_json::to_vec(tools).map_or(0, |v| v.len() as u64);
    estimate_tokens(system, messages)
        .saturating_add(schemas.div_ceil(4))
        .saturating_add(output.into())
}

/// Context report that includes schemas and space reserved for the answer.
pub fn request_report(
    system: &str,
    messages: &[Message],
    tools: &[crate::llm::ToolSpec],
    window: u64,
    output: u32,
) -> ContextReport {
    let mut rep = report(system, messages, "", window);
    rep.tokens = request_tokens(system, messages, tools, output);
    rep.pct = (rep.tokens.saturating_mul(100) / rep.window.max(1)).min(100) as u8;
    rep
}

/// Reject an estimated oversized request before it can incur provider cost.
pub fn ensure_fits(request: &crate::llm::CompletionRequest, window: u64) -> crate::Result<()> {
    let tokens = request_tokens(
        request.system.as_deref().unwrap_or(""),
        &request.messages,
        &request.tools,
        request.max_tokens.unwrap_or(0),
    );
    if tokens > window {
        return Err(crate::Error::Config(format!(
            "context for {} needs about {tokens} tokens including tools and output allowance, above its {window}-token window after compaction; use a larger-window model, start a new session with the remaining task, or correct [context_windows] if this limit is wrong",
            request.model
        )));
    }
    Ok(())
}

/// Whether auto-compact should run.
pub fn should_compact(rep: &ContextReport) -> bool {
    rep.pct >= AUTO_COMPACT_PCT
}

/// Replace the prefix with a deterministic extract. No-op when already short.
/// `note` is kept in the extract: what must outlive the messages dropped.
pub fn compact(messages: &[Message], note: &str, keep_user_turns: usize) -> Vec<Message> {
    let user_idx: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == "user")
        .map(|(i, _)| i)
        .collect();
    let cut = if user_idx.len() > keep_user_turns {
        user_idx[user_idx.len() - keep_user_turns.max(1)]
    } else {
        0
    };
    let mut out = if cut > 0 {
        let mut next = vec![Message {
            role: "user".into(),
            content: extract_prefix(&messages[..cut], note),
            tool_call_id: None,
            tool_calls: None,
        }];
        next.extend(messages[cut..].iter().cloned());
        next
    } else {
        messages.to_vec()
    };
    // A single user turn can contain many tool rounds. Keep call/result
    // identities and all narrative, but shorten older bulky tool results.
    // The newest result batch remains intact for the next model response.
    let latest_call = out
        .iter()
        .rposition(|m| m.tool_calls.as_ref().is_some_and(|c| !c.is_empty()))
        .unwrap_or(0);
    for m in &mut out[..latest_call] {
        if m.role == "tool" && m.content.len() > 2048 {
            let mut head = 768;
            while !m.content.is_char_boundary(head) {
                head -= 1;
            }
            let mut tail = m.content.len() - 768;
            while !m.content.is_char_boundary(tail) {
                tail += 1;
            }
            m.content = format!(
                "{}\n[earlier tool output shortened; re-read the source if needed]\n{}",
                &m.content[..head],
                &m.content[tail..]
            );
        }
    }
    if estimate_tokens("", &out) < estimate_tokens("", messages) {
        out
    } else {
        messages.to_vec()
    }
}

fn extract_prefix(old: &[Message], note: &str) -> String {
    let mut tools = BTreeSet::new();
    let mut files = BTreeSet::new();
    for m in old {
        if let Some(calls) = &m.tool_calls {
            for c in calls {
                if !c.name.is_empty() {
                    tools.insert(c.name.clone());
                }
                if let Ok(v) = serde_json::from_str::<Value>(&c.arguments) {
                    if let Some(p) = v.get("path").and_then(Value::as_str) {
                        if !p.is_empty() {
                            files.insert(p.to_string());
                        }
                    }
                }
            }
        }
    }
    let mut s = String::from("[compacted earlier context]\n");
    if !tools.is_empty() {
        s.push_str("Tools used: ");
        s.push_str(&tools.into_iter().collect::<Vec<_>>().join(", "));
        s.push('\n');
    }
    if !files.is_empty() {
        s.push_str("Files touched: ");
        s.push_str(&files.into_iter().collect::<Vec<_>>().join(", "));
        s.push('\n');
    }
    s.push_str("Earlier user messages and assistant notes (verbatim):\n");
    for m in old
        .iter()
        .filter(|m| matches!(m.role.as_str(), "user" | "assistant") && !m.content.is_empty())
    {
        s.push_str(&format!("\n{}:\n{}\n", m.role, m.content));
    }
    let note = note.trim();
    if !note.is_empty() {
        s.push_str(note);
        if !note.ends_with('\n') {
            s.push('\n');
        }
    }
    s.push_str(&format!("Dropped {} older messages.\n", old.len()));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: &str) -> Message {
        Message {
            role: role.into(),
            content: content.into(),
            tool_call_id: None,
            tool_calls: None,
        }
    }

    #[test]
    fn grok_window_is_500k() {
        assert_eq!(window_for("grok-4.6"), 500_000);
        assert_eq!(window_for("mystery"), 200_000);
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(500_000), "500k");
        assert_eq!(format_context_used(0, 500_000), "0/500k");
        assert_eq!(format_context_used(12_000, 500_000), "12k/500k");
    }

    #[test]
    fn compact_keeps_tail_and_extracts_files() {
        let mut messages = Vec::new();
        for i in 0..6 {
            messages.push(msg("user", &format!("turn {i}")));
            messages.push(Message {
                role: "assistant".into(),
                content: String::new(),
                tool_call_id: None,
                tool_calls: Some(vec![crate::llm::AssistantToolCall {
                    id: format!("c{i}"),
                    name: "read_file".into(),
                    arguments: format!(r#"{{"path":"src/f{i}.rs"}}"#),
                }]),
            });
            messages.push(Message {
                role: "tool".into(),
                content: "x".repeat(8_000),
                tool_call_id: Some(format!("c{i}")),
                tool_calls: None,
            });
        }
        let out = compact(&messages, "ship the flag", 2);
        assert!(
            out.len() < messages.len(),
            "{} vs {}",
            out.len(),
            messages.len()
        );
        assert!(out[0].content.contains("[compacted"));
        assert!(out[0].content.contains("src/f0.rs"));
        assert!(out[0].content.contains("read_file"));
        assert!(out[0].content.contains("ship the flag"));
        assert!(out.iter().any(|m| m.content == "turn 5"));
        assert!(!out.iter().any(|m| m.content == "turn 0"));
        let before = estimate_tokens("", &messages);
        let after = estimate_tokens("", &out);
        assert!(after < before, "{after} vs {before}");
    }

    #[test]
    fn breakdown_sums_to_estimate() {
        let messages = vec![
            msg("user", "hello there"),
            msg("assistant", "hi"),
            Message {
                role: "tool".into(),
                content: "x".repeat(400),
                tool_call_id: Some("c1".into()),
                tool_calls: None,
            },
        ];
        let rows = breakdown("sys", &messages);
        let names: Vec<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            names,
            [
                "system prompt",
                "user messages",
                "assistant text",
                "tool output"
            ]
        );
        let total: u64 = rows.iter().map(|(_, n)| n).sum();
        let est = estimate_tokens("sys", &messages);
        assert!(
            total >= est && total <= est + rows.len() as u64,
            "{total} vs {est}"
        );
    }

    #[test]
    fn a_long_single_turn_shrinks_without_losing_constraints_or_tool_identity() {
        let mut messages = vec![msg(
            "user",
            "Never change the public API; keep compatibility.",
        )];
        for i in 0..3 {
            messages.push(Message {
                role: "assistant".into(),
                content: "Still need to validate compatibility".into(),
                tool_call_id: None,
                tool_calls: Some(vec![crate::llm::AssistantToolCall {
                    id: format!("c{i}"),
                    name: "read_file".into(),
                    arguments: "{}".into(),
                }]),
            });
            messages.push(Message {
                role: "tool".into(),
                content: "界".repeat(6000),
                tool_call_id: Some(format!("c{i}")),
                tool_calls: None,
            });
        }
        let next = compact(&messages, "", 4);
        assert_eq!(next.len(), messages.len());
        assert!(estimate_tokens("", &next) < estimate_tokens("", &messages) / 2);
        assert_eq!(next[0], messages[0]);
        assert_eq!(next.last(), messages.last());
        for (before, after) in messages.iter().zip(&next) {
            assert_eq!(before.tool_calls, after.tool_calls);
            assert_eq!(before.tool_call_id, after.tool_call_id);
            if before.role != "tool" {
                assert_eq!(before.content, after.content);
            }
        }
        assert!(next[2].content.contains("re-read the source"));
    }

    #[test]
    fn extracted_history_preserves_user_constraints_and_unfinished_work() {
        let mut history = vec![
            msg("user", "Keep the CSV format compatible."),
            msg("assistant", "Still need the empty-file regression."),
        ];
        history.push(msg("tool", &"bulky output".repeat(2000)));
        history.push(msg("user", "Continue"));
        let next = compact(&history, "approved plan path", 1);
        assert!(next[0].content.contains("Keep the CSV format compatible."));
        assert!(
            next[0]
                .content
                .contains("Still need the empty-file regression.")
        );
        assert!(next[0].content.contains("approved plan path"));
        assert_eq!(next.last(), history.last());
    }

    #[test]
    fn short_transcript_is_unchanged() {
        let messages = vec![msg("user", "hi"), msg("assistant", "hello")];
        assert_eq!(compact(&messages, "", 4), messages);
    }

    #[test]
    fn report_pct_uses_window() {
        let messages = vec![msg("user", &"a".repeat(400))];
        let r = report("", &messages, "mystery", 0);
        assert_eq!(r.window, 200_000);
        assert!(r.pct < 5);
        let r2 = report("", &messages, "mystery", 200);
        assert!(r2.pct >= 50, "{}", r2.pct);
        assert!(r2.render().contains("context"));
    }
}
