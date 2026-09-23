//! Events the session emits to every subscriber (TUI, headless, MCP).

use serde::{Deserialize, Serialize};

use crate::ids::SubagentId;
use crate::phase::Phase;
use crate::role::Role;

/// A unit of progress from the session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    /// Streamed assistant text.
    Token {
        /// UTF-8 delta.
        text: String,
    },
    /// Streamed reasoning (if the model sends it).
    Reasoning {
        /// UTF-8 delta.
        text: String,
    },
    /// A tool call started.
    ToolCall {
        /// Provider tool-call id.
        id: String,
        /// Tool name.
        name: String,
        /// JSON arguments.
        args: serde_json::Value,
        /// Who issued the call.
        role: Role,
        /// Short human label (`read Cargo.toml`), produced by the core.
        #[serde(default)]
        summary: Option<String>,
    },
    /// A tool call finished.
    ToolResult {
        /// Provider tool-call id.
        id: String,
        /// Result body (may be truncated by later layers).
        output: String,
        /// True when the tool returned an error payload.
        is_error: bool,
        /// Wall-clock duration of the call.
        #[serde(default)]
        duration_ms: Option<u64>,
    },
    /// A user turn began (orchestrator only).
    TurnStarted {
        /// Monotonic turn number within the process.
        turn: u64,
    },
    /// A user turn ended, however it ended.
    TurnFinished {
        /// Turn number from [`AgentEvent::TurnStarted`].
        turn: u64,
        /// Tool calls made during the turn.
        tools: u32,
        /// Wall-clock duration.
        duration_ms: u64,
    },
    /// Token and USD accounting for a completed model call.
    Spend {
        /// Connection name.
        connection: String,
        /// Model id.
        model: String,
        /// Who spent it.
        role: Role,
        /// Child id when a specialist spent.
        subagent_id: Option<SubagentId>,
        /// Prompt tokens.
        input_tokens: u64,
        /// Completion tokens.
        output_tokens: u64,
        /// Cached / discounted input tokens if reported.
        cached_tokens: u64,
        /// USD total for this call. `None` means unknown price (`$?.??`).
        total_usd: Option<f64>,
    },
    /// Orchestrator phase changed.
    PhaseChanged {
        /// New phase.
        phase: Phase,
    },
    /// A specialist started.
    SubagentStarted {
        /// Child id.
        id: SubagentId,
        /// Role.
        role: Role,
        /// Short task label.
        description: String,
    },
    /// What a running specialist is doing now (`edit src/store.py`,
    /// `bash python3 -m unittest`). Progress for the crew card, not chat.
    SubagentActivity {
        /// Child id.
        id: SubagentId,
        /// Role.
        role: Role,
        /// Short description.
        text: String,
    },
    /// A specialist finished.
    SubagentFinished {
        /// Child id.
        id: SubagentId,
        /// Role (older logs default to builder).
        #[serde(default = "default_finished_role")]
        role: Role,
        /// One-line summary.
        summary: String,
        /// Specialist last message for the chat pane (not the orchestrator transcript).
        #[serde(default)]
        body: String,
    },
    /// The model switched hats with the user's yes (`request_hat`).
    ModeChanged {
        /// The new hat.
        role: Role,
    },
    /// Something Ryter did on the user's behalf that they should know about
    /// (it set up git in the folder). Not an error; the turn goes on.
    Notice {
        /// Human-readable message.
        message: String,
    },
    /// The newest undo checkpoint changed (a build turn took one, `/undo`
    /// or a file revert used one, a session loaded). `/changes` compares the
    /// last turn against it.
    Checkpoint {
        /// Commit id, or `None` when the session has none.
        sha: Option<String>,
    },
    /// A drafted commit message for `/commit`.
    CommitDraft {
        /// The message, when drafting worked.
        message: Option<String>,
        /// Why it didn't.
        error: Option<String>,
    },
    /// `/commit` finished.
    Committed {
        /// `<short sha> <subject>` on success.
        summary: Option<String>,
        /// What went wrong (a hook refused, no identity, …).
        error: Option<String>,
    },
    /// One file was put back from `/changes`.
    Reverted {
        /// The file.
        path: String,
        /// What went wrong, if it did.
        error: Option<String>,
    },
    /// Unrecoverable session error.
    Error {
        /// Human-readable message.
        message: String,
    },
    /// `/context` snapshot.
    Context {
        /// Heuristic tokens.
        tokens: u64,
        /// Window used for the percentage.
        window: u64,
        /// 0–100.
        pct: u8,
        /// Transcript length.
        messages: usize,
        /// Per-contributor token estimates (`system prompt`, `tool output`, …).
        #[serde(default)]
        breakdown: Vec<(String, u64)>,
    },
    /// Transcript was compacted.
    Compacted {
        /// Tokens before.
        before: u64,
        /// Tokens after.
        after: u64,
        /// Window.
        window: u64,
    },
    /// Result of `GET /models` for the model picker.
    ModelsListed {
        /// Catalog rows.
        models: Vec<crate::llm::ModelInfo>,
    },
    /// In-flight turn stopped because the user (or MCP) cancelled.
    Cancelled,
    /// Outbound MCP servers (re)connected; per-server status text.
    McpStatus {
        /// `name → connected · N tools` or `error: …`.
        #[serde(default)]
        servers: Vec<(String, String)>,
    },
    /// Active session changed (`/new`, `/resume`).
    Session {
        /// Session id.
        id: String,
        /// Phase on disk.
        phase: Phase,
        /// Display title.
        title: String,
    },
}

fn default_finished_role() -> Role {
    Role::Builder
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_tag_is_snake_case() {
        let ev = AgentEvent::PhaseChanged {
            phase: Phase::Build,
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["kind"], "phase_changed");
        assert_eq!(v["phase"], "build");
    }

    #[test]
    fn old_logs_without_new_fields_still_deserialize() {
        let ev: AgentEvent = serde_json::from_str(
            r#"{"kind":"tool_call","id":"c1","name":"bash","args":{},"role":"orchestrator"}"#,
        )
        .unwrap();
        assert!(matches!(ev, AgentEvent::ToolCall { summary: None, .. }));
        let ev: AgentEvent = serde_json::from_str(
            r#"{"kind":"tool_result","id":"c1","output":"ok","is_error":false}"#,
        )
        .unwrap();
        assert!(matches!(
            ev,
            AgentEvent::ToolResult {
                duration_ms: None,
                ..
            }
        ));
        let ev: AgentEvent = serde_json::from_str(
            r#"{"kind":"context","tokens":1,"window":2,"pct":50,"messages":3}"#,
        )
        .unwrap();
        assert!(matches!(ev, AgentEvent::Context { breakdown, .. } if breakdown.is_empty()));
        let ev: AgentEvent =
            serde_json::from_str(r#"{"kind":"turn_finished","turn":1,"tools":2,"duration_ms":3}"#)
                .unwrap();
        assert!(matches!(ev, AgentEvent::TurnFinished { tools: 2, .. }));
    }
}
