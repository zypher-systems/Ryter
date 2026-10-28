//! Inference providers: SpaceXSI, OpenRouter, Anthropic, OpenAI, and generic wires.

mod http;
#[cfg(test)]
pub(crate) use http::parse_models_json;
pub mod model_cache;
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
    /// Start of an Anthropic `thinking` or `redacted_thinking` content block.
    ///
    /// OpenRouter-style `Reasoning` deltas alone are display-only; this opens
    /// a block that must be echoed on the next Messages request when tools run.
    ThinkingOpen {
        /// Content-block index on the wire.
        stream_key: Option<String>,
        /// Set when the block is `redacted_thinking` (payload arrives at start).
        redacted_data: Option<String>,
    },
    /// Cryptographic signature for the open thinking block (`signature_delta`).
    ThinkingSignature {
        stream_key: Option<String>,
        signature: String,
    },
    /// End of the open thinking / redacted_thinking block.
    ThinkingClose { stream_key: Option<String> },
    /// Incremental tool call.
    ToolCall {
        /// Stable stream identity: a wire index or Responses item id.
        /// Separate from the call id sent back with the tool result.
        stream_key: Option<String>,
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
    /// The host that served the call, where a router names one: OpenRouter's
    /// `provider` on every chunk. What a call cost depends on it.
    Host(String),
    /// The model ran into the output-token ceiling mid-answer. Without this a
    /// truncated turn is indistinguishable from a finished one.
    Truncated,
    /// Stream finished.
    Done,
}

/// One Anthropic Messages content block kept for wire echo.
///
/// Extended thinking with tools requires the assistant turn to come back
/// complete and unmodified, including every `thinking` / `redacted_thinking`
/// block and its signature. Stored in arrival order so interleaved
/// thinking → tool_use → thinking → tool_use survives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessagesBlock {
    /// Extended-thinking block with signature.
    Thinking { thinking: String, signature: String },
    /// Safety-redacted thinking; `data` must be returned unchanged.
    RedactedThinking { data: String },
    /// Assistant text that sat between thinking / tool blocks on the wire.
    Text { text: String },
    /// Tool call in Messages shape (`input` is a JSON object).
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
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
    /// Anthropic Messages content in wire order, when the turn included
    /// thinking blocks. `messages_body` sends this list instead of rebuilding
    /// from `content` + `tool_calls`. Other backends ignore it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messages_blocks: Option<Vec<MessagesBlock>>,
}

impl Message {
    /// A message with no tool calls and no Messages echo blocks.
    pub fn plain(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            tool_call_id: None,
            tool_calls: None,
            messages_blocks: None,
        }
    }
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
    /// Reasoning effort (`low`, `medium`, `high`). OpenRouter gets
    /// `reasoning.effort`; Anthropic Messages gets extended thinking with a
    /// matching `budget_tokens`. `None` / `default` sends nothing, and then
    /// some models reason without limit: glm-5.3-flashx spent 191s and ~27k
    /// tokens thinking before its first tool call.
    pub reasoning: Option<String>,
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
    /// USD per million cached input tokens, when the catalog prices them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_per_million: Option<f64>,
    /// USD per million tokens written to the cache, when the catalog prices them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_per_million: Option<f64>,
    /// Connection this row was listed from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<String>,
    /// Release time (unix seconds), when the catalog says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<u64>,
    /// Whether the model accepts tools, when the catalog says. A model
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
            cached_per_million: None,
            cache_write_per_million: None,
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
/// by content-block index). Keep that identity through parsing, so parallel
/// calls can interleave their arguments without joining different JSON values.
#[derive(Debug, Default)]
pub struct ToolCallAccumulator {
    calls: Vec<AssistantToolCall>,
    /// Whether the provider named the call's id; a made-up one can be replaced.
    given: Vec<bool>,
    keys: std::collections::HashMap<String, usize>,
    active: Option<usize>,
}

impl ToolCallAccumulator {
    /// The call arriving now, as far as it has come.
    pub fn last(&self) -> Option<&AssistantToolCall> {
        self.active.and_then(|i| self.calls.get(i))
    }

