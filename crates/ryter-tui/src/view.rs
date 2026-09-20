//! Drawable UI state (no terminal, no agent).

use ryter_core::Phase;
use ryter_core::SlashCatalog;
use ryter_core::format_usd;

/// One configured connection for `/connections` and the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnRow {
    /// Name (`spacexai`).
    pub name: String,
    /// Kind.
    pub kind: String,
    /// Default model id.
    pub model: String,
    /// Whether a key is resolvable (never the secret).
    pub has_key: bool,
}

/// A task queue row for the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoRow {
    /// Title.
    pub title: String,
    /// `pending` / `running` / `done` / `blocked`.
    pub status: String,
}

/// A running specialist row under the header.
#[derive(Debug, Clone, PartialEq)]
pub struct CrewRow {
    /// Subagent id (for `/agents` kill).
    pub id: String,
    /// builder / planner / …
    pub role: String,
    /// Short task label.
    pub label: String,
    /// Optional spend for this child.
    pub spend: Option<f64>,
    /// Status word (`running`, `auditing`, …).
    pub status: String,
}

/// One scrollback row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogLine {
    /// User.
    User(String),
    /// Assistant / streaming text.
    Assistant(String),
    /// Collapsed tool row.
    Tool(String),
    /// System / slash output.
    System(String),
    /// Merge notice.
    Merge(String),
    /// Specialist output (display only; not orchestrator context).
    Specialist { role: String, text: String },
}

/// Which two-or-more-way setting a [`Overlay::Choice`] edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceKind {
    /// Auditor gate.
    Auditor,
    /// Tool permission: ask vs always.
    Tools,
    /// Theme name.
    Theme,
    /// Campaign phase.
    Phase,
    /// Load a saved crew preset.
    CrewPreset,
    /// Resume a saved session.
    Resume,
    /// Delete a saved session.
    DeleteSession,
    /// Trust project `.ryter/`.
    Trust,
    /// Sandbox profile.
    Sandbox,
}

/// Panel inside [`Overlay::Mcp`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpPane {
    /// Inbound summary + outbound list.
    Home,
    /// Links, token, listen toggles.
    Inbound,
    /// Capture outbound server name.
    AddName {
        /// Buffer.
        buf: String,
    },
    /// Capture command.
    AddCommand {
        /// Server name.
        name: String,
        /// Buffer.
        buf: String,
    },
    /// Capture args (space-separated).
    AddArgs {
        /// Server name.
        name: String,
        /// Executable.
        command: String,
        /// Buffer.
        buf: String,
    },
}

/// Panel inside [`Overlay::Skills`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillsPane {
    /// List + add.
    Home,
    /// Arguments before invoke.
    Args {
        /// Skill or command name.
        name: String,
        /// Buffer.
        buf: String,
    },
    /// New skill name.
    AddSkillName {
        /// Buffer.
        buf: String,
    },
    /// New skill description.
    AddSkillDesc {
        /// Name.
        name: String,
        /// Buffer.
        buf: String,
    },
    /// New user-command name.
    AddCmdName {
        /// Buffer.
        buf: String,
    },
}

/// Panel inside [`Overlay::Hooks`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HooksPane {
    /// List + add.
    Home,
    /// Pick event (4 rows).
    AddEvent,
    /// Command or URL.
    AddTarget {
        /// Event name.
        event: String,
        /// Buffer.
        buf: String,
    },
    /// Optional matcher glob.
    AddMatcher {
        /// Event name.
        event: String,
        /// Command or URL.
        target: String,
        /// Buffer.
        buf: String,
    },
}

/// One row in a choice picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceItem {
    /// Value applied on Enter.
    pub id: String,
    /// Shown label.
    pub label: String,
}

