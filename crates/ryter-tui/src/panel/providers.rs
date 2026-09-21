//! `/provider` — connections list and add wizard (`R-POP-19..23`).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::text::Line;
use ryter_core::{ConnectionConfig, connection_template};

use super::{Body, Notice, Outcome, Panel, widgets};
use crate::action::Action;
use crate::chat::wrap;
use crate::theme::Theme;
use crate::view::View;

const KINDS: [&str; 4] = ["spacexai", "openrouter", "openai_compat", "anthropic"];

/// Add-connection wizard steps (`R-POP-22`).
#[derive(Debug, Clone, PartialEq)]
enum Step {
    Name,
    Kind {
        name: String,
        idx: usize,
    },
    BaseUrl {
        name: String,
        kind: String,
    },
    EnvKey {
        name: String,
        kind: String,
        base_url: String,
    },
    Model {
        name: String,
        kind: String,
        base_url: String,
        env_key: String,
    },
    Review {
        name: String,
        conn: ConnectionConfig,
    },
}

impl Step {
    fn number(&self) -> usize {
        match self {
            Step::Name => 1,
            Step::Kind { .. } => 2,
            Step::BaseUrl { .. } => 3,
            Step::EnvKey { .. } => 4,
            Step::Model { .. } => 5,
            Step::Review { .. } => 6,
        }
    }

