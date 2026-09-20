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

/// Whether auto-compact should run.
pub fn should_compact(rep: &ContextReport) -> bool {
    rep.pct >= AUTO_COMPACT_PCT && rep.messages > KEEP_USER_TURNS * 2
}

/// Replace the prefix with a deterministic extract. No-op when already short.
pub fn compact(messages: &[Message], pass_note: &str, keep_user_turns: usize) -> Vec<Message> {
    let user_idx: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == "user")
        .map(|(i, _)| i)
        .collect();
    if user_idx.len() <= keep_user_turns {
        return messages.to_vec();
    }
    let cut = user_idx[user_idx.len() - keep_user_turns];
    if cut == 0 {
        return messages.to_vec();
    }
    let extract = extract_prefix(&messages[..cut], pass_note);
    let mut out = vec![Message {
        role: "user".into(),
        content: extract,
        tool_call_id: None,
        tool_calls: None,
    }];
    out.extend(messages[cut..].iter().cloned());
    out
}

fn extract_prefix(old: &[Message], pass_note: &str) -> String {
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
    let note = pass_note.trim();
    if !note.is_empty() {
        s.push_str("Latest pass note:\n");
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
