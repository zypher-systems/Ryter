//! Events the session emits to every subscriber (TUI, headless, MCP).

use serde::{Deserialize, Serialize};

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
        /// What an edit did to the file, with line numbers.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diff: Option<Box<crate::diff::FileDiff>>,
    },
    /// A user turn began.
    TurnStarted {
        /// Monotonic turn number within the process.
        turn: u64,
        /// The hat it runs in, which says whose conversation it is part of.
        role: Role,
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
        /// The hat it was spent in.
        role: Role,
        /// Prompt tokens.
        input_tokens: u64,
        /// Completion tokens.
        output_tokens: u64,
        /// Cached / discounted input tokens if reported.
        cached_tokens: u64,
        /// USD total for this call. `None` means unknown price (`$?.??`).
        total_usd: Option<f64>,
    },
    /// The test hat filed a report. It is in the conversation the other
    /// hats share from here on, and in its file.
    Tested {
        /// Tester model id.
        model: String,
        /// `✗ 2 of 5 failed`, `✓ 5 of 5 passed`.
        headline: String,
        /// Whether every scenario passed.
        passed: bool,
        /// The report's rows: a pass one line, a failure opened out.
        rows: Vec<String>,
        /// The report's file, as a path in the project.
        file: String,
        /// The number of the first scenario that failed, for "retest 3".
        #[serde(default)]
        first_failed: Option<usize>,
        /// The files it tested, as a git tree: a commit of anything else
        /// was not tested.
        #[serde(default)]
        tree: Option<String>,
        /// What the turn that filed it cost; `None` when unpriced.
        #[serde(default)]
        total_usd: Option<f64>,
        /// How long that turn took.
        #[serde(default)]
        duration_ms: u64,
    },
    /// The product a test started is up, or was stopped. While it is up the
    /// user can stop it (`/stop`), and is asked about it on quit.
    Product {
        /// Whether it is running.
        running: bool,
        /// When it was started, as the user's clock reads.
        #[serde(default)]
        at: String,
        /// Where it answers.
        #[serde(default)]
        address: Option<String>,
        /// The command that stops it.
        #[serde(default)]
        stop: Option<String>,
    },
    /// The review hat reviewed the uncommitted work (`/audit`, or the offer
    /// after a build turn). The review itself is the turn's answer.
    Reviewed {
        /// Reviewer model id.
        model: String,
        /// Its connection.
        connection: String,
        /// `Some(true)` for `VERDICT: PASS`, `Some(false)` for FAIL, `None`
        /// when it gave neither.
        verdict: Option<bool>,
        /// The files it reviewed, as a git tree: a commit of anything else
        /// was not reviewed.
        #[serde(default)]
        tree: Option<String>,
        /// What the review cost; `None` when unpriced.
        #[serde(default)]
        total_usd: Option<f64>,
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
    /// Where the model list stands: shown from the cache while a fresh one
    /// downloads, or why the fresh one didn't come. `None` when it's current.
    ModelsNote {
        /// One line for the picker.
        note: Option<String>,
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
        /// Display title.
        title: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_tag_is_snake_case() {
        let ev = AgentEvent::ModeChanged {
            role: Role::SoloReview,
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["kind"], "mode_changed");
        assert_eq!(v["role"], "review");
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
