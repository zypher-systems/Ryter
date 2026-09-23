//! Drawable UI state (no terminal, no agent). `R-STATE-01/02`.

pub mod history;
pub mod scroll;

use std::cell::RefCell;
use std::collections::BTreeMap;

use ryter_core::{Phase, SlashCatalog, UiConfig, format_usd};

use crate::action::Action;
use crate::activity::{Activity, Mode as ActivityMode};
use crate::chat::cache::RenderCache;
use crate::chat::{Message, MessageKind, MessageMeta, OffsetTimestamp, SystemLevel, ToolStatus};
use crate::composer::Composer;
use crate::palette::Palette;
use crate::panel::PanelStack;
use history::History;
use scroll::ChatScroll;

/// One configured connection for `/provider` and the info panel.
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

/// A task queue row for the info panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoRow {
    /// Title.
    pub title: String,
    /// `pending` / `running` / `done` / `blocked`.
    pub status: String,
}

/// A running specialist row.
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
    /// `now_ms` when it started.
    pub started_ms: u64,
}

/// Everything the draw path needs.
#[derive(Debug, Clone)]
pub struct View {
    /// Current phase.
    pub phase: Phase,
    /// Who the user is talking to: a hat in solo mode (build, plan,
    /// review), or the crew's lead (`Orchestrator`) in crew mode.
    pub mode: ryter_core::Role,
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
    /// Transcript, oldest first (`R-CHAT-02`).
    pub messages: Vec<Message>,
    /// Next message id.
    pub next_id: u64,
    /// Current turn number.
    pub turn: u64,
    /// Scroll state.
    pub scroll: ChatScroll,
    /// Reasoning per turn (display only, `R-ACT-12/13`).
    pub reasoning: BTreeMap<u64, String>,
    /// Activity strip.
    pub activity: Activity,
    /// Composer.
    pub composer: Composer,
    /// Command palette when open.
    pub palette: Option<Palette>,
    /// Popouts.
    pub panels: PanelStack,
    /// Prompt history.
    pub history: History,
    /// Info panel visible (session state).
    pub panel_visible: bool,
    /// Message queued while busy (`R-COMP-16`).
    pub queued_prompt: Option<String>,
    /// Resolved speaker name (`R-CHAT-11`).
    pub username: String,
    /// Local UTC offset in seconds.
    pub tz_offset: i32,
    /// Parallel specialists.
    pub crew: Vec<CrewRow>,
    /// Model is running.
    pub busy: bool,
    /// Cancel requested.
    pub cancelling: bool,
    /// Project path shown in the header (`~/workspace/ryter`).
    pub cwd: String,
    /// Current git branch, if any.
    pub git_branch: Option<String>,
    /// `ask` / `always`.
    pub perm_mode: String,
    /// Skills + user slash commands.
    pub catalog: SlashCatalog,
    /// Editable hook rows (persisted to `hooks.toml`).
    pub hooks: Vec<ryter_core::HookConfig>,
    /// Active theme name.
    pub theme_name: String,
    /// Built-ins plus `~/.ryter/themes/*.toml`.
    pub theme_names: Vec<String>,
    /// Render-cache generation (bumped on theme change).
    pub theme_generation: u32,
    /// Known connections.
    pub connections: Vec<ConnRow>,
    /// Whether the active connection has a key.
    pub has_key: bool,
    /// Auditor gate.
    pub auditor_on: bool,
    /// Task list.
    pub todos: Vec<TodoRow>,
    /// The chat's folded lookup row and what it has counted, while lookups
    /// keep coming.
    pub lookups: Option<(u64, crate::chat::toolview::Lookups)>,
    /// What this turn has done, for its closing line.
    pub tally: crate::chat::toolview::TurnTally,
    /// Tool call id → (tool, target), for its result.
    pub tool_calls: BTreeMap<String, (String, String)>,
    /// Last known token count.
    pub ctx_tokens: Option<u64>,
    /// Window used for the context bar.
    pub ctx_window: Option<u64>,
    /// Transcript message count from the last `Context` event.
    pub ctx_messages: Option<usize>,
    /// Per-contributor token estimates.
    pub ctx_breakdown: Vec<(String, u64)>,
    /// `$2/M input / $6/M output` for the active model.
    pub price_label: String,
    /// Last known USD / million input.
    pub price_in: Option<f64>,
    /// Last known USD / million output.
    pub price_out: Option<f64>,
    /// Prices from the model lists providers returned (`/models`, the crew
    /// builder): the fallback when the built-in price book doesn't know a
    /// model, which is most of OpenRouter's catalog.
    pub catalog_rates: BTreeMap<String, (f64, f64)>,
    /// What this project (its git repository) has cost across sessions.
    /// Read from disk at startup and when `/spend` opens; live spend is added
    /// as it happens.
    pub project_spend: Option<ryter_core::project::ProjectSpend>,
    /// The newest undo checkpoint: `/changes` compares the last turn to it.
    pub last_checkpoint: Option<String>,
    /// The latest test run's result (`✓ 13 passed`), for a commit receipt.
    pub last_tests: Option<String>,
    /// The model edited files after that run.
    pub tests_stale: bool,
    /// The user's reasoning level per model (`low` / `medium` / `high` /
    /// `default`); a model not listed is "auto".
    pub model_reasoning: BTreeMap<String, String>,
    /// Live `[specialists.*]` assignment (edited by `/crew`).
    pub specialists: BTreeMap<String, ryter_core::RoleModel>,
    /// Unix socket path if inbound MCP is listening.
    pub mcp_listen: Option<String>,
    /// Current session id.
    pub session_id: String,
    /// Session title.
    pub session_title: String,
    /// Inbound MCP enabled.
    pub mcp_inbound: bool,
    /// Configured TCP bind (`127.0.0.1:8765`).
    pub mcp_bind: Option<String>,
    /// Bound TCP address, if the TUI started a listener.
    pub mcp_tcp_listen: Option<String>,
    /// Outbound MCP servers.
    pub mcp_servers: BTreeMap<String, ryter_core::McpServerConfig>,
    /// Outbound server status text (`connected`, last error).
    pub mcp_status: BTreeMap<String, String>,
    /// Named inbound bearer tokens (full secret; panel masks unless revealed).
    pub mcp_tokens: Vec<(String, String)>,
    /// Token name to show in the clear.
    pub mcp_reveal: Option<String>,
    /// Known USD by role name.
    pub spend_by_role: BTreeMap<String, f64>,
    /// Known USD by connection.
    pub spend_by_conn: BTreeMap<String, f64>,
    /// `(calls, input, output, cached, usd)` by role.
    pub spend_rows_role: BTreeMap<String, SpendRow>,
    /// Same by connection.
    pub spend_rows_conn: BTreeMap<String, SpendRow>,
    /// Calls with unknown price (`R-POP-45`).
    pub unpriced_calls: u32,
    /// Session budget cap (0 = none).
    pub budget_usd: f64,
    /// The cap to restore when the budget is switched back on.
    pub budget_last: f64,
    /// `[spend] task_budget_usd`: one task's cap, budget or not.
    pub task_budget_usd: f64,
    /// Warn threshold.
    pub warn_usd: f64,
    /// `[subagents] max`.
    pub max_crew: u32,
    /// Sandbox profile name.
    pub sandbox_profile: String,
    /// `[features] web`.
    pub web: bool,
    /// `[ui]` settings in effect.
    pub ui: UiConfig,
    /// Recently run command names (`R-PAL-12`).
    pub recent_commands: Vec<String>,
    /// Connectivity test status per connection.
    pub conn_tests: BTreeMap<String, String>,
    /// Last export path reported by `/spend`.
    pub last_export: Option<String>,
    /// Monotonic clock in ms, advanced by the loop.
    pub now_ms: u64,
    /// `Ctrl+C` armed for quit until this time (`R-COMP-14`).
    pub quit_armed_until: Option<u64>,
    /// Render cache (derived; clones start empty).
    pub cache: RefCell<RenderCache>,
    /// Startup warnings not yet shown.
    pub pending_warnings: Vec<String>,
}