    /// Fold one `StreamDelta::ToolCall` in.
    pub fn push(&mut self, id: &str, name: &str, arguments: &str) {
        self.push_keyed(None, id, name, arguments);
    }

    /// Fold a fragment with the protocol's identity. A legacy fragment
    /// without any identity continues the last active call.
    ///
    /// The id decides when it disagrees with the key: Ollama's
    /// chat-completions stream (and some gateways) send every parallel call
    /// as `index: 0` with its own id, and those are separate calls.
    pub fn push_keyed(&mut self, key: Option<&str>, id: &str, name: &str, arguments: &str) {
        let by_id = || self.calls.iter().position(|c| c.id == id);
        let target = match key {
            Some(key) => match self.keys.get(key).copied() {
                Some(index)
                    if !id.is_empty() && self.given[index] && self.calls[index].id != id =>
                {
                    by_id()
                }
                keyed => keyed,
            },
            None if !id.is_empty() => by_id(),
            None => self.active,
        };
        let index = match target {
            Some(index) => {
                let call = &mut self.calls[index];
                if !id.is_empty() {
                    call.id = id.to_string();
                    self.given[index] = true;
                }
                if !name.is_empty() {
                    call.name = name.to_string();
                }
                call.arguments.push_str(arguments);
                index
            }
            None => {
                let index = self.calls.len();
                self.calls.push(AssistantToolCall {
                    id: if id.is_empty() {
                        format!("call_{}", self.calls.len() + 1)
                    } else {
                        id.to_string()
                    },
                    name: name.to_string(),
                    arguments: arguments.to_string(),
                });
                self.given.push(!id.is_empty());
                index
            }
        };
        if let Some(key) = key {
            self.keys.insert(key.to_string(), index);
        }
        self.active = Some(index);
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

/// Rebuilds Anthropic Messages content blocks for echo-back after tools.
///
/// Only activates once a thinking / redacted_thinking block opens. Until then
/// the agent keeps using `content` + `tool_calls` alone (no wire change).
#[derive(Debug, Default)]
pub struct MessagesEchoAccumulator {
    blocks: Vec<MessagesBlock>,
    open_thinking: Option<OpenThinking>,
    pending_text: String,
    /// True after the first thinking block; from then on text and tool_use
    /// join the echo list in arrival order.
    active: bool,
    /// Argument JSON fragments for each `ToolUse` entry in `blocks`.
    tool_args: Vec<String>,
    /// Open tool_use block index in `blocks`, keyed by stream identity.
    open_tools: std::collections::HashMap<String, usize>,
    /// Fallback when a fragment has no stream key.
    last_tool: Option<usize>,
}

#[derive(Debug)]
enum OpenThinking {
    Plain { text: String, signature: String },
    Redacted { data: String },
}

impl MessagesEchoAccumulator {
    /// Whether any thinking block has been seen (echo list is live).
    pub fn active(&self) -> bool {
        self.active
    }

    /// Fold one stream delta that may belong on the Messages echo list.
    pub fn push(&mut self, delta: &StreamDelta) {
        match delta {
            StreamDelta::ThinkingOpen {
                redacted_data: Some(data),
                ..
            } => {
                self.flush_text();
                self.active = true;
                self.open_thinking = Some(OpenThinking::Redacted { data: data.clone() });
            }
            StreamDelta::ThinkingOpen {
                redacted_data: None,
                ..
            } => {
                self.flush_text();
                self.active = true;
                self.open_thinking = Some(OpenThinking::Plain {
                    text: String::new(),
                    signature: String::new(),
                });
            }
            StreamDelta::Reasoning(text) if self.open_thinking.is_some() => {
                if let Some(OpenThinking::Plain { text: buf, .. }) = &mut self.open_thinking {
                    buf.push_str(text);
                }
            }
            StreamDelta::ThinkingSignature { signature, .. } => {
                if let Some(OpenThinking::Plain { signature: sig, .. }) = &mut self.open_thinking {
                    *sig = signature.clone();
                }
            }
            StreamDelta::ThinkingClose { .. } => {
                if let Some(open) = self.open_thinking.take() {
                    self.push_thinking(open);
                }
            }
            StreamDelta::Text(t) if self.active => {
                self.pending_text.push_str(t);
            }
            StreamDelta::ToolCall {
                stream_key,
                id,
                name,
                arguments,
            } if self.active => self.push_tool(stream_key.as_deref(), id, name, arguments),
            _ => {}
        }
    }

    fn push_thinking(&mut self, open: OpenThinking) {
        self.blocks.push(match open {
            OpenThinking::Plain { text, signature } => MessagesBlock::Thinking {
                thinking: text,
                signature,
            },
            OpenThinking::Redacted { data } => MessagesBlock::RedactedThinking { data },
        });
        self.tool_args.push(String::new());
    }

    fn push_tool(&mut self, stream_key: Option<&str>, id: &str, name: &str, arguments: &str) {
        self.flush_text();
        let key = stream_key
            .filter(|k| !k.is_empty())
            .map(str::to_string)
            .or_else(|| (!id.is_empty()).then(|| id.to_string()));
        let continue_last = id.is_empty() && name.is_empty();
        let target = key
            .as_ref()
            .and_then(|k| self.open_tools.get(k).copied())
            .or(self.last_tool.filter(|_| continue_last));
        let index = match target {
            Some(index) => {
                if let MessagesBlock::ToolUse {
                    id: bid,
                    name: bname,
                    ..
                } = &mut self.blocks[index]
                {
                    if !id.is_empty() {
                        *bid = id.to_string();
                    }
                    if !name.is_empty() {
                        *bname = name.to_string();
                    }
                }
                index
            }
            None => {
                let index = self.blocks.len();
                self.blocks.push(MessagesBlock::ToolUse {
                    id: if id.is_empty() {
                        format!("call_{}", index + 1)
                    } else {
                        id.to_string()
                    },
                    name: name.to_string(),
                    input: serde_json::json!({}),
                });
                self.tool_args.push(String::new());
                if let Some(k) = key {
                    self.open_tools.insert(k, index);
                }
                index
            }
        };
        self.tool_args[index].push_str(arguments);
        self.last_tool = Some(index);
    }

    fn flush_text(&mut self) {
        if self.pending_text.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.pending_text);
        self.blocks.push(MessagesBlock::Text { text });
        self.tool_args.push(String::new());
    }

    /// Finished echo list, or `None` when the turn had no thinking blocks.
    pub fn finish(mut self) -> Option<Vec<MessagesBlock>> {
        if !self.active {
            return None;
        }
        if let Some(open) = self.open_thinking.take() {
            self.push_thinking(open);
        }
        self.flush_text();
        for (block, args) in self.blocks.iter_mut().zip(self.tool_args.iter()) {
            if let MessagesBlock::ToolUse { input, .. } = block {
                *input = serde_json::from_str(args).unwrap_or_else(|_| serde_json::json!({}));
            }
        }
        self.blocks.retain(|b| match b {
            MessagesBlock::ToolUse { name, .. } => !name.is_empty(),
            _ => true,
        });
        Some(self.blocks)
    }
}

/// Streaming inference backend.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Stream a completion.
    async fn stream(&self, req: CompletionRequest) -> Result<DeltaStream>;
    /// List models on this connection.
    async fn list_models(&self) -> Result<Vec<ModelInfo>>;
    /// Of `models`, the ones this connection serves but this account cannot
    /// use: on OpenRouter, models the account's privacy settings (zero data
    /// retention), provider rules, or guardrails leave with no provider. Their
    /// first request fails. Empty when the provider cannot tell.
    async fn refused(&self, _models: &[String]) -> Vec<String> {
        Vec::new()
    }
}

