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
    /// `/sessions`.
    Sessions(SessionsMode),
    /// `/spend`.
    Spend,
    /// `$` on the ledger: the spend drawer.
    SpendDrawer,
    /// `/budget`.
    Budget,
    /// `/settings`.
    Settings,
    /// `/theme`.
    Theme,
    /// `/tools`.
    Tools,
    /// `/mcp`.
    Mcp,
    /// `/skills`.
    Skills,
    /// `/rules`.
    Rules,
    /// `/hooks`.
    Hooks,
    /// `/context`.
    Context,
    /// `/help`.
    Help,
    /// `/doctor`.
    Doctor,
    /// `/changes`.
    Changes,
    /// `/commit`.
    Commit,
    /// `Shift+Tab` from a primary hat: the specialists to choose from.
    Specialists,
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
    /// Submit the user's message.
    Submit(String),
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
    /// Begin secret capture for a connection, or for the search provider.
    BeginSetKey(String),
    /// Choose where `web_search` looks (`/provider`): `tavily`, `searxng`
    /// with its address, or empty for none. Saved to settings.toml.
    SetSearch {
        /// The provider word.
        provider: String,
        /// A SearXNG server's address.
        url: Option<String>,
    },
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
    /// Tool permission mode: ask, always, or yolo.
    SetTools {
        /// The mode.
        mode: ryter_core::ToolsMode,
    },
    /// Give a hat its own model (and persist).
    SetHatModel {
        /// The hat: `plan`, `build`, or `review`.
        role: String,
        /// Connection.
        connection: String,
        /// Model.
        model: String,
    },
    /// Put a hat back to following the model every hat uses.
    ResetHatModel(String),
    /// List every connection's models, for a hat's seat.
    ListAllModels {
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
    /// Set a model's reasoning level (`None` is auto), wherever it runs.
    SetModelReasoning {
        /// Model id.
        model: String,
        /// `low` / `medium` / `high` / `default`, or `None` for auto.
        level: Option<String>,
    },
    /// Switch hats.
    SetMode(ryter_core::Role),
    /// `/undo [force]`: put back what the last build turn changed.
    Undo {
        /// Even over the user's later edits to those files.
        force: bool,
    },
    /// `/redo [force]`: reverse the last undo.
    Redo {
        /// Even over the user's edits since the undo.
        force: bool,
    },
    /// `/stop`: stop the product Ryter started for a test.
    StopProduct,
    /// The answer to "stop the project?" on quit: stop it and leave, or
    /// leave it running.
    QuitAnswer {
        /// Stop it first.
        stop: bool,
    },
    /// `/audit`: the audit hat audits the uncommitted work now.
    ReviewNow,
    /// `y` on the audit popout: the build hat repairs what the audit
    /// found, the audit's file as its brief.
    RepairFromAudit {
        /// The audit's file, as a path in the project; `None` when none
        /// was written.
        file: Option<String>,
    },
    /// `o` on the audit popout: open the audit's file.
    OpenAuditFile(String),
    /// `/changes` `x`: put one file back as `base` had it.
    Revert {
        /// Commit to restore from.
        base: String,
        /// Repository-relative path.
        path: String,
    },
    /// The workbench's `x`: put one change (hunk) of a file back.
    RevertHunk {
        /// Commit to restore from.
        base: String,
        /// Repository-relative path.
        path: String,
        /// Which change, as the workbench numbers them.
        hunk: usize,
    },
    /// Open the workbench (`^T`, or `/changes` on the ledger).
    OpenWorkbench,
    /// `/commit`: draft a message for these paths.
    DraftCommit(Vec<String>),
    /// `/commit`: commit these paths with this message.
    Commit {
        /// Repository-relative paths.
        paths: Vec<String>,
        /// Full message, receipt included.
        message: String,
    },
    /// Turn commit receipts on or off, and remember it.
    SetReceipts(bool),
    /// Save everything the `/budget` panel edits.
    SaveBudget {
        /// Session cap; `0` is off.
        usd: f64,
        /// Warn threshold.
        warn: f64,
    },
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
    /// The mouse button came up over a selection: copy what it covers.
    CopySelection(crate::select::Selection),
    /// Reply to a plan.
    PlanReply(ryter_core::user_io::PlanAnswer),
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
