//! Core types, config, spend, and inference for the Ryter coding harness.

#![forbid(unsafe_code)]

pub mod agent;
pub mod cancel;
pub mod clock;
pub mod compact;
pub mod config;
pub mod decisions;
pub mod diff;
pub mod doctor;
pub mod error;
pub mod event;
pub mod gate;
pub mod git;
pub mod hooks;
pub mod ids;
pub mod llm;
pub mod mcp;
pub mod memory;
pub mod page;
pub mod plan;
pub mod project;
pub mod prompt;
pub mod review;
pub mod role;
pub mod rules;
pub mod run;
pub mod sandbox;
pub mod session;
pub mod skill;
pub mod spend;
pub mod testing;
pub mod tools;
pub mod trace;
pub mod update;
pub mod user_io;

pub use agent::{Agent, StopReason, TurnResult, tool_summary};
pub use cancel::Cancel;
pub use compact::{ContextReport, format_context_used, format_tokens, window_for};
pub use config::{
    Config, ConnectionConfig, FeaturesConfig, HookConfig, LastRoute, McpServerConfig, McpSettings,
    PriceOverride, RoleModel, SandboxConfig, SecretStore, SpendConfig, UI_KEYS, UiConfig,
    connection_template, has_secret, list_mcp_tokens, load, load_at, load_last_route,
    load_user_connections, new_inbound_token, resolve_route, resolve_secret, resolve_secret_with,
    save_hooks, save_last_route, save_mcp, save_mcp_tokens, save_settings, save_user_connections,
    store_secret_at, user_connection_names, write_default_config, write_default_connection,
};
pub use doctor::Report as DoctorReport;
pub use error::{Error, Result};
pub use event::AgentEvent;
pub use hooks::{HookDecision, HookEvent, HookSet};
pub use ids::{ConnectionId, SessionId};
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
pub use memory::load_project_memory;
pub use prompt::load_project_instructions;
pub use role::{Role, Thread};
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
