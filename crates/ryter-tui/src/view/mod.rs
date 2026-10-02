//! Drawable UI state (no terminal, no agent). `R-STATE-01/02`.

pub mod history;
pub mod scroll;

use std::cell::RefCell;
use std::collections::BTreeMap;

use ryter_core::{SlashCatalog, UiConfig, format_usd};

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

/// A conversation's chat while the other one is on screen: its messages,
/// where it was scrolled to, and what was drawn of it.
#[derive(Debug, Clone)]
pub struct ParkedChat {
    messages: Vec<Message>,
    turn: u64,
    scroll: ChatScroll,
    reasoning: BTreeMap<u64, String>,
    lookups: Option<(u64, crate::chat::toolview::Lookups)>,
    cache: RefCell<RenderCache>,
}

impl Default for ParkedChat {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            turn: 0,
            scroll: ChatScroll::new(),
            reasoning: BTreeMap::new(),
            lookups: None,
            cache: RefCell::new(RenderCache::new()),
        }
    }
}

/// Everything the draw path needs.
#[derive(Debug, Clone)]
pub struct View {
    /// The hat the next message goes out in: build, plan, review, or test.
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
    /// Model is running.
    pub busy: bool,
    /// Cancel requested.
    pub cancelling: bool,
    /// `Ctrl+O`: every edit's diff shown whole instead of folded.
    pub diffs_expanded: bool,
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
    /// Where the model list stands (`AgentEvent::ModelsNote`): from the
    /// cache and refreshing, or why the fresh one didn't come.
    pub models_note: Option<String>,
    /// The workbench, when open (`^T`).
    pub workbench: Option<crate::workbench::Workbench>,
    /// The chat's folded lookup row and what it has counted, while lookups
    /// keep coming.
    pub lookups: Option<(u64, crate::chat::toolview::Lookups)>,
    /// What this turn has done, for its closing line.
    pub tally: crate::chat::toolview::TurnTally,
    /// Session spend when the running turn started, for its closing line.
    pub turn_spend_from: Option<f64>,
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
    /// Prices from the model lists providers returned (`/models`): the
    /// fallback when the built-in price book doesn't know a
    /// model, which is most of OpenRouter's catalog.
    pub catalog_rates: BTreeMap<String, (f64, f64)>,
    /// What this project (its git repository) has cost across sessions.
    /// Read from disk at startup and when `/spend` opens; live spend is added
    /// as it happens.
    pub project_spend: Option<ryter_core::project::ProjectSpend>,
    /// Where the project cost is counted, when that isn't the folder Ryter
    /// runs in: the repository around it (`~/workspace`). The rail names it,
    /// so a total that carries over from other folders explains itself.
    pub project_root: Option<String>,
    /// The newest undo checkpoint: `/changes` compares the last turn to it.
    pub last_checkpoint: Option<String>,
    /// The latest test run's result (`✓ 13 passed`), for a commit receipt.
    pub last_tests: Option<String>,
    /// The model edited files after that run.
    pub tests_stale: bool,
    /// The review hat's last review: the files it read (as a git tree), its
    /// model, and its verdict. For a commit receipt.
    pub last_review: Option<(Option<String>, String, Option<bool>)>,
    /// The user's reasoning level per model (`low` / `medium` / `high` /
    /// `default`); a model not listed is "auto".
    pub model_reasoning: BTreeMap<String, String>,
    /// Each hat's own model, where it has one (set in `/models`).
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
    /// `[spend] review_usd`: the most one review may spend (0 = no limit).
    pub review_usd: f64,
    /// Warn threshold.
    pub warn_usd: f64,
    /// Sandbox profile name.
    pub sandbox_profile: String,
    /// `[update] mode`: what Ryter does about a newer release at launch.
    pub update_mode: ryter_core::config::UpdateMode,
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
    /// The conversation on screen: the one the plan, build and review hats
    /// share, or the tester's own.
    pub shown: ryter_core::Thread,
    /// The conversation the running turn is part of. What the turn says
    /// goes there, whichever is on screen.
    pub turn_thread: ryter_core::Thread,
    /// The conversation that isn't on screen.
    pub parked: ParkedChat,
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
    /// The seats that can have a model of their own, after the first (the
    /// model the rest follow): the hats.
    pub fn seat_roles(&self) -> &'static [&'static str] {
        HAT_ROLES
    }

    /// The model the next message goes to: this hat's own where it has
    /// one, otherwise the one every hat uses.
    pub fn hat_model(&self) -> &str {
        self.specialists
            .get(self.mode.as_str())
            .filter(|r| r.is_override())
            .and_then(|r| r.model.as_deref())
            .unwrap_or(&self.model)
    }

    /// `build`, `plan`, `review`, or `test`.
    pub fn mode_label(&self) -> &'static str {
        self.mode.as_str()
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

    pub fn new(connection: String, model: String, cwd: String) -> Self {
        Self {
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
            busy: false,
            cancelling: false,
            diffs_expanded: false,
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
            workbench: None,
            models_note: None,
            lookups: None,
            tally: Default::default(),
            turn_spend_from: None,
            tool_calls: BTreeMap::new(),
            ctx_tokens: None,
            ctx_window: None,
            ctx_messages: None,
            ctx_breakdown: Vec::new(),
            price_label: String::new(),
            price_in: None,
            catalog_rates: BTreeMap::new(),
            project_spend: None,
            project_root: None,
            last_checkpoint: None,
            last_tests: None,
            tests_stale: false,
            last_review: None,
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
            review_usd: 0.0,
            warn_usd: 1.0,
            sandbox_profile: "off".into(),
            update_mode: ryter_core::config::UpdateMode::default(),
            web: false,
            // Tests and snapshots start on the classic layout; the app takes
            // the user's `[ui] layout` (ledger by default) from config.
            ui: UiConfig {
                layout: "classic".into(),
                ..UiConfig::default()
            },
            recent_commands: Vec::new(),
            conn_tests: BTreeMap::new(),
            last_export: None,
            now_ms: 0,
            quit_armed_until: None,
            cache: RefCell::new(RenderCache::new()),
            pending_warnings: Vec::new(),
            shown: ryter_core::Thread::Main,
            turn_thread: ryter_core::Thread::Main,
            parked: ParkedChat::default(),
        }
    }

    // -- the two conversations --------------------------------------------------

    /// Put `thread`'s chat on screen. The other keeps its messages and its
    /// place, and comes back as it was left.
    pub fn show(&mut self, thread: ryter_core::Thread) {
        if thread == self.shown {
            return;
        }
        std::mem::swap(&mut self.messages, &mut self.parked.messages);
        std::mem::swap(&mut self.turn, &mut self.parked.turn);
        std::mem::swap(&mut self.scroll, &mut self.parked.scroll);
        std::mem::swap(&mut self.reasoning, &mut self.parked.reasoning);
        std::mem::swap(&mut self.lookups, &mut self.parked.lookups);
        std::mem::swap(&mut self.cache, &mut self.parked.cache);
        self.shown = thread;
    }

    /// Every message of the session, in both conversations: the shared one
    /// first.
    pub fn session_messages(&self) -> impl Iterator<Item = &Message> {
        let (main, test) = if self.shown == ryter_core::Thread::Main {
            (&self.messages, &self.parked.messages)
        } else {
            (&self.parked.messages, &self.messages)
        };
        main.iter().chain(test.iter())
    }

    /// Whether the tester's own conversation has anything in it.
    pub fn test_thread_started(&self) -> bool {
        if self.shown == ryter_core::Thread::Test {
            !self.messages.is_empty()
        } else {
            !self.parked.messages.is_empty()
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
        // The model that is answering: this hat's.
        let model = self.hat_model().to_string();
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
        // The message is typed into the conversation on screen, and the
        // turn it starts is part of that one.
        self.turn_thread = self.shown;
        self.history.push(&shown);
        self.push(MessageKind::User, shown);
        self.scroll.on_submit(turn);
        self.busy = true;
        self.cancelling = false;
        self.activity.start(turn, self.now_ms);
        Action::Submit(expanded)
    }

    /// What the model said last in this turn, before it asked: its last
    /// paragraph, on one line, for the "why" on a permission prompt.
    pub fn last_words(&self) -> Option<String> {
        let m = self
            .messages
            .iter()
            .rev()
            .take_while(|m| m.turn == self.turn)
            .find(|m| {
                matches!(m.kind, MessageKind::Assistant { .. }) && !m.body.trim().is_empty()
            })?;
        let para = m
            .body
            .rsplit("\n\n")
            .map(str::trim)
            .find(|p| !p.is_empty())?;
        let flat = para.split_whitespace().collect::<Vec<_>>().join(" ");
        Some(crate::chat::wrap::truncate(&flat, 160))
    }

    /// Anything in the transcript worth confirming before `/new`.
    pub fn has_content(&self) -> bool {
        self.messages
            .iter()
            .any(|m| matches!(m.kind, MessageKind::User | MessageKind::Assistant { .. }))
    }

    /// Reset transcript state for `/new` / `/resume` (`R-CHAT-04`).
    pub fn reset_transcript(&mut self) {
        // Both conversations: a new session has neither.
        self.show(ryter_core::Thread::Main);
        self.parked = ParkedChat::default();
        self.turn_thread = ryter_core::Thread::Main;
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

    /// Whether the ledger folds any finished turn right now.
    pub fn has_folded_turns(&self) -> bool {
        let latest = self.messages.iter().map(|m| m.turn).max().unwrap_or(0);
        self.messages.iter().any(|m| {
            m.turn != latest
                && matches!(
                    m.kind,
                    MessageKind::System {
                        level: crate::chat::SystemLevel::Receipt
                    }
                )
        })
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

/// What a row of spend is called on screen. A total saved while crew mode
/// existed names its roles, and its lead was `orchestrator` in the logs.
pub fn role_label(role: &str) -> &str {
    match role {
        "orchestrator" => "lead",
        other => other,
    }
}

/// The hats that can have a model of their own, in the order the work goes.
pub const HAT_ROLES: &[&str] = &["plan", "build", "review", "test"];

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
        let mut v = View::new("x".into(), "m".into(), "p".into());
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