/// Aggregated spend row for `/spend`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SpendRow {
    /// Model calls.
    pub calls: u32,
    /// Input tokens.
    pub input: u64,
    /// Output tokens.
    pub output: u64,
    /// Cached tokens.
    pub cached: u64,
    /// Known USD.
    pub usd: f64,
    /// Any unpriced call.
    pub unpriced: bool,
}

impl View {
    /// Empty session chrome for tests / startup.
    /// In crew mode: messages go to the lead, and the crew does the work.
    pub fn crew_mode(&self) -> bool {
        self.mode == ryter_core::Role::Orchestrator
    }

    /// `build`, `plan`, `review`, or `crew`.
    pub fn mode_label(&self) -> &'static str {
        if self.crew_mode() {
            "crew"
        } else {
            self.mode.as_str()
        }
    }

    /// `model`'s reasoning choice for people: `auto`, `low`, … .
    pub fn reasoning_label(&self, model: &str) -> &'static str {
        ryter_core::config::reasoning_label(self.model_reasoning.get(model).map(String::as_str))
    }

    /// The level `model` actually gets in `role`, after auto picks one.
    pub fn reasoning_effective(&self, role: ryter_core::Role, model: &str) -> String {
        ryter_core::config::effort_for(None, Some(&self.model_reasoning), role, model)
            .unwrap_or_else(|| "model's own".into())
    }

    pub fn new(phase: Phase, connection: String, model: String, cwd: String) -> Self {
        Self {
            phase,
            mode: ryter_core::Role::SoloBuild,
            connection,
            model,
            spend: None,
            spend_unknown: false,
            ctx_pct: None,
            messages: Vec::new(),
            next_id: 1,
            turn: 0,
            scroll: ChatScroll::new(),
            reasoning: BTreeMap::new(),
            activity: Activity::new(ActivityMode::Collapsed),
            composer: Composer::new(),
            palette: None,
            panels: PanelStack::default(),
            history: History::default(),
            panel_visible: true,
            queued_prompt: None,
            username: "you".into(),
            tz_offset: 0,
            crew: Vec::new(),
            busy: false,
            cancelling: false,
            cwd,
            git_branch: None,
            perm_mode: "ask".into(),
            catalog: SlashCatalog::default(),
            hooks: Vec::new(),
            theme_name: "dark".into(),
            theme_names: crate::theme::BUILTIN_THEMES
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            theme_generation: 0,
            connections: Vec::new(),
            has_key: false,
            auditor_on: true,
            todos: Vec::new(),
            lookups: None,
            tally: Default::default(),
            tool_calls: BTreeMap::new(),
            ctx_tokens: None,
            ctx_window: None,
            ctx_messages: None,
            ctx_breakdown: Vec::new(),
            price_label: String::new(),
            price_in: None,
            catalog_rates: BTreeMap::new(),
            project_spend: None,
            last_checkpoint: None,
            last_tests: None,
            tests_stale: false,
            model_reasoning: BTreeMap::new(),
            price_out: None,
            specialists: BTreeMap::new(),
            mcp_listen: None,
            session_id: String::new(),
            session_title: String::new(),
            mcp_inbound: true,
            mcp_bind: None,
            mcp_tcp_listen: None,
            mcp_servers: BTreeMap::new(),
            mcp_status: BTreeMap::new(),
            mcp_tokens: Vec::new(),
            mcp_reveal: None,
            spend_by_role: BTreeMap::new(),
            spend_by_conn: BTreeMap::new(),
            spend_rows_role: BTreeMap::new(),
            spend_rows_conn: BTreeMap::new(),
            unpriced_calls: 0,
            budget_usd: 0.0,
            budget_last: 5.0,
            task_budget_usd: 3.0,
            warn_usd: 1.0,
            max_crew: 4,
            sandbox_profile: "off".into(),
            web: false,
            ui: UiConfig::default(),
            recent_commands: Vec::new(),
            conn_tests: BTreeMap::new(),
            last_export: None,
            now_ms: 0,
            quit_armed_until: None,
            cache: RefCell::new(RenderCache::new()),
            pending_warnings: Vec::new(),
        }
    }

    // -- clock ----------------------------------------------------------------

    /// Current wall-clock timestamp at the configured offset.
    pub fn now(&self) -> OffsetTimestamp {
        OffsetTimestamp::now(self.tz_offset)
    }

    /// Advance the monotonic clock (spinner, elapsed, quit arming).
    pub fn tick(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
        self.activity.tick(now_ms);
        if self.quit_armed_until.is_some_and(|t| now_ms > t) {
            self.quit_armed_until = None;
        }
    }

    // -- messages -------------------------------------------------------------

    /// Append a message and return it for further edits.
    pub fn push(&mut self, kind: MessageKind, body: impl Into<String>) -> &mut Message {
        let id = self.next_id;
        self.next_id += 1;
        let at = self.now();
        self.messages.push(Message {
            id,
            turn: self.turn,
            kind,
            body: body.into(),
            at,
            meta: MessageMeta::default(),
            rev: 0,
        });
        self.messages.last_mut().expect("just pushed")
    }

    /// Info system message.
    pub fn system(&mut self, text: impl Into<String>) {
        self.push(
            MessageKind::System {
                level: SystemLevel::Info,
            },
            text,
        );
    }

    /// Warning system message.
    pub fn warn(&mut self, text: impl Into<String>) {
        self.push(
            MessageKind::System {
                level: SystemLevel::Warn,
            },
            text,
        );
    }

    /// Error system message.
    pub fn error(&mut self, text: impl Into<String>) {
        self.push(
            MessageKind::System {
                level: SystemLevel::Error,
            },
            text,
        );
    }

    /// Streamed assistant delta (`R-CHAT-03`).
    pub fn on_token(&mut self, text: &str) {
        let model = self.model.clone();
        match self.messages.last_mut() {
            Some(m) if matches!(m.kind, MessageKind::Assistant { .. }) => m.append(text),
            _ => {
                self.push(MessageKind::Assistant { model }, text);
            }
        }
        if self.activity.busy() {
            self.activity.verb = crate::activity::Verb::Writing;
            self.activity.tokens += (text.len() as u64).div_ceil(4);
        }
    }

    /// Start a turn from user text: push the message, anchor, mark busy.
    pub fn submit_user(&mut self, shown: String, expanded: String) -> Action {
        if self.busy {
            self.queued_prompt = Some(expanded);
            self.system("queued · sent when the current turn completes");
            return Action::None;
        }
        self.turn += 1;
        let turn = self.turn;
        self.history.push(&shown);
        self.push(MessageKind::User, shown);
        self.scroll.on_submit(turn);
        self.busy = true;
        self.cancelling = false;
        self.activity.start(turn, self.now_ms);
        Action::Submit(expanded)
    }

    /// Anything in the transcript worth confirming before `/new`.
    pub fn has_content(&self) -> bool {
        self.messages
            .iter()
            .any(|m| matches!(m.kind, MessageKind::User | MessageKind::Assistant { .. }))
    }

    /// Reset transcript state for `/new` / `/resume` (`R-CHAT-04`).
    pub fn reset_transcript(&mut self) {
        self.messages.clear();
        self.reasoning.clear();
        self.history.clear();
        self.turn = 0;
        self.scroll = ChatScroll::new();
        self.activity = Activity::new(self.activity.mode);
        self.queued_prompt = None;
        self.busy = false;
        self.cancelling = false;
        self.cache.borrow_mut().clear();
    }

    /// Mark the last running tool row with its result.
    pub fn finish_tool(&mut self, id: &str, is_error: bool, duration_ms: Option<u64>) {
        if let Some(m) = self.messages.iter_mut().rev().find(|m| {
            matches!(m.kind, MessageKind::Tool { .. }) && m.meta.tool_id.as_deref() == Some(id)
        }) {
            if let MessageKind::Tool { status, .. } = &mut m.kind {
                *status = if is_error {
                    ToolStatus::Error
                } else {
                    ToolStatus::Ok
                };
            }
            m.meta.duration_ms = duration_ms;
            m.touch();
        }
    }

    /// Top row of the anchored turn's first message, if rendered.
    pub fn anchor_top(&self) -> Option<usize> {
        let t = self.scroll.anchor_turn?;
        self.scroll
            .turn_tops
            .borrow()
            .iter()
            .find(|(turn, _)| *turn == t)
            .map(|(_, top)| *top)
    }

    /// The anchored user message.
    pub fn anchor_message(&self) -> Option<&Message> {
        let t = self.scroll.anchor_turn?;
        self.messages
            .iter()
            .find(|m| m.turn == t && matches!(m.kind, MessageKind::User))
    }

    // -- commands -------------------------------------------------------------

    /// Record a palette run for the recency boost.
    pub fn note_recent(&mut self, name: &str) {
        self.recent_commands.retain(|n| n != name);
        self.recent_commands.insert(0, name.to_string());
        self.recent_commands.truncate(10);
    }

    /// Spend label for the header / cards (`$?.??` when any turn was unpriced).
    pub fn spend_label(&self) -> String {
        if self.spend_unknown && self.spend.is_none() {
            format_usd(None)
        } else {
            format_usd(self.spend)
        }
    }

    /// Context window in use for gauges.
    pub fn ctx_window_or_default(&self) -> u64 {
        self.ctx_window
            .unwrap_or_else(|| ryter_core::window_for(&self.model))
    }

    /// Context fraction 0..1.
    pub fn ctx_frac(&self) -> f64 {
        let w = self.ctx_window_or_default().max(1) as f64;
        (self.ctx_tokens.unwrap_or(0) as f64 / w).clamp(0.0, 1.0)
    }
}