/// Floating picker (provider or model).
#[derive(Debug, Clone, PartialEq)]
pub enum Overlay {
    /// Pick a provider; Enter uses it or asks for a key.
    Provider {
        /// Highlighted row.
        selected: usize,
        /// Filter text.
        filter: String,
    },
    /// Pick a model; typing filters.
    Model {
        /// Filter text.
        filter: String,
        /// Highlighted row among filtered items.
        selected: usize,
        /// Full catalog (unfiltered).
        items: Vec<ryter_core::ModelInfo>,
        /// Waiting on `GET /models`.
        loading: bool,
        /// When set, picking a model assigns this specialist and returns to `/crew`.
        assign_role: Option<String>,
    },
    /// Configure specialist models; Enter on a role opens the model picker.
    Crew {
        /// Highlighted row.
        selected: usize,
        /// When set, the composer is capturing a preset name.
        save_buf: Option<String>,
    },
    /// Running specialists; Enter kills the highlighted row.
    Agents {
        /// Highlighted row.
        selected: usize,
    },
    /// Configure inbound and outbound MCP.
    Mcp {
        /// Highlighted row.
        selected: usize,
        /// Which panel.
        pane: McpPane,
    },
    /// Skills and user slash commands.
    Skills {
        /// Highlighted row.
        selected: usize,
        /// Which panel.
        pane: SkillsPane,
    },
    /// Lifecycle hooks.
    Hooks {
        /// Highlighted row.
        selected: usize,
        /// Which panel.
        pane: HooksPane,
    },
    /// Destructive tool `y/n/a`.
    Permission {
        /// Tool name.
        tool: String,
        /// One-line summary.
        summary: String,
    },
    /// `ask_user` prompt.
    AskUser {
        /// Question.
        question: String,
        /// Choices (empty = free text).
        options: Vec<String>,
        /// Highlighted option.
        selected: usize,
        /// Free-text buffer.
        buf: String,
    },
    /// Spend roll-up table.
    Spend,
    /// Budget, max, sandbox, inbound, web.
    Settings {
        /// Highlighted row.
        selected: usize,
        /// Edit buffer when typing a value.
        edit: Option<String>,
    },
    /// Small on/off (or few-way) picker.
    Choice {
        /// Window title.
        title: String,
        /// What Enter should change.
        kind: ChoiceKind,
        /// Options.
        options: Vec<ChoiceItem>,
        /// Highlighted row.
        selected: usize,
        /// Filter text.
        filter: String,
    },
}

/// Slash palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashMenu {
    /// Filter text after `/`.
    pub filter: String,
    /// Matching command names.
    pub matches: Vec<String>,
    /// Highlighted index.
    pub selected: usize,
}

/// Everything the draw path needs.
#[derive(Debug, Clone)]
pub struct View {
    /// Current phase.
    pub phase: Phase,
    /// Connection name.
    pub connection: String,
    /// Model id.
    pub model: String,
    /// Known USD total.
    pub spend: Option<f64>,
    /// Some turns unpriced.
    pub spend_unknown: bool,
    /// Context fill 0–100, if known.
    pub ctx_pct: Option<u8>,
    /// Scrollback (oldest first).
    pub lines: Vec<LogLine>,
    /// Parallel specialists.
    pub crew: Vec<CrewRow>,
    /// Composer contents (no prompt char).
    pub composer: String,
    /// Slash overlay.
    pub slash: Option<SlashMenu>,
    /// Handoff in progress.
    pub handoff_to: Option<Phase>,
    /// Permission prompt.
    pub permission: Option<String>,
    /// Model is running.
    pub busy: bool,
    /// Project path shown in the header (`~/workspace/ryter`).
    pub cwd: String,
    /// Current git branch, if any.
    pub git_branch: Option<String>,
    /// `ask` / `always`.
    pub perm_mode: String,
    /// Skills + user slash commands.
    pub catalog: SlashCatalog,
    /// Preformatted `/hooks` listing.
    pub hooks_help: String,
    /// Editable hook rows (persisted to `hooks.toml`).
    pub hooks: Vec<ryter_core::HookConfig>,
    /// Active theme name.
    pub theme_name: String,
    /// Built-ins plus `~/.ryter/themes/*.toml`.
    pub theme_names: Vec<String>,
    /// Known connections.
    pub connections: Vec<ConnRow>,
    /// Whether the active connection has a key.
    pub has_key: bool,
    /// Auditor gate.
    pub auditor_on: bool,
    /// Task list for the sidebar.
    pub todos: Vec<TodoRow>,
    /// Last known token count.
    pub ctx_tokens: Option<u64>,
    /// Window used for the context bar.
    pub ctx_window: Option<u64>,
    /// Connection name we are capturing an API key for.
    pub secret_for: Option<String>,
    /// Key buffer (never drawn in the clear).
    pub secret_buf: String,
    /// Floating picker.
    pub overlay: Option<Overlay>,
    /// `$2/M input / $6/M output` for the active model.
    pub price_label: String,
    /// Last known USD / million input (for last.toml).
    pub price_in: Option<f64>,
    /// Last known USD / million output (for last.toml).
    pub price_out: Option<f64>,
    /// Live `[specialists.*]` assignment (edited by `/crew`).
    pub specialists: std::collections::BTreeMap<String, ryter_core::RoleModel>,
    /// Unix socket path if inbound MCP is listening.
    pub mcp_listen: Option<String>,
    /// Current session id.
    pub session_id: String,
    /// Inbound MCP enabled.
    pub mcp_inbound: bool,
    /// Configured TCP bind (`127.0.0.1:8765`).
    pub mcp_bind: Option<String>,
    /// Bound TCP address, if the TUI started a listener.
    pub mcp_tcp_listen: Option<String>,
    /// Outbound MCP servers.
    pub mcp_servers: std::collections::BTreeMap<String, ryter_core::McpServerConfig>,
    /// Named inbound bearer tokens (full secret; overlay masks unless revealed).
    pub mcp_tokens: Vec<(String, String)>,
    /// Token name to show in the clear.
    pub mcp_reveal: Option<String>,
    /// Known USD by role name.
    pub spend_by_role: std::collections::BTreeMap<String, f64>,
    /// Known USD by connection.
    pub spend_by_conn: std::collections::BTreeMap<String, f64>,
    /// Session budget cap (0 = none).
    pub budget_usd: f64,
    /// Warn threshold.
    pub warn_usd: f64,
    /// `[subagents] max`.
    pub max_crew: u32,
    /// Sandbox profile name.
    pub sandbox_profile: String,
    /// `[features] web`.
    pub web: bool,
}