/// Build a live HTTP provider from a connection and a resolved key.
pub fn http_provider(conn: &ConnectionConfig, api_key: String) -> HttpProvider {
    HttpProvider::new(conn, api_key)
}

/// Yields pre-recorded streams. Each `stream` call consumes the next scripted turn.
pub struct ReplayProvider {
    turns: Mutex<VecDeque<Vec<StreamDelta>>>,
    models: Vec<ModelInfo>,
    refused: Vec<String>,
    /// When set, every [`Provider::stream`] fails with this provider error.
    stream_err: Option<String>,
    /// When true, the last remaining turn is cloned instead of consumed.
    pub(crate) repeat_last: bool,
}

impl ReplayProvider {
    /// Replay these deltas on every `stream` call (cloned each time).
    pub fn new(deltas: Vec<StreamDelta>) -> Self {
        Self {
            turns: Mutex::new(VecDeque::from([deltas])),
            models: Vec::new(),
            refused: Vec::new(),
            stream_err: None,
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
            refused: Vec::new(),
            stream_err: None,
            repeat_last: false,
        }
    }

    /// Answer [`Provider::refused`] as an account that cannot use `models`.
    pub fn refusing(mut self, models: &[&str]) -> Self {
        self.refused = models.iter().map(|m| m.to_string()).collect();
        self
    }

