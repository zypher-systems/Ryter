//! Bounded, plain-text snapshots of the active conversation for MCP readers.

use crate::llm::Message;

const LIMIT: usize = 64 * 1024;
const MESSAGE_LIMIT: usize = 4096;

/// Render recent messages without reading other sessions or copying huge bodies.
/// Call when refreshing the host snapshot, outside the inbound request path.
pub fn transcript_snapshot(messages: &[Message]) -> String {
    let mut entries = Vec::new();
    let mut remaining = LIMIT - 128; // Room for the omission notice.
    for message in messages.iter().rev() {
        let mut entry = Entry::default();
        entry.push("[");
        entry.push(&message.role);
        entry.push("]\n");
        entry.push(&message.content);
        if let Some(id) = &message.tool_call_id {
            entry.push("\nTool result id: ");
            entry.push(id);
        }
        if let Some(calls) = &message.tool_calls {
            for call in calls {
                entry.push("\nTool call ");
                entry.push(&call.id);
                entry.push(" ");
                entry.push(&call.name);
                entry.push(": ");
                entry.push(&call.arguments);
                if entry.truncated {
                    break;
                }
            }
        }
        if entry.truncated {
            entry.text.push_str("\n[message truncated]");
        }
        entry.text.push_str("\n\n");
        if entry.text.len() > remaining {
            break;
        }
        remaining -= entry.text.len();
        entries.push(entry.text);
    }
    let mut text = String::new();
    if entries.len() < messages.len() {
        text.push_str("[Earlier messages omitted; showing recent conversation.]\n\n");
    }
    for entry in entries.into_iter().rev() {
        text.push_str(&entry);
    }
    text
}

#[derive(Default)]
struct Entry {
    text: String,
    truncated: bool,
}

impl Entry {
    fn push(&mut self, value: &str) {
        let mut end = value.len().min(MESSAGE_LIMIT - self.text.len());
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        self.text.push_str(&value[..end]);
        self.truncated |= end < value.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::AssistantToolCall;

    fn message(role: &str, content: impl Into<String>) -> Message {
        Message {
            role: role.into(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    #[test]
    fn renders_recent_roles_tools_and_unicode_with_bounded_memory() {
        let mut messages = vec![message("user", "oldest")];
        for _ in 0..100 {
            messages.push(message("user", "界".repeat(100_000)));
        }
        let mut last = message("assistant", "latest answer");
        last.tool_calls = Some(vec![AssistantToolCall {
            id: "call-1".into(),
            name: "read_file".into(),
            arguments: "{\"path\":\"main.rs\"}".into(),
        }]);
        messages.push(last);
        let text = transcript_snapshot(&messages);
        assert!(text.len() <= LIMIT);
        assert!(text.starts_with("[Earlier messages omitted"));
        assert!(!text.contains("oldest"));
        assert!(text.contains("[message truncated]"));
        assert!(text.contains("[assistant]\nlatest answer\nTool call call-1 read_file:"));
        assert_eq!(transcript_snapshot(&[]), "");
    }
}