impl View {
    /// Empty session chrome for tests / startup.
    pub fn new(phase: Phase, connection: String, model: String, cwd: String) -> Self {
        Self {
            phase,
            connection,
            model,
            spend: None,
            spend_unknown: false,
            ctx_pct: None,
            lines: Vec::new(),
            crew: Vec::new(),
            composer: String::new(),
            slash: None,
            handoff_to: None,
            permission: None,
            busy: false,
            cwd,
            git_branch: None,
            perm_mode: "ask".into(),
            catalog: SlashCatalog::default(),
            hooks_help: String::new(),
            hooks: Vec::new(),
            theme_name: "dark".into(),
            theme_names: vec!["default-16".into(), "dark".into()],
            connections: Vec::new(),
            has_key: false,
            auditor_on: true,
            todos: Vec::new(),
            ctx_tokens: None,
            ctx_window: None,
            secret_for: None,
            secret_buf: String::new(),
            overlay: None,
            price_label: String::new(),
            price_in: None,
            price_out: None,
            specialists: std::collections::BTreeMap::new(),
            mcp_listen: None,
            session_id: String::new(),
            mcp_inbound: true,
            mcp_bind: None,
            mcp_tcp_listen: None,
            mcp_servers: std::collections::BTreeMap::new(),
            mcp_tokens: Vec::new(),
            mcp_reveal: None,
            spend_by_role: std::collections::BTreeMap::new(),
            spend_by_conn: std::collections::BTreeMap::new(),
            budget_usd: 5.0,
            warn_usd: 1.0,
            max_crew: 4,
            sandbox_profile: "off".into(),
            web: false,
        }
    }

    /// Rows on the MCP home list (inbound + servers + add).
    pub fn mcp_home_len(&self) -> usize {
        1 + self.mcp_servers.len() + 1
    }

    /// Sorted outbound server names.
    pub fn mcp_server_names(&self) -> Vec<String> {
        self.mcp_servers.keys().cloned().collect()
    }

    /// stdio command clients spawn.
    pub fn mcp_stdio_cmd(&self) -> &'static str {
        "ryter mcp serve"
    }

    /// Unix URI for attach, if listening.
    pub fn mcp_unix_uri(&self) -> Option<String> {
        self.mcp_listen.as_ref().map(|p| {
            if p.starts_with('/') {
                format!("unix://{p}")
            } else {
                format!("unix:///{p}")
            }
        })
    }

    /// JSON snippet for a client `mcp.json`.
    pub fn mcp_client_snippet(&self) -> String {
        r#"{"mcpServers":{"ryter":{"command":"ryter","args":["mcp","serve"]}}}"#.into()
    }

    /// Skills then user commands, then two add rows.
    pub fn skills_home_len(&self) -> usize {
        self.catalog.skills.len() + self.catalog.commands.len() + 2
    }

    /// Configured hooks plus add.
    pub fn hooks_home_len(&self) -> usize {
        self.hooks.len() + 1
    }

    /// Filtered model rows for the picker.
    pub fn filtered_models(&self) -> Vec<&ryter_core::ModelInfo> {
        let Some(Overlay::Model { filter, items, .. }) = &self.overlay else {
            return Vec::new();
        };
        let f = filter.to_ascii_lowercase();
        items
            .iter()
            .filter(|m| {
                if m.id.is_empty() {
                    return true;
                }
                if f.is_empty() {
                    return true;
                }
                let id = m.id.to_ascii_lowercase();
                let short = crate::chat::short_model(&m.id).to_ascii_lowercase();
                let conn = m.connection.as_deref().unwrap_or("").to_ascii_lowercase();
                id.contains(&f) || short.contains(&f) || conn.contains(&f)
            })
            .collect()
    }

    /// Filtered provider rows.
    pub fn filtered_providers(&self) -> Vec<&ConnRow> {
        let filter = match &self.overlay {
            Some(Overlay::Provider { filter, .. }) => filter.to_ascii_lowercase(),
            _ => String::new(),
        };
        self.connections
            .iter()
            .filter(|c| {
                filter.is_empty()
                    || c.name.to_ascii_lowercase().contains(&filter)
                    || c.kind.to_ascii_lowercase().contains(&filter)
            })
            .collect()
    }

    /// Filtered rows for a choice overlay.
    pub fn filtered_choices(&self) -> Vec<&ChoiceItem> {
        let Some(Overlay::Choice {
            filter, options, ..
        }) = &self.overlay
        else {
            return Vec::new();
        };
        let f = filter.to_ascii_lowercase();
        options
            .iter()
            .filter(|c| {
                f.is_empty()
                    || c.id.to_ascii_lowercase().contains(&f)
                    || c.label.to_ascii_lowercase().contains(&f)
            })
            .collect()
    }

    /// Spend label for the header.
    pub fn spend_label(&self) -> String {
        if self.spend_unknown && self.spend.is_none() {
            format_usd(None)
        } else {
            format_usd(self.spend)
        }
    }
}

