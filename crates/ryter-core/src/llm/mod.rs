//! Inference providers: SpaceXAI, OpenRouter, and generic OpenAI/Anthropic wires.

mod http;
mod parse;

use async_trait::async_trait;
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Mutex;

use crate::config::ConnectionConfig;
use crate::error::Result;
use crate::spend::Usage;

pub use http::HttpProvider;
pub use parse::{Backend, parse_sse};

/// A boxed stream of inference deltas.
pub type DeltaStream = Pin<Box<dyn Stream<Item = Result<StreamDelta>> + Send>>;

/// One step of a streamed completion.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamDelta {
    /// Assistant text.
    Text(String),
    /// Reasoning / thinking text.
    Reasoning(String),
    /// Incremental tool call.
    ToolCall {
        /// Provider id (may be empty on a later delta).
        id: String,
        /// Function name (may be empty on a later delta).
        name: String,
        /// Arguments JSON fragment.
        arguments: String,
    },
    /// Token usage (usually on the last chunk).
    Usage(Usage),
    /// Provider-reported USD (OpenRouter generation cost). Wins over the price book.
    ReportedCost(f64),
    /// The model ran into the output-token ceiling mid-answer. Without this a
    /// truncated turn is indistinguishable from a finished one.
    Truncated,
    /// Stream finished.
    Done,
}

/// A chat message sent to the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// `system`, `user`, `assistant`, or `tool`.
    pub role: String,
    /// Text body.
    #[serde(default)]
    pub content: String,
    /// Tool-call id when role is `tool`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Assistant tool calls (when the model invoked tools).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<AssistantToolCall>>,
}

/// One tool call on an assistant message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantToolCall {
    /// Provider call id.
    pub id: String,
    /// Function name.
    pub name: String,
    /// JSON arguments (full object as a string).
    pub arguments: String,
}

/// Tool advertised to the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    /// Function name.
    pub name: String,
    /// Human description.
    pub description: String,
    /// JSON Schema object for arguments.
    pub parameters: serde_json::Value,
}

/// A completion request.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    /// Model id.
    pub model: String,
    /// Optional system prompt (also may appear as a system message).
    pub system: Option<String>,
    /// Conversation.
    pub messages: Vec<Message>,
    /// Tools. Empty means none.
    pub tools: Vec<ToolSpec>,
    /// Max output tokens.
    pub max_tokens: Option<u32>,
}

/// An entry from `GET /models`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Provider model id.
    pub id: String,
    /// Context window if advertised.
    #[serde(default)]
    pub context_length: Option<u64>,
    /// USD per million input tokens, when the catalog includes it.
    #[serde(default)]
    pub input_per_million: Option<f64>,
    /// USD per million output tokens, when the catalog includes it.
    #[serde(default)]
    pub output_per_million: Option<f64>,
    /// Connection this row was listed from (crew picker).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<String>,
    /// Release time (unix seconds), when the catalog says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<u64>,
    /// Whether the model accepts tools, when the catalog says. A crew role
    /// that cannot call tools cannot read a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<bool>,
}

impl ModelInfo {
    /// Catalog row with an id and optional window.
    pub fn named(id: impl Into<String>, context_length: Option<u64>) -> Self {
        Self {
            id: id.into(),
            context_length,
            input_per_million: None,
            output_per_million: None,
            connection: None,
            created: None,
            tools: None,
        }
    }
}

/// Reassembles streamed tool calls.
///
/// Providers send a call's id and name once and then its arguments in
/// fragments that carry no id (chat completions keys them by `index`, Messages
/// by content-block index). An id-less fragment therefore continues the most
/// recent call. Treating each one as a new call dropped every argument after
/// the first fragment, so real tool calls arrived with empty arguments.
#[derive(Debug, Default)]
pub struct ToolCallAccumulator {
    calls: Vec<AssistantToolCall>,
}

impl ToolCallAccumulator {
    /// Fold one `StreamDelta::ToolCall` in.
    pub fn push(&mut self, id: &str, name: &str, arguments: &str) {
        let target = if id.is_empty() {
            self.calls.last_mut()
        } else {
            self.calls.iter_mut().find(|c| c.id == id)
        };
        match target {
            Some(call) => {
                if !name.is_empty() {
                    call.name = name.to_string();
                }
                call.arguments.push_str(arguments);
            }
            None => self.calls.push(AssistantToolCall {
                id: if id.is_empty() {
                    format!("call_{}", self.calls.len() + 1)
                } else {
                    id.to_string()
                },
                name: name.to_string(),
                arguments: arguments.to_string(),
            }),
        }
    }

    /// Completed calls in arrival order. Nameless fragments are dropped.
    pub fn finish(self) -> Vec<AssistantToolCall> {
        self.calls
            .into_iter()
            .filter(|c| !c.name.is_empty())
            .collect()
    }

    /// Nothing accumulated yet.
    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }
}

/// Streaming inference backend.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Stream a completion.
    async fn stream(&self, req: CompletionRequest) -> Result<DeltaStream>;
    /// List models on this connection.
    async fn list_models(&self) -> Result<Vec<ModelInfo>>;
}

/// Build a live HTTP provider from a connection and a resolved key.
pub fn http_provider(conn: &ConnectionConfig, api_key: String) -> HttpProvider {
    HttpProvider::new(conn, api_key)
}

