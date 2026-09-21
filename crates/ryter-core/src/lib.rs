//! Core types, config, spend, and inference for the Ryter coding harness.

#![forbid(unsafe_code)]

pub mod agent;
pub mod cancel;
pub mod compact;
pub mod config;
pub mod crew;
pub mod doctor;
pub mod error;
pub mod event;
pub mod git;
pub mod hooks;
pub mod ids;
pub mod llm;
pub mod mcp;
pub mod memory;
pub mod phase;
pub mod prompt;
pub mod queue;
pub mod role;
pub mod sandbox;
pub mod session;
pub mod skill;
pub mod spend;
pub mod tools;
pub mod trace;
pub mod user_io;

pub use agent::{Agent, ChildHandle, StopReason, TurnResult, tool_summary};
pub use cancel::Cancel;
pub use compact::{ContextReport, format_context_used, format_tokens, window_for};
pub use config::{
    AuditorConfig, Config, ConnectionConfig, FeaturesConfig, HookConfig, LastRoute,
    McpServerConfig, McpSettings, PriceOverride, RoleModel, SandboxConfig, SpendConfig,
    SubagentsConfig, UI_KEYS, UiConfig, connection_template, has_secret, list_crew_presets,
    list_mcp_tokens, load, load_at, load_crew_preset, load_last_route, load_user_connections,
    new_inbound_token, resolve_route, resolve_secret, resolve_secret_with, save_crew,
    save_crew_preset, save_hooks, save_last_route, save_mcp, save_mcp_tokens, save_settings,
    save_user_connections, store_secret_at, user_connection_names, write_default_config,
    write_default_connection,
};
pub use doctor::Report as DoctorReport;
pub use error::{Error, Result};
pub use event::AgentEvent;
pub use hooks::{HookDecision, HookEvent, HookSet};
pub use ids::{ConnectionId, SessionId, SubagentId};
pub use llm::{
    AssistantToolCall, Backend, CompletionRequest, HttpProvider, Message, ModelInfo, Provider,
    ReplayProvider, StreamDelta, ToolSpec, http_provider,
};
pub use mcp::{
    EchoHost, InboundHost, McpHub, StatusSnapshot, check_tcp, default_socket_path, serve_echo,
    serve_inbound, serve_session, serve_tcp,
};

#[cfg(unix)]
pub use mcp::{bind_unix, serve_unix};
pub use memory::{ensure_project_memory, load_project_memory};
pub use phase::Phase;
pub use prompt::{PromptKind, load_project_instructions, orchestrator_system, specialist_messages};
pub use role::Role;
pub use sandbox::SandboxProfile;
pub use session::{Session, SessionInfo, SpendRecord};
pub use skill::{
    Skill, SlashCatalog, UserCommand, load_catalog, remove_catalog_entry, write_command,
    write_skill,
};
pub use spend::{PriceBook, Rates, Usage, format_rates, format_rates_short, format_usd};
pub use tools::{
    Decision, ToolContext, ToolOutput, decide, gated_execute, specs_for, specs_for_opts, tools_for,
};
pub use user_io::{Permission, UserIo, UserRequest};

/// Crate version, same as the CLI binary.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
