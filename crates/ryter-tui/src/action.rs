//! Side effects the event loop performs on behalf of commands, keys, and panels.

use ryter_core::{ConnectionConfig, Permission};

/// Session browser entry mode (`R-POP-38`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionsMode {
    /// Resume on Enter.
    Browse,
    /// Rename focused.
    Rename,
    /// Delete focused.
    Delete,
}

/// Panels the loop can open (`panel::open`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelId {
    /// `/provider`.
    Providers,
    /// `/models`.
    Models,
    /// `/crew`.
    Crew,
    /// The crew builder (from `/crew`, or on first launch).
    CrewBuilder,
    /// `/agents`.
    Agents,
    /// `/sessions`.
    Sessions(SessionsMode),
    /// `/spend`.
    Spend,
    /// `/budget`.
    Budget,
    /// `/settings`.
    Settings,
    /// `/theme`.
    Theme,
    /// `/tools`.
    Tools,
    /// `/auditor`.
    Auditor,
    /// `/mcp`.
    Mcp,
    /// `/skills`.
    Skills,
    /// `/hooks`.
    Hooks,
    /// `/context`.
    Context,
    /// `/help`.
    Help,
    /// `/doctor`.
    Doctor,
}

/// Everything the loop knows how to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Nothing extra.
    None,
    /// Leave the TUI.
    Quit,
    /// Clear and repaint the terminal (`Ctrl+L`).
    Redraw,
    /// Release or re-grab the mouse (`Ctrl+G`).
    ToggleMouse,
    /// Start a new session.
    New,
    /// Submit text to the orchestrator.
    Submit(String),
    /// Toggle auditor gate.
    SetAuditor(bool),
    /// Switch and persist the theme.
    SetTheme(String),
    /// Live-preview a theme without persisting.
    PreviewTheme(String),
    /// Drop the preview.
    RevertTheme,
    /// Ask the worker for `/context`.
    Context,
    /// Force a compact pass.
    Compact,
    /// Switch the active connection.
    UseConnection(String),
    /// Persist an API key for `name`.
    SetKey {
        /// Connection.
        name: String,
        /// Secret.
        key: String,
    },
    /// Begin secret capture for a connection.
    BeginSetKey(String),
    /// Live connectivity test.
    TestConnection(String),
    /// Write a new user connection.
    AddConnection {
        /// Name.
        name: String,
        /// Definition.
        conn: ConnectionConfig,
    },
    /// Remove a user connection.
    RemoveConnection(String),
    /// Switch model id on the current connection.
    SetModel(String),
    /// Tool permission: `true` = always approve Ask.
    SetTools {
        /// Always.
        always: bool,
    },
    /// Assign a specialist model (and persist).
    SetCrewRole {
        /// Role.
        role: String,
        /// Connection.
        connection: String,
        /// Model.
        model: String,
    },
    /// Reset a role to follow the orchestrator.
    ResetCrewRole(String),
    /// Save the live crew as a named preset.
    SaveCrewPreset(String),
    /// Load a named crew preset.
    LoadCrewPreset(String),
    /// Delete a named crew preset.
    DeleteCrewPreset(String),
    /// Apply a suggested tiered crew (`/crew` → `s`).
    ApplyCrewTiering(std::collections::BTreeMap<String, ryter_core::RoleModel>),
    /// List models for a specialist assignment (all connections).
    ListCrewModels {
        /// Role.
        role: String,
    },
    /// Stop the in-flight turn.
    Cancel,
    /// Open a popout.
    OpenPanel(PanelId),
    /// Load a saved session.
    Resume(String),
    /// Delete a saved session.
    DeleteSession(String),
    /// Set the current session title.
    RenameSession(String),
    /// Set the session budget in USD; `0` turns it off. Applies now and is
    /// saved as the default.
    SetBudget(f64),
    /// Switch hats (solo mode), or to the crew lead (`Orchestrator`).
    SetMode(ryter_core::Role),
    /// `/crew` from solo mode: the crew builder first time, then crew mode.
    EnterCrew,
    /// `/undo`: put files back as they were before the last build turn.
    Undo,
    /// Test that each `(connection, model)` answers a tiny request with a tool.
    ProbeModels(Vec<(String, String)>),
    /// Save what the crew builder chose: the lead's route, the crew, the budget.
    SaveCrewSetup {
        /// Lead connection.
        lead_connection: String,
        /// Lead model.
        lead_model: String,
        /// Architect, builder, auditor.
        crew: std::collections::BTreeMap<String, ryter_core::RoleModel>,
        /// Session budget; `0` is off.
        budget: f64,
        /// Per-task cap that fits this crew's normal work.
        task_cap: f64,
    },
    /// Save everything the `/budget` panel edits.
    SaveBudget {
        /// Session cap; `0` is off.
        usd: f64,
        /// Warn threshold.
        warn: f64,
        /// Per-task cap.
        task: f64,
    },
    /// Kill a running specialist.
    KillAgent(String),
    /// Kill every running specialist.
    KillAllAgents,
    /// Persist MCP config and reconnect outbound servers.
    SaveMcp,
    /// Bind TCP inbound on the running TUI.
    McpListenTcp,
    /// Write a skill stub and reload the catalog.
    WriteSkill {
        /// Slash name.
        name: String,
        /// One-line description.
        description: String,
    },
    /// Write a user-command stub.
    WriteCommand {
        /// Slash name.
        name: String,
    },
    /// Delete a skill or command file.
    RemoveCatalog(std::path::PathBuf),
    /// Open a catalog file in `$EDITOR`.
    EditCatalog(std::path::PathBuf),
    /// Persist hooks and reload the agent set.
    SaveHooks,
    /// Persist settings.toml and apply live knobs.
    SaveSettings,
    /// Reply to a permission prompt.
    PermissionReply(Permission),
    /// Reply to `ask_user`.
    AskUserReply(String),
    /// Trust or skip project `.ryter/`.
    TrustProject(bool),
    /// Write `~/.ryter/spend-<session>.csv`.
    ExportSpend,
    /// Write `~/.ryter/doctor-report.txt`.
    SaveDoctorReport,
    /// Several in order.
    Many(Vec<Action>),
}