/// Built-in slash commands.
pub fn slash_commands() -> &'static [&'static str] {
    &[
        "quit",
        "new",
        "spend",
        "provider",
        "models",
        "handoff",
        "plan",
        "architect",
        "build",
        "audit",
        "help",
        "theme",
        "auditor",
        "mcp",
        "skills",
        "hooks",
        "context",
        "compact",
        "doctor",
        "settings",
        "tools",
        "ask",
        "auto",
        "always",
        "phase",
        "crew",
        "cancel",
        "resume",
        "sessions",
        "rename",
        "delete",
        "agents",
    ]
}

/// Specialist kinds shown in `/crew`.
pub const CREW_ROLES: &[&str] = &["planner", "architect", "builder", "auditor"];

/// Rows on the MCP inbound pane.
pub const MCP_INBOUND_ROWS: usize = 6;

/// Rows on `/settings` (budget, warn, max, sandbox, inbound, web).
pub const SETTINGS_ROWS: usize = 6;

/// Label under a crew role (`default (grok-4.6)` or a short model id).
pub fn crew_role_label(view: &View, role: &str) -> String {
    let orch = shortish(&view.model);
    let Some(rm) = view.specialists.get(role) else {
        return format!("default ({orch})");
    };
    if !rm.is_override() {
        return format!("default ({orch})");
    }
    let conn = rm.connection.as_deref().unwrap_or(view.connection.as_str());
    let model = rm.model.as_deref().unwrap_or(view.model.as_str());
    if conn == view.connection && model == view.model {
        format!("default ({orch})")
    } else {
        format!("{} · {conn}", crate::chat::short_model(model))
    }
}

fn shortish(model: &str) -> String {
    crate::chat::short_model(model).to_string()
}

/// Options for a choice overlay.
pub fn choice_options(kind: ChoiceKind, view: &View) -> Vec<ChoiceItem> {
    match kind {
        ChoiceKind::Auditor => vec![
            ChoiceItem {
                id: "on".into(),
                label: "on  — audit then auto-merge".into(),
            },
            ChoiceItem {
                id: "off".into(),
                label: "off — merge without audit".into(),
            },
        ],
        ChoiceKind::Tools => vec![
            ChoiceItem {
                id: "ask".into(),
                label: "ask  — confirm destructive tools".into(),
            },
            ChoiceItem {
                id: "always".into(),
                label: "always — auto-approve (deny still wins)".into(),
            },
        ],
        ChoiceKind::Theme => view
            .theme_names
            .iter()
            .map(|n| ChoiceItem {
                id: n.clone(),
                label: n.clone(),
            })
            .collect(),
        ChoiceKind::CrewPreset | ChoiceKind::Resume | ChoiceKind::DeleteSession => Vec::new(),
        ChoiceKind::Trust => vec![
            ChoiceItem {
                id: "yes".into(),
                label: "yes — trust this project's .ryter/".into(),
            },
            ChoiceItem {
                id: "no".into(),
                label: "no — stay untrusted".into(),
            },
        ],
        ChoiceKind::Sandbox => vec![
            ChoiceItem {
                id: "off".into(),
                label: "off".into(),
            },
            ChoiceItem {
                id: "workspace".into(),
                label: "workspace".into(),
            },
            ChoiceItem {
                id: "read-only".into(),
                label: "read-only".into(),
            },
        ],
        ChoiceKind::Phase => vec![
            ChoiceItem {
                id: "plan".into(),
                label: "plan — planners".into(),
            },
            ChoiceItem {
                id: "architect".into(),
                label: "architect — architects".into(),
            },
            ChoiceItem {
                id: "build".into(),
                label: "build — builders + auditor".into(),
            },
            ChoiceItem {
                id: "audit".into(),
                label: "audit — extra review".into(),
            },
        ],
    }
}