/// Yields pre-recorded streams. Each `stream` call consumes the next scripted turn.
pub struct ReplayProvider {
    turns: Mutex<VecDeque<Vec<StreamDelta>>>,
    models: Vec<ModelInfo>,
    /// When true, the last remaining turn is cloned instead of consumed.
    repeat_last: bool,
}

impl ReplayProvider {
    /// Replay these deltas on every `stream` call (cloned each time).
    pub fn new(deltas: Vec<StreamDelta>) -> Self {
        Self {
            turns: Mutex::new(VecDeque::from([deltas])),
            models: Vec::new(),
            repeat_last: true,
        }
    }

    /// One recorded SSE document, reused.
    pub fn from_sse(backend: Backend, sse: &str) -> Result<Self> {
        Ok(Self::new(parse_sse(backend, sse)?))
    }

    /// Distinct turns for an agent loop (tool call then final answer, …).
    pub fn scripted(turns: Vec<Vec<StreamDelta>>) -> Self {
        Self {
            turns: Mutex::new(VecDeque::from(turns)),
            models: Vec::new(),
            repeat_last: false,
        }
    }
}

#[async_trait]
impl Provider for ReplayProvider {
    async fn stream(&self, _req: CompletionRequest) -> Result<DeltaStream> {
        let mut q = self.turns.lock().expect("replay mutex");
        let deltas = if self.repeat_last && q.len() == 1 {
            q.front().cloned().unwrap_or_default()
        } else {
            q.pop_front().unwrap_or_default()
        };
        drop(q);
        Ok(Box::pin(futures_util::stream::iter(
            deltas.into_iter().map(Ok),
        )))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(self.models.clone())
    }
}

/// Wire protocol for a connection.
pub fn backend_for(conn: &ConnectionConfig) -> Backend {
    match conn.api_backend.as_str() {
        "responses" => Backend::Responses,
        "messages" => Backend::Messages,
        _ => Backend::ChatCompletions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    #[test]
    fn chat_fixture_streams_text_and_usage() {
        let sse = include_str!("../../fixtures/chat_completions.sse");
        let deltas = parse_sse(Backend::ChatCompletions, sse).unwrap();
        let text: String = deltas
            .iter()
            .filter_map(|d| match d {
                StreamDelta::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "Hello world");
        let usage = deltas.iter().find_map(|d| match d {
            StreamDelta::Usage(u) => Some(*u),
            _ => None,
        });
        assert_eq!(
            usage,
            Some(Usage {
                input_tokens: 12,
                output_tokens: 3,
                cached_tokens: 2,
            })
        );
        assert!(
            deltas
                .iter()
                .any(|d| matches!(d, StreamDelta::Reasoning(_)))
        );
        assert!(deltas.iter().any(|d| matches!(d, StreamDelta::Done)));
    }

    #[test]
    fn responses_fixture() {
        let sse = include_str!("../../fixtures/responses.sse");
        let deltas = parse_sse(Backend::Responses, sse).unwrap();
        let text: String = deltas
            .iter()
            .filter_map(|d| match d {
                StreamDelta::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "Hello world");
        assert!(deltas.iter().any(|d| matches!(
            d,
            StreamDelta::Usage(Usage {
                input_tokens: 12,
                output_tokens: 2,
                ..
            })
        )));
    }

    #[test]
    fn messages_fixture() {
        let sse = include_str!("../../fixtures/messages.sse");
        let deltas = parse_sse(Backend::Messages, sse).unwrap();
        let text: String = deltas
            .iter()
            .filter_map(|d| match d {
                StreamDelta::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "Hi");
        assert!(deltas.iter().any(|d| matches!(
            d,
            StreamDelta::Usage(u) if u.output_tokens == 2
        )));
    }

    #[test]
    fn accumulator_continues_id_less_fragments() {
        let mut a = ToolCallAccumulator::default();
        a.push("c1", "read_file", "");
        a.push("", "", "{\"path\":");
        a.push("", "", "\"a.rs\"}");
        a.push("c2", "grep", "{\"pattern\":\"x\"}");
        let calls = a.finish();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].arguments, "{\"path\":\"a.rs\"}");
        assert_eq!(calls[1].name, "grep");
    }

    #[test]
    fn accumulator_merges_fragments_keyed_by_id() {
        // Responses repeats the item id on every argument fragment.
        let mut a = ToolCallAccumulator::default();
        a.push("fc_1", "read_file", "");
        a.push("fc_1", "", "{\"path\":");
        a.push("fc_1", "", "\"a.rs\"}");
        let calls = a.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments, "{\"path\":\"a.rs\"}");
    }

    #[tokio::test]
    async fn replay_provider_yields_fixture() {
        let sse = include_str!("../../fixtures/chat_completions.sse");
        let p = ReplayProvider::from_sse(Backend::ChatCompletions, sse).unwrap();
        let mut s = p
            .stream(CompletionRequest {
                model: "x".into(),
                system: None,
                messages: vec![],
                tools: vec![],
                max_tokens: None,
            })
            .await
            .unwrap();
        let mut n = 0;
        while let Some(d) = s.next().await {
            d.unwrap();
            n += 1;
        }
        assert!(n >= 3);
    }
}