/// What a role is called on screen. The code and logs say `orchestrator`; the
/// user talks to the lead.
pub fn role_label(role: &str) -> &str {
    match role {
        "orchestrator" => "lead",
        other => other,
    }
}

/// Specialist kinds shown in `/crew`.
pub const CREW_ROLES: &[&str] = &["architect", "builder", "auditor"];

/// Label under a crew role (`default (grok-4.6)` or a short model id).
pub fn crew_role_label(view: &View, role: &str) -> String {
    let orch = crate::chat::short_model(&view.model).to_string();
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
        created: None,
        tools: None,
    }
}

/// Resolve the speaker name (`R-CHAT-11`): `[ui] username` → git → `$USER` → `you`.
pub fn resolve_username(
    configured: &str,
    git_name: Option<String>,
    env_user: Option<String>,
) -> String {
    let c = configured.trim();
    if !c.is_empty() {
        return c.to_string();
    }
    if let Some(g) = git_name
        .map(|g| g.trim().to_string())
        .filter(|g| !g.is_empty())
    {
        return g;
    }
    if let Some(u) = env_user
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
    {
        return u;
    }
    "you".into()
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
            "architect".into(),
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
            crew_role_label(&v, "architect"),
            "claude-sonnet-4.6 · openrouter"
        );
        assert_eq!(crew_role_label(&v, "builder"), "default (grok-4.6)");
        assert_eq!(crew_role_label(&v, "auditor"), "default (grok-4.6)");
    }

    #[test]
    fn username_resolution_order() {
        assert_eq!(
            resolve_username("Dusty", Some("Git".into()), Some("u".into())),
            "Dusty"
        );
        assert_eq!(
            resolve_username("", Some("Git Name".into()), Some("u".into())),
            "Git Name"
        );
        assert_eq!(
            resolve_username("", Some("  ".into()), Some("u".into())),
            "u"
        );
        assert_eq!(resolve_username("", None, None), "you");
    }

    #[test]
    fn submit_streams_into_one_assistant_message() {
        let mut v = View::new(Phase::Build, "x".into(), "m".into(), "p".into());
        let a = v.submit_user("hi".into(), "hi".into());
        assert_eq!(a, Action::Submit("hi".into()));
        assert_eq!(v.turn, 1);
        assert!(v.busy);
        assert_eq!(v.scroll.anchor_turn, Some(1));
        v.on_token("hel");
        v.on_token("lo");
        assert_eq!(v.messages.len(), 2);
        assert_eq!(v.messages[1].body, "hello");
        assert_eq!(v.messages[1].rev, 1);
        // Submitting while busy queues (R-COMP-16).
        let a = v.submit_user("next".into(), "next".into());
        assert_eq!(a, Action::None);
        assert_eq!(v.queued_prompt.as_deref(), Some("next"));
    }
}