    /// Fail every [`Provider::stream`] with a provider error (for choose-time checks).
    pub fn failing(mut self, err: impl Into<String>) -> Self {
        self.stream_err = Some(err.into());
        self
    }
}

#[async_trait]
impl Provider for ReplayProvider {
    async fn stream(&self, _req: CompletionRequest) -> Result<DeltaStream> {
        if let Some(err) = &self.stream_err {
            return Err(crate::Error::Provider(err.clone()));
        }
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

    async fn refused(&self, models: &[String]) -> Vec<String> {
        models
            .iter()
            .filter(|m| self.refused.contains(m))
            .cloned()
            .collect()
    }
}

/// A provider error a person can act on, for the ones Ryter recognises.
/// Provider errors are long JSON; OpenRouter's refusal under zero data
/// retention runs to a paragraph with routing metadata.
pub fn explain_error(e: &str) -> Option<String> {
    let lower = e.to_ascii_lowercase();
    if lower.contains("data policy") || lower.contains("zero data retention") {
        return Some(
            "not available under your account's data policy (e.g. zero data retention); \
             change it at https://openrouter.ai/settings/privacy or pick another model"
                .into(),
        );
    }
    if lower.contains("tool use") || lower.contains("tools") && lower.contains("support") {
        return Some(
            "no provider serves it with tool use, which Ryter needs to read and change files"
                .into(),
        );
    }
    if lower.contains("not a valid model") {
        return Some("not a model this connection knows (renamed or retired?)".into());
    }
    if lower.contains("max price") || lower.contains("[routing]") {
        return Some(
            "no host passes the connection's [routing] (its max_price, or another of its \
             terms); loosen it with r on the connection's row in /provider (its config.toml table only counts while nothing is saved there), or pick another model"
                .into(),
        );
    }
    if lower.contains("401") || lower.contains("403") || lower.contains("unauthorized") {
        return Some("the key was refused for this model".into());
    }
    if lower.contains("404") || lower.contains("not found") || lower.contains("no endpoints") {
        return Some("not found, or no provider serves it for this account".into());
    }
    if lower.contains("402") || lower.contains("credit") {
        return Some("out of credits".into());
    }
    None
}

/// Can this account actually run `model` with tools? One tiny request with
/// one tool, capped at 16 output tokens: a fraction of a cent. It catches
/// what no catalog says: a data policy that leaves the model no provider,
/// missing tool support, no credits, or a model that was retired. Used when
/// a hat's model is chosen in `/models`, so the failure is at choose time
/// rather than on the first real message.
pub async fn probe_model(provider: &dyn Provider, model: &str) -> std::result::Result<(), String> {
    use futures_util::StreamExt;
    let req = CompletionRequest {
        model: model.to_string(),
        system: None,
        messages: vec![Message {
            role: "user".into(),
            content: "Reply with the single word: ok".into(),
            tool_call_id: None,
            tool_calls: None,
            messages_blocks: None,
        }],
        tools: vec![ToolSpec {
            name: "noop".into(),
            description: "Does nothing. Do not call it.".into(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        }],
        max_tokens: Some(16),
        reasoning: None,
    };
    let run = async {
        let mut stream = provider.stream(req).await.map_err(|e| e.to_string())?;
        while let Some(d) = stream.next().await {
            d.map_err(|e| e.to_string())?;
        }
        Ok::<(), String>(())
    };
    match tokio::time::timeout(std::time::Duration::from_secs(45), run).await {
        Ok(r) => r.map_err(|e| short_reason(&e)),
        Err(_) => Err("no answer in 45s".into()),
    }
}

/// Provider errors are long JSON; keep the part a person can act on.
fn short_reason(e: &str) -> String {
    explain_error(e).unwrap_or_else(|| e.lines().next().unwrap_or(e).chars().take(140).collect())
}

/// A Claude model's generation, `(major, minor)`, from its id:
/// `claude-opus-5-5` is `(5, 5)`, `claude-haiku-4-5` is `(4, 5)`,
/// `anthropic.claude-sonnet-4-6` is `(4, 6)`. A date suffix is not a minor
/// version. `None` for anything that isn't a Claude id.
pub fn claude_version(model: &str) -> Option<(u32, u32)> {
    if !model.contains("claude") {
        return None;
    }
    let mut nums = model
        .split(['-', '.', '@'])
        .filter_map(|s| s.parse::<u32>().ok());
    let major = nums.next()?;
    let minor = nums.next().filter(|n| *n < 100).unwrap_or(0);
    Some((major, minor))
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
                cache_write_tokens: 0,
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
        // Input arrives first, output last; the round is priced on both.
        let usage = deltas
            .iter()
            .fold(crate::spend::Usage::default(), |acc, d| match d {
                StreamDelta::Usage(u) => acc.merge(*u),
                _ => acc,
            });
        assert!(
            usage.input_tokens > 0 && usage.output_tokens == 2,
            "{usage:?}"
        );
    }

    /// Extended thinking with tools: each SSE frame is parsed on its own (as
    /// the HTTP layer does), then the echo accumulator rebuilds the content
    /// list with the thinking signature intact.
    #[test]
    fn messages_thinking_tools_fixture_echoes_signature() {
        let fixture = include_str!("../../fixtures/messages_thinking_tools.sse");
        let mut echo = MessagesEchoAccumulator::default();
        let mut calls = ToolCallAccumulator::default();
        let mut reasoning = String::new();
        for frame in fixture.split("\n\n") {
            let (deltas, _) = parse::parse_blocks(Backend::Messages, frame).unwrap();
            for delta in deltas {
                echo.push(&delta);
                match delta {
                    StreamDelta::Reasoning(t) => reasoning.push_str(&t),
                    StreamDelta::ToolCall {
                        stream_key,
                        id,
                        name,
                        arguments,
                    } => calls.push_keyed(stream_key.as_deref(), &id, &name, &arguments),
                    _ => {}
                }
            }
        }
        assert_eq!(reasoning, "I should read the file first.");
        let calls = calls.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "toolu_1");
        assert_eq!(calls[0].arguments, r#"{"path":"a.rs"}"#);
        let blocks = echo.finish().expect("thinking must activate the echo list");
        assert_eq!(
            blocks,
            vec![
                MessagesBlock::Thinking {
                    thinking: "I should read the file first.".into(),
                    signature: "sig_echo_me_unmodified".into(),
                },
                MessagesBlock::ToolUse {
                    id: "toolu_1".into(),
                    name: "read_file".into(),
                    input: serde_json::json!({"path": "a.rs"}),
                },
            ]
        );
    }

    #[tokio::test]
    async fn replay_provider_yields_thinking_tools_fixture() {
        let sse = include_str!("../../fixtures/messages_thinking_tools.sse");
        let p = ReplayProvider::from_sse(Backend::Messages, sse).unwrap();
        let mut s = p
            .stream(CompletionRequest {
                model: "claude".into(),
                system: None,
                messages: vec![],
                tools: vec![],
                max_tokens: None,
                reasoning: Some("medium".into()),
            })
            .await
            .unwrap();
        let mut echo = MessagesEchoAccumulator::default();
        while let Some(d) = s.next().await {
            echo.push(&d.unwrap());
        }
        let blocks = echo.finish().unwrap();
        assert!(
            matches!(
                &blocks[0],
                MessagesBlock::Thinking { signature, .. }
                    if signature == "sig_echo_me_unmodified"
            ),
            "{blocks:?}"
        );
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

    #[test]
    fn interleaved_protocol_calls_keep_their_arguments_and_result_ids() {
        for (backend, fixture) in [
            (
                Backend::ChatCompletions,
                include_str!("../../fixtures/parallel_chat.sse"),
            ),
            (
                Backend::Messages,
                include_str!("../../fixtures/parallel_messages.sse"),
            ),
            (
                Backend::Responses,
                include_str!("../../fixtures/parallel_responses.sse"),
            ),
        ] {
            let mut calls = ToolCallAccumulator::default();
            // HTTP parses each frame separately; identity cannot depend on
            // state that exists only while parsing a complete fixture.
            for frame in fixture.split("\n\n") {
                let (deltas, _) = parse::parse_blocks(backend, frame).unwrap();
                for delta in deltas {
                    if let StreamDelta::ToolCall {
                        stream_key,
                        id,
                        name,
                        arguments,
                    } = delta
                    {
                        calls.push_keyed(stream_key.as_deref(), &id, &name, &arguments);
                    }
                }
            }
            let calls = calls.finish();
            assert_eq!(calls.len(), 2, "{backend:?}");
            for (call, id) in calls.iter().zip(["a", "b"]) {
                assert_eq!(call.id, id, "{backend:?}");
                assert_eq!(call.name, "read_file");
                assert_eq!(
                    call.arguments,
                    format!("{{\"path\":\"{id}.txt\"}}"),
                    "{backend:?}"
                );
            }
        }
    }

    #[test]
    fn indexed_arguments_can_arrive_before_the_call_definition() {
        let mut calls = ToolCallAccumulator::default();
        calls.push_keyed(Some("0"), "", "", "{\"path\":");
        calls.push_keyed(Some("1"), "b", "read_file", "{\"path\":\"b.txt\"}");
        calls.push_keyed(Some("0"), "a", "read_file", "\"a.txt\"}");
        assert_eq!(calls.last().unwrap().id, "a");
        let calls = calls.finish();
        assert_eq!(calls[0].id, "a");
        assert_eq!(calls[0].arguments, "{\"path\":\"a.txt\"}");
        assert_eq!(calls[1].id, "b");
        assert_eq!(calls[1].arguments, "{\"path\":\"b.txt\"}");
    }

    /// Ollama's chat-completions stream sends every parallel call as
    /// `index: 0` with its own id. Keyed by index alone they became one call
    /// with two JSON values joined.
    #[test]
    fn calls_sharing_an_index_stay_apart_when_their_ids_differ() {
        let mut calls = ToolCallAccumulator::default();
        calls.push_keyed(Some("0"), "a", "read_file", "{\"path\":");
        // The same id again, and no id at all, continue the call.
        calls.push_keyed(Some("0"), "a", "", "\"a");
        calls.push_keyed(Some("0"), "", "", ".txt\"}");
        calls.push_keyed(Some("0"), "b", "read_file", "{\"path\":");
        calls.push_keyed(Some("0"), "", "", "\"b.txt\"}");
        assert_eq!(calls.last().unwrap().id, "b");
        // An id seen before goes back to its own call, as it always did.
        calls.push_keyed(Some("0"), "a", "", " ");
        let calls = calls.finish();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            (calls[0].id.as_str(), calls[0].arguments.as_str()),
            ("a", "{\"path\":\"a.txt\"} ")
        );
        assert_eq!(
            (calls[1].id.as_str(), calls[1].arguments.as_str()),
            ("b", "{\"path\":\"b.txt\"}")
        );
    }

    #[test]
    fn a_stream_reusing_index_zero_parses_into_separate_calls() {
        let frame = |id: &str| {
            format!(
                "data: {{\"choices\":[{{\"delta\":{{\"tool_calls\":[{{\"index\":0,\"id\":\"{id}\",\"type\":\"function\",\"function\":{{\"name\":\"read_file\",\"arguments\":\"{{\\\"path\\\":\\\"{id}.txt\\\"}}\"}}}}]}}}}]}}\n\n"
            )
        };
        let sse = format!("{}{}data: [DONE]\n\n", frame("a"), frame("b"));
        let deltas = parse_sse(Backend::ChatCompletions, &sse).unwrap();
        let mut calls = ToolCallAccumulator::default();
        for delta in deltas {
            if let StreamDelta::ToolCall {
                stream_key,
                id,
                name,
                arguments,
            } = delta
            {
                calls.push_keyed(stream_key.as_deref(), &id, &name, &arguments);
            }
        }
        let calls = calls.finish();
        assert_eq!(calls.len(), 2);
        for (call, id) in calls.iter().zip(["a", "b"]) {
            assert_eq!(call.id, id);
            assert_eq!(call.arguments, format!("{{\"path\":\"{id}.txt\"}}"));
        }
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
                reasoning: None,
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

    #[test]
    fn a_claude_id_gives_its_generation() {
        assert_eq!(claude_version("claude-opus-5-5"), Some((5, 5)));
        assert_eq!(claude_version("claude-opus-5"), Some((5, 0)));
        assert_eq!(claude_version("claude-haiku-4-5"), Some((4, 5)));
        assert_eq!(
            claude_version("claude-sonnet-4-20250514"),
            Some((4, 0)),
            "a date is no minor"
        );
        assert_eq!(claude_version("claude-3-5-sonnet-20241022"), Some((3, 5)));
        assert_eq!(claude_version("anthropic.claude-sonnet-4-6"), Some((4, 6)));
        assert_eq!(claude_version("claude-opus-4-5@20251101"), Some((4, 5)));
        assert_eq!(claude_version("grok-4.6"), None);
        assert_eq!(
            claude_version("anthropic/claude-sonnet-4.6"),
            Some((4, 6)),
            "OpenRouter's spelling"
        );
    }

    #[test]
    fn probe_failures_read_as_reasons() {
        let cases = [
            (
                r#"provider: http 400 Bad Request: {"error":{"message":"nonexistent-vendor/no-such-model is not a valid model ID","code":400}}"#,
                "renamed or retired",
            ),
            (
                r#"http 404: {"error":{"message":"No endpoints found matching your data policy (Zero data retention)"}}"#,
                "data policy",
            ),
            (
                r#"http 404: {"error":{"message":"No endpoints found that support tool use"}}"#,
                "tool use",
            ),
            (
                r#"http 404: {"error":{"message":"No endpoints found that satisfy the max price for this request","metadata":{"failed_routing_step":"Filter by Max Price"}}}"#,
                "[routing]",
            ),
            (
                r#"http 402: {"error":{"message":"Insufficient credits"}}"#,
                "credits",
            ),
            (
                r#"http 401: {"error":{"message":"User not found."}}"#,
                "key was refused",
            ),
        ];
        for (raw, want) in cases {
            let got = short_reason(raw);
            assert!(got.contains(want), "{raw} -> {got}");
            assert!(!got.contains('{'), "no JSON left: {got}");
        }
    }

    #[tokio::test]
    async fn probe_passes_a_stream_that_answers() {
        let p = ReplayProvider::new(vec![StreamDelta::Text("ok".into()), StreamDelta::Done]);
        probe_model(&p, "test-model").await.expect("usable");
    }

    #[tokio::test]
    async fn probe_maps_a_provider_refusal_to_a_short_reason() {
        let p = ReplayProvider::new(vec![]).failing(
            r#"http 404: {"error":{"message":"No endpoints found matching your data policy (Zero data retention)"}}"#,
        );
        let err = probe_model(&p, "refused-model")
            .await
            .expect_err("should fail");
        assert!(err.contains("data policy"), "{err}");
        assert!(!err.contains('{'), "{err}");
    }
}