/// Shipped model list when `GET /models` is unavailable.
pub fn fallback_models(kind: &str, default_model: &str) -> Vec<ryter_core::ModelInfo> {
    let mut v = Vec::new();
    if kind == "spacexai" || default_model.starts_with("grok-") {
        v.push(model_row("grok-4.6", 500_000, 2.0, 6.0));
        v.push(model_row("grok-4.5", 256_000, 2.0, 6.0));
        v.push(model_row("grok-4.3", 256_000, 1.25, 2.5));
        v.push(model_row("grok-build-0.1", 256_000, 1.0, 2.0));
    }
    if !default_model.is_empty() && !v.iter().any(|m| m.id == default_model) {
        v.insert(
            0,
            ryter_core::ModelInfo::named(
                default_model,
                Some(ryter_core::window_for(default_model)),
            ),
        );
    }
    v
}

fn model_row(id: &str, window: u64, input: f64, output: f64) -> ryter_core::ModelInfo {
    ryter_core::ModelInfo {
        id: id.into(),
        context_length: Some(window),
        input_per_million: Some(input),
        output_per_million: Some(output),
        connection: None,
    }
}

/// Filter slash commands by prefix (no leading slash).
pub fn filter_slash(filter: &str, extra: &[String]) -> Vec<String> {
    let f = filter.trim_start_matches('/').to_ascii_lowercase();
    let mut out: Vec<String> = slash_commands()
        .iter()
        .filter(|c| c.starts_with(&f) || f.is_empty())
        .map(|c| (*c).to_string())
        .collect();
    for e in extra {
        let el = e.to_ascii_lowercase();
        if (el.starts_with(&f) || f.is_empty()) && !out.iter().any(|x| x == e || x == &el) {
            out.push(e.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ryter_core::RoleModel;

    #[test]
    fn crew_roles_default_to_orchestrator_model() {
        let v = View::new(
            Phase::Build,
            "openrouter".into(),
            "anthropic/claude-sonnet-4.6".into(),
            "p".into(),
        );
        for role in CREW_ROLES {
            assert_eq!(
                crew_role_label(&v, role),
                "default (claude-sonnet-4.6)",
                "{role}"
            );
        }
    }

    #[test]
    fn crew_override_is_shown_until_it_matches_orchestrator() {
        let mut v = View::new(
            Phase::Build,
            "spacexai".into(),
            "grok-4.6".into(),
            "p".into(),
        );
        v.specialists.insert(
            "planner".into(),
            RoleModel {
                connection: Some("openrouter".into()),
                model: Some("anthropic/claude-sonnet-4.6".into()),
            },
        );
        v.specialists.insert(
            "builder".into(),
            RoleModel {
                connection: Some("spacexai".into()),
                model: Some("grok-4.6".into()),
            },
        );
        assert_eq!(
            crew_role_label(&v, "planner"),
            "claude-sonnet-4.6 · openrouter"
        );
        assert_eq!(crew_role_label(&v, "builder"), "default (grok-4.6)");
        assert_eq!(crew_role_label(&v, "auditor"), "default (grok-4.6)");
    }

    #[test]
    fn mcp_home_has_inbound_and_add() {
        let v = View::new(
            Phase::Build,
            "spacexai".into(),
            "grok-4.6".into(),
            "p".into(),
        );
        assert_eq!(v.mcp_home_len(), 2);
        assert!(v.mcp_client_snippet().contains("mcp"));
        assert!(v.mcp_client_snippet().contains("serve"));
    }

    #[test]
    fn skills_home_includes_add_rows() {
        let mut v = View::new(
            Phase::Build,
            "spacexai".into(),
            "grok-4.6".into(),
            "p".into(),
        );
        v.catalog.skills.push(ryter_core::Skill {
            name: "review".into(),
            description: "Review".into(),
            user_invocable: true,
            body: "look".into(),
            source: std::path::PathBuf::from("/tmp/SKILL.md"),
        });
        assert_eq!(v.skills_home_len(), 3);
        assert_eq!(v.hooks_home_len(), 1);
    }
}