    fn title(&self) -> &'static str {
        match self {
            Step::Name => "connection name",
            Step::Kind { .. } => "kind",
            Step::BaseUrl { .. } => "base url",
            Step::EnvKey { .. } => "env var holding the key",
            Step::Model { .. } => "default model",
            Step::Review { .. } => "review",
        }
    }

    fn takes_text(&self) -> bool {
        !matches!(self, Step::Kind { .. } | Step::Review { .. })
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Mode {
    Browse,
    Add(Step),
    ConfirmRemove(String),
}

/// Connections panel.
#[derive(Debug, Clone)]
pub struct Providers {
    selected: usize,
    mode: Mode,
    error: Option<String>,
}

impl Providers {
    /// Start on the active connection.
    pub fn new(view: &View) -> Self {
        Self {
            selected: view
                .connections
                .iter()
                .position(|c| c.name == view.connection)
                .unwrap_or(0),
            mode: Mode::Browse,
            error: None,
        }
    }

    /// Rows: connections + `add` + `test`.
    fn len(&self, view: &View) -> usize {
        view.connections.len() + 2
    }

    fn step_prefill(step: &Step) -> String {
        match step {
            Step::BaseUrl { kind, .. } => connection_template(kind)
                .map(|c| c.base_url)
                .unwrap_or_default(),
            Step::EnvKey { kind, .. } => connection_template(kind)
                .ok()
                .and_then(|c| c.env_key)
                .unwrap_or_default(),
            Step::Model { kind, .. } => connection_template(kind)
                .ok()
                .and_then(|c| c.default_model)
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    fn enter_step(&mut self, view: &mut View, step: Step) {
        let pre = Self::step_prefill(&step);
        view.composer.set_text(&pre);
        self.error = None;
        self.mode = Mode::Add(step);
    }
}

impl Panel for Providers {
    fn kind(&self) -> &'static str {
        "providers"
    }

    fn title(&self, _view: &View) -> String {
        match &self.mode {
            Mode::Browse => "connections".into(),
            Mode::Add(_) => "add connection".into(),
            Mode::ConfirmRemove(n) => format!("remove {n}"),
        }
    }

    fn status(&self, view: &View) -> String {
        match &self.mode {
            Mode::Browse => format!("{}", view.connections.len()),
            Mode::Add(s) => format!("step {} of 6", s.number()),
            Mode::ConfirmRemove(_) => String::new(),
        }
    }

    fn legend(&self, _view: &View) -> String {
        match &self.mode {
            Mode::Browse => "enter use · k set key · t test · d remove · esc".into(),
            Mode::Add(Step::Kind { .. }) => "↑↓ choose · enter next · esc back".into(),
            Mode::Add(Step::Review { .. }) => {
                "enter write ~/.ryter/connections.toml · esc back".into()
            }
            Mode::Add(_) => "type · enter next · esc back".into(),
            Mode::ConfirmRemove(n) => format!("type `{n}` · enter remove · esc cancel"),
        }
    }

    fn input(&self, _view: &View) -> Option<String> {
        match &self.mode {
            Mode::Add(s) if s.takes_text() => Some(s.title().into()),
            Mode::ConfirmRemove(_) => Some("confirm".into()),
            _ => None,
        }
    }

    fn size(&self, view: &View) -> (u16, u16) {
        (70, (self.len(view) + 4).clamp(8, 18) as u16)
    }

    fn render(&self, view: &View, width: u16, _height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let mut lines: Vec<Line<'static>> = Vec::new();
        match &self.mode {
            Mode::Browse | Mode::ConfirmRemove(_) => {
                for (i, c) in view.connections.iter().enumerate() {
                    let active = c.name == view.connection;
                    let dot = if active { "●" } else { "○" };
                    let color = if c.has_key { theme.success } else { theme.warn };
                    let mut secondary = c.kind.clone();
                    if !c.model.is_empty() {
                        secondary.push_str(" · ");
                        secondary.push_str(crate::chat::short_model(&c.model));
                    }
                    let mut status = if c.has_key {
                        "key set".to_string()
                    } else {
                        "no key".into()
                    };
                    if let Some(t) = view.conn_tests.get(&c.name) {
                        status = t.clone();
                    }
                    lines.push(widgets::list_row(
                        dot,
                        &c.name,
                        &secondary,
                        &status,
                        i == self.selected,
                        w,
                        theme,
                        Some(color),
                    ));
                }
                lines.push(widgets::blank(theme));
                lines.push(widgets::list_row(
                    "+",
                    "add connection",
                    "",
                    "",
                    self.selected == view.connections.len(),
                    w,
                    theme,
                    Some(theme.accent),
                ));
                lines.push(widgets::list_row(
                    "⚙",
                    "test connection",
                    "live GET /models on the highlighted row",
                    "",
                    self.selected == view.connections.len() + 1,
                    w,
                    theme,
                    Some(theme.accent),
                ));
                if let Mode::ConfirmRemove(n) = &self.mode {
                    lines.push(widgets::blank(theme));
                    lines.push(widgets::colored(
                        &format!(
                            "remove `{n}` from ~/.ryter/connections.toml? its stored key is kept."
                        ),
                        theme.warn,
                        theme,
                    ));
                }
            }
            Mode::Add(step) => {
                lines.push(widgets::step_line(step.number(), 6, step.title(), theme));
                lines.push(widgets::blank(theme));
                match step {
                    Step::Name => {
                        lines.push(widgets::text("a short name, e.g. `work-openrouter`", theme));
                    }
                    Step::Kind { idx, .. } => {
                        for (i, k) in KINDS.iter().enumerate() {
                            lines.push(widgets::list_row(
                                "◇",
                                k,
                                kind_note(k),
                                "",
                                i == *idx,
                                w,
                                theme,
                                None,
                            ));
                        }
                    }
                    Step::BaseUrl { .. } => {
                        lines.push(widgets::text("the API root the client posts to", theme));
                    }
                    Step::EnvKey { .. } => {
                        lines.push(widgets::text(
                            "environment variable read at startup; or set a key later with k",
                            theme,
                        ));
                    }
                    Step::Model { .. } => {
                        lines.push(widgets::text(
                            "model id used when none is chosen in /models",
                            theme,
                        ));
                    }
                    Step::Review { name, conn } => {
                        lines.push(widgets::note(
                            "will write to ~/.ryter/connections.toml:",
                            theme,
                        ));
                        lines.push(widgets::blank(theme));
                        for row in [
                            format!("[{name}]"),
                            format!("kind = \"{}\"", conn.kind),
                            format!("base_url = \"{}\"", conn.base_url),
                            format!("api_backend = \"{}\"", conn.api_backend),
                            format!("env_key = \"{}\"", conn.env_key.clone().unwrap_or_default()),
                            format!(
                                "default_model = \"{}\"",
                                conn.default_model.clone().unwrap_or_default()
                            ),
                        ] {
                            lines.push(widgets::colored(
                                &wrap::truncate(&row, w.saturating_sub(2)),
                                theme.code_fg,
                                theme,
                            ));
                        }
                    }
                }
                if let Some(e) = &self.error {
                    lines.push(widgets::blank(theme));
                    lines.push(widgets::colored(&format!("✕ {e}"), theme.error, theme));
                }
            }
        }
        Body {
            lines,
            scroll: None,
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        match self.mode.clone() {
            Mode::Browse => {
                let n = self.len(view);
                match key.code {
                    KeyCode::Esc => Outcome::Close,
                    KeyCode::Up => {
                        self.selected = super::step(self.selected, -1, n);
                        Outcome::Stay
                    }
                    KeyCode::Down => {
                        self.selected = super::step(self.selected, 1, n);
                        Outcome::Stay
                    }
                    KeyCode::Enter => {
                        if self.selected == view.connections.len() {
                            self.enter_step(view, Step::Name);
                            return Outcome::Stay;
                        }
                        if self.selected == view.connections.len() + 1 {
                            return Outcome::Act(Action::TestConnection(view.connection.clone()));
                        }
                        match view.connections.get(self.selected) {
                            Some(c) => Outcome::CloseAct(Action::UseConnection(c.name.clone())),
                            None => Outcome::Stay,
                        }
                    }
                    KeyCode::Char('k') => match view.connections.get(self.selected) {
                        Some(c) => Outcome::CloseAct(Action::BeginSetKey(c.name.clone())),
                        None => Outcome::Stay,
                    },
                    KeyCode::Char('t') => match view.connections.get(self.selected) {
                        Some(c) => {
                            view.conn_tests.insert(c.name.clone(), "testing…".into());
                            Outcome::Act(Action::TestConnection(c.name.clone()))
                        }
                        None => Outcome::Stay,
                    },
                    KeyCode::Char('d') => match view.connections.get(self.selected) {
                        Some(c) => {
                            self.mode = Mode::ConfirmRemove(c.name.clone());
                            view.composer.clear();
                            Outcome::Stay
                        }
                        None => Outcome::Stay,
                    },
                    _ => Outcome::Stay,
                }
            }
            Mode::ConfirmRemove(name) => match key.code {
                KeyCode::Esc => {
                    self.mode = Mode::Browse;
                    view.composer.clear();
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    if view.composer.text().trim() == name {
                        view.composer.clear();
                        self.mode = Mode::Browse;
                        self.selected = 0;
                        Outcome::Act(Action::RemoveConnection(name))
                    } else {
                        Outcome::Stay
                    }
                }
                _ => {
                    super::edit_field(&mut view.composer, key);
                    Outcome::Stay
                }
            },
            Mode::Add(step) => self.wizard_key(key, view, step),
        }
    }

    fn on_notice(&mut self, n: &Notice, view: &mut View) {
        if let Notice::ConnTest { name, result } = n {
            let text = match result {
                Ok(ms) => format!("ok {ms}ms"),
                Err(e) => format!("failed {}", wrap::truncate(e, 24)),
            };
            view.conn_tests.insert(name.clone(), text);
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}

fn kind_note(kind: &str) -> &'static str {
    match kind {
        "spacexai" => "xAI Grok · messages API",
        "openrouter" => "OpenRouter · chat completions",
        "openai_compat" => "any OpenAI-compatible server",
        "anthropic" => "Anthropic · messages API",
        _ => "",
    }
}

impl Providers {
    fn wizard_key(&mut self, key: KeyEvent, view: &mut View, step: Step) -> Outcome {
        match key.code {
            KeyCode::Esc => {
                // Back one step (R-POP-57).
                let back = match step {
                    Step::Name => None,
                    Step::Kind { .. } => Some(Step::Name),
                    Step::BaseUrl { name, kind } => Some(Step::Kind {
                        idx: KINDS.iter().position(|k| *k == kind).unwrap_or(0),
                        name,
                    }),
                    Step::EnvKey { name, kind, .. } => Some(Step::BaseUrl { name, kind }),
                    Step::Model {
                        name,
                        kind,
                        base_url,
                        ..
                    } => Some(Step::EnvKey {
                        name,
                        kind,
                        base_url,
                    }),
                    Step::Review { name, conn } => Some(Step::Model {
                        name,
                        kind: conn.kind,
                        base_url: conn.base_url,
                        env_key: conn.env_key.unwrap_or_default(),
                    }),
                };
                match back {
                    Some(s) => self.enter_step(view, s),
                    None => {
                        self.mode = Mode::Browse;
                        view.composer.clear();
                    }
                }
                Outcome::Stay
            }
            KeyCode::Up | KeyCode::Down if matches!(step, Step::Kind { .. }) => {
                if let Mode::Add(Step::Kind { idx, .. }) = &mut self.mode {
                    let d = if key.code == KeyCode::Up { -1 } else { 1 };
                    *idx = super::step(*idx, d, KINDS.len());
                }
                Outcome::Stay
            }
            KeyCode::Enter => {
                let text = view.composer.text().trim().to_string();
                match step {
                    Step::Name => {
                        let name = text.to_ascii_lowercase();
                        if name.is_empty()
                            || !name
                                .chars()
                                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                        {
                            self.error = Some("name: letters, digits, - and _ only".into());
                            return Outcome::Stay;
                        }
                        if view.connections.iter().any(|c| c.name == name) {
                            self.error = Some(format!("`{name}` already exists"));
                            return Outcome::Stay;
                        }
                        self.enter_step(view, Step::Kind { name, idx: 0 });
                    }
                    Step::Kind { name, idx } => {
                        self.enter_step(
                            view,
                            Step::BaseUrl {
                                name,
                                kind: KINDS[idx].into(),
                            },
                        );
                    }
                    Step::BaseUrl { name, kind } => {
                        if !(text.starts_with("http://") || text.starts_with("https://")) {
                            self.error =
                                Some("base url must start with http:// or https://".into());
                            return Outcome::Stay;
                        }
                        self.enter_step(
                            view,
                            Step::EnvKey {
                                name,
                                kind,
                                base_url: text,
                            },
                        );
                    }
                    Step::EnvKey {
                        name,
                        kind,
                        base_url,
                    } => {
                        self.enter_step(
                            view,
                            Step::Model {
                                name,
                                kind,
                                base_url,
                                env_key: text,
                            },
                        );
                    }
                    Step::Model {
                        name,
                        kind,
                        base_url,
                        env_key,
                    } => {
                        let mut conn =
                            connection_template(&kind).unwrap_or_else(|_| ConnectionConfig {
                                kind: kind.clone(),
                                base_url: String::new(),
                                api_backend: "chat_completions".into(),
                                env_key: None,
                                api_key: None,
                                default_model: None,
                                http_referer: None,
                                x_title: None,
                            });
                        conn.base_url = base_url;
                        conn.env_key = (!env_key.is_empty()).then_some(env_key);
                        conn.default_model = (!text.is_empty()).then_some(text);
                        view.composer.clear();
                        self.error = None;
                        self.mode = Mode::Add(Step::Review { name, conn });
                    }
                    Step::Review { name, conn } => {
                        self.mode = Mode::Browse;
                        view.composer.clear();
                        return Outcome::Act(Action::AddConnection { name, conn });
                    }
                }
                Outcome::Stay
            }
            _ => {
                if step.takes_text() {
                    super::edit_field(&mut view.composer, key);
                    self.error = None;
                }
                Outcome::Stay
            }
        }
    }
}
