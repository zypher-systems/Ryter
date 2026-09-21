//! `/mcp` — outbound servers and inbound listener (`R-POP-56..59`).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::text::Line;
use ryter_core::{AgentEvent, McpServerConfig};

use super::{Body, Outcome, Panel, widgets};
use crate::action::Action;
use crate::chat::wrap;
use crate::theme::Theme;
use crate::view::View;

/// Add-server wizard (`R-POP-57`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    Name,
    Command {
        name: String,
    },
    Args {
        name: String,
        command: String,
    },
    Review {
        name: String,
        command: String,
        args: Vec<String>,
    },
}

impl Step {
    fn number(&self) -> usize {
        match self {
            Step::Name => 1,
            Step::Command { .. } => 2,
            Step::Args { .. } => 3,
            Step::Review { .. } => 4,
        }
    }

    fn title(&self) -> &'static str {
        match self {
            Step::Name => "server name",
            Step::Command { .. } => "command",
            Step::Args { .. } => "arguments",
            Step::Review { .. } => "review",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Pane {
    Home,
    Inbound,
    Add(Step),
    ConfirmRemove(String),
}

/// MCP panel.
#[derive(Debug, Clone)]
pub struct Mcp {
    pane: Pane,
    selected: usize,
    error: Option<String>,
}

impl Default for Mcp {
    fn default() -> Self {
        Self {
            pane: Pane::Home,
            selected: 0,
            error: None,
        }
    }
}

/// `ryter mcp serve` — stdio command clients spawn.
pub const STDIO_CMD: &str = "ryter mcp serve";

/// JSON snippet for a client `mcp.json`.
pub const CLIENT_SNIPPET: &str =
    r#"{"mcpServers":{"ryter":{"command":"ryter","args":["mcp","serve"]}}}"#;

/// Unix URI for attach, if listening.
pub fn unix_uri(view: &View) -> Option<String> {
    view.mcp_listen.as_ref().map(|p| {
        if p.starts_with('/') {
            format!("unix://{p}")
        } else {
            format!("unix:///{p}")
        }
    })
}

fn sanitize(name: &str) -> String {
    name.trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

impl Mcp {
    fn server_names(view: &View) -> Vec<String> {
        view.mcp_servers.keys().cloned().collect()
    }

    /// Home rows: inbound + servers + add.
    fn home_len(view: &View) -> usize {
        1 + view.mcp_servers.len() + 1
    }

    /// Inbound rows.
    const INBOUND_LEN: usize = 6;

    fn enter_step(&mut self, view: &mut View, step: Step) {
        view.composer.clear();
        self.error = None;
        self.pane = Pane::Add(step);
    }
}

impl Panel for Mcp {
    fn kind(&self) -> &'static str {
        "mcp"
    }

    fn title(&self, _view: &View) -> String {
        match &self.pane {
            Pane::Home => "mcp".into(),
            Pane::Inbound => "mcp · inbound".into(),
            Pane::Add(_) => "mcp · add server".into(),
            Pane::ConfirmRemove(n) => format!("mcp · remove {n}"),
        }
    }

    fn status(&self, view: &View) -> String {
        match &self.pane {
            Pane::Home => {
                let up = view
                    .mcp_status
                    .values()
                    .filter(|s| s.starts_with("connected"))
                    .count();
                format!("{up}/{} up", view.mcp_servers.len())
            }
            Pane::Add(s) => format!("step {} of 4", s.number()),
            _ => String::new(),
        }
    }

    fn legend(&self, _view: &View) -> String {
        match &self.pane {
            Pane::Home => {
                "enter open/toggle · r reconnect · d remove · esc  · changes apply immediately"
                    .into()
            }
            Pane::Inbound => "enter toggle/copy · v reveal token · esc back".into(),
            Pane::Add(Step::Review { .. }) => "enter write ~/.ryter/mcp.toml · esc back".into(),
            Pane::Add(_) => "type · enter next · esc back".into(),
            Pane::ConfirmRemove(n) => format!("type `{n}` · enter · esc"),
        }
    }

    fn input(&self, _view: &View) -> Option<String> {
        match &self.pane {
            Pane::Add(s) if !matches!(s, Step::Review { .. }) => Some(s.title().into()),
            Pane::ConfirmRemove(_) => Some("confirm".into()),
            _ => None,
        }
    }

    fn size(&self, view: &View) -> (u16, u16) {
        (
            74,
            (Self::home_len(view).max(Self::INBOUND_LEN) + 6).clamp(10, 22) as u16,
        )
    }

    fn render(&self, view: &View, width: u16, _height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let mut lines: Vec<Line<'static>> = Vec::new();
        match &self.pane {
            Pane::Home | Pane::ConfirmRemove(_) => {
                let inbound_status = match (&view.mcp_listen, &view.mcp_tcp_listen) {
                    (Some(_), Some(t)) => format!("unix + tcp {t}"),
                    (Some(_), None) => "unix socket".into(),
                    (None, Some(t)) => format!("tcp {t}"),
                    (None, None) if view.mcp_inbound => "enabled · restart to bind".into(),
                    (None, None) => "off".into(),
                };
                lines.push(widgets::list_row(
                    if view.mcp_listen.is_some() || view.mcp_tcp_listen.is_some() {
                        "●"
                    } else {
                        "○"
                    },
                    "inbound",
                    "let other agents drive this session",
                    &inbound_status,
                    self.selected == 0,
                    w,
                    theme,
                    Some(if view.mcp_listen.is_some() {
                        theme.success
                    } else {
                        theme.dim
                    }),
                ));
                lines.push(widgets::blank(theme));
                lines.push(widgets::note("outbound servers", theme));
                for (i, (name, cfg)) in view.mcp_servers.iter().enumerate() {
                    let status = view.mcp_status.get(name).cloned().unwrap_or_else(|| {
                        if cfg.enabled {
                            "not connected".into()
                        } else {
                            "disabled".into()
                        }
                    });
                    let color = if status.starts_with("connected") {
                        theme.success
                    } else if status.starts_with("error") {
                        theme.error
                    } else {
                        theme.dim
                    };
                    let cmd = format!("{} {}", cfg.command, cfg.args.join(" "))
                        .trim()
                        .to_string();
                    lines.push(widgets::list_row(
                        if cfg.enabled { "●" } else { "○" },
                        name,
                        &cmd,
                        &wrap::truncate(&status, 28),
                        self.selected == 1 + i,
                        w,
                        theme,
                        Some(color),
                    ));
                }
                if view.mcp_servers.is_empty() {
                    lines.push(widgets::note("  none configured", theme));
                }
                lines.push(widgets::list_row(
                    "+",
                    "add server",
                    "stdio command → ~/.ryter/mcp.toml",
                    "",
                    self.selected == 1 + view.mcp_servers.len(),
                    w,
                    theme,
                    Some(theme.accent),
                ));
                if let Pane::ConfirmRemove(n) = &self.pane {
                    lines.push(widgets::blank(theme));
                    lines.push(widgets::colored(
                        &format!("remove server `{n}`?"),
                        theme.warn,
                        theme,
                    ));
                }
            }
            Pane::Inbound => {
                let rows: [(&str, String, String); Self::INBOUND_LEN] = [
                    (
                        "inbound",
                        "accept prompts from other agents".into(),
                        if view.mcp_inbound {
                            "[ on ]".into()
                        } else {
                            "[ off ]".into()
                        },
                    ),
                    ("stdio command", STDIO_CMD.into(), "copy to chat".into()),
                    (
                        "unix socket",
                        unix_uri(view).unwrap_or_else(|| "not listening".into()),
                        String::new(),
                    ),
                    (
                        "tcp bind",
                        view.mcp_bind.clone().unwrap_or_else(|| "off".into()),
                        view.mcp_tcp_listen
                            .clone()
                            .map(|t| format!("listening {t}"))
                            .unwrap_or_default(),
                    ),
                    (
                        "bearer token",
                        match view.mcp_tokens.iter().find(|(n, _)| n == "default") {
                            Some((n, t)) if view.mcp_reveal.as_deref() == Some(n.as_str()) => {
                                t.clone()
                            }
                            Some(_) => "••••••••  (v to reveal)".into(),
                            None => "none · enter creates one".into(),
                        },
                        String::new(),
                    ),
                    (
                        "client snippet",
                        "mcp.json for Cursor / Claude".into(),
                        "copy to chat".into(),
                    ),
                ];
                for (i, (label, value, status)) in rows.iter().enumerate() {
                    lines.push(widgets::list_row(
                        "·",
                        label,
                        value,
                        status,
                        self.selected == i,
                        w,
                        theme,
                        None,
                    ));
                }
            }
            Pane::Add(step) => {
                lines.push(widgets::step_line(step.number(), 4, step.title(), theme));
                lines.push(widgets::blank(theme));
                match step {
                    Step::Name => lines.push(widgets::text("letters, digits, - and _", theme)),
                    Step::Command { .. } => {
                        lines.push(widgets::text("executable to spawn, e.g. `npx`", theme))
                    }
                    Step::Args { .. } => lines.push(widgets::text(
                        "space-separated; leave empty for none",
                        theme,
                    )),
                    Step::Review {
                        name,
                        command,
                        args,
                    } => {
                        lines.push(widgets::note("will write to ~/.ryter/mcp.toml:", theme));
                        lines.push(widgets::blank(theme));
                        for row in [
                            format!("[servers.{name}]"),
                            format!("command = \"{command}\""),
                            format!(
                                "args = [{}]",
                                args.iter()
                                    .map(|a| format!("\"{a}\""))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                            "enabled = true".to_string(),
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
        match self.pane.clone() {
            Pane::Home => {
                let n = Self::home_len(view);
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
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        if self.selected == 0 {
                            self.pane = Pane::Inbound;
                            self.selected = 0;
                            return Outcome::Stay;
                        }
                        let names = Self::server_names(view);
                        if let Some(name) = names.get(self.selected - 1) {
                            if let Some(s) = view.mcp_servers.get_mut(name) {
                                s.enabled = !s.enabled;
                            }
                            return Outcome::Act(Action::SaveMcp);
                        }
                        self.enter_step(view, Step::Name);
                        Outcome::Stay
                    }
                    KeyCode::Char('r') => Outcome::Act(Action::SaveMcp),
                    KeyCode::Char('d') => {
                        let names = Self::server_names(view);
                        if self.selected >= 1 {
                            if let Some(name) = names.get(self.selected - 1) {
                                self.pane = Pane::ConfirmRemove(name.clone());
                                view.composer.clear();
                            }
                        }
                        Outcome::Stay
                    }
                    _ => Outcome::Stay,
                }
            }
            Pane::ConfirmRemove(name) => match key.code {
                KeyCode::Esc => {
                    self.pane = Pane::Home;
                    view.composer.clear();
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    if view.composer.text().trim() == name {
                        view.mcp_servers.remove(&name);
                        view.mcp_status.remove(&name);
                        view.composer.clear();
                        self.pane = Pane::Home;
                        self.selected = 0;
                        Outcome::Act(Action::SaveMcp)
                    } else {
                        Outcome::Stay
                    }
                }
                _ => {
                    super::edit_field(&mut view.composer, key);
                    Outcome::Stay
                }
            },
            Pane::Inbound => match key.code {
                KeyCode::Esc => {
                    self.pane = Pane::Home;
                    self.selected = 0;
                    view.mcp_reveal = None;
                    Outcome::Stay
                }
                KeyCode::Up => {
                    self.selected = super::step(self.selected, -1, Self::INBOUND_LEN);
                    Outcome::Stay
                }
                KeyCode::Down => {
                    self.selected = super::step(self.selected, 1, Self::INBOUND_LEN);
                    Outcome::Stay
                }
                KeyCode::Char('v') => {
                    view.mcp_reveal = if view.mcp_reveal.is_some() {
                        None
                    } else {
                        Some("default".into())
                    };
                    Outcome::Stay
                }
                KeyCode::Enter | KeyCode::Char(' ') => match self.selected {
                    0 => {
                        view.mcp_inbound = !view.mcp_inbound;
                        Outcome::Act(Action::SaveMcp)
                    }
                    1 => {
                        view.system(format!("mcp stdio: {STDIO_CMD}"));
                        Outcome::Stay
                    }
                    2 => {
                        let uri = unix_uri(view).unwrap_or_else(|| {
                            "unix socket not listening (restart with inbound on)".into()
                        });
                        view.system(uri);
                        Outcome::Stay
                    }
                    3 => {
                        if view.mcp_bind.is_some() {
                            view.mcp_bind = None;
                            Outcome::Act(Action::SaveMcp)
                        } else {
                            view.mcp_bind = Some("127.0.0.1:8765".into());
                            if view.mcp_tokens.is_empty() {
                                view.mcp_tokens
                                    .push(("default".into(), ryter_core::new_inbound_token()));
                                view.mcp_reveal = Some("default".into());
                            }
                            Outcome::Act(Action::McpListenTcp)
                        }
                    }
                    4 => {
                        let tok = ryter_core::new_inbound_token();
                        if let Some(row) = view.mcp_tokens.iter_mut().find(|(n, _)| n == "default")
                        {
                            row.1 = tok;
                        } else {
                            view.mcp_tokens.push(("default".into(), tok));
                        }
                        view.mcp_reveal = Some("default".into());
                        Outcome::Act(Action::SaveMcp)
                    }
                    _ => {
                        view.system(CLIENT_SNIPPET);
                        Outcome::Stay
                    }
                },
                _ => Outcome::Stay,
            },
            Pane::Add(step) => match key.code {
                KeyCode::Esc => {
                    let back = match step {
                        Step::Name => None,
                        Step::Command { .. } => Some(Step::Name),
                        Step::Args { name, .. } => Some(Step::Command { name }),
                        Step::Review { name, command, .. } => Some(Step::Args { name, command }),
                    };
                    match back {
                        Some(s) => self.enter_step(view, s),
                        None => {
                            self.pane = Pane::Home;
                            view.composer.clear();
                        }
                    }
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    let text = view.composer.text().trim().to_string();
                    match step {
                        Step::Name => {
                            let name = sanitize(&text);
                            if name.is_empty() {
                                self.error = Some("name required".into());
                                return Outcome::Stay;
                            }
                            if view.mcp_servers.contains_key(&name) {
                                self.error = Some(format!("`{name}` already exists"));
                                return Outcome::Stay;
                            }
                            self.enter_step(view, Step::Command { name });
                        }
                        Step::Command { name } => {
                            if text.is_empty() {
                                self.error = Some("command required".into());
                                return Outcome::Stay;
                            }
                            self.enter_step(
                                view,
                                Step::Args {
                                    name,
                                    command: text,
                                },
                            );
                        }
                        Step::Args { name, command } => {
                            let args = text.split_whitespace().map(str::to_string).collect();
                            view.composer.clear();
                            self.error = None;
                            self.pane = Pane::Add(Step::Review {
                                name,
                                command,
                                args,
                            });
                        }
                        Step::Review {
                            name,
                            command,
                            args,
                        } => {
                            view.mcp_servers.insert(
                                name,
                                McpServerConfig {
                                    command,
                                    args,
                                    enabled: true,
                                    env: std::collections::BTreeMap::new(),
                                },
                            );
                            self.pane = Pane::Home;
                            self.selected = 0;
                            return Outcome::Act(Action::SaveMcp);
                        }
                    }
                    Outcome::Stay
                }
                _ => {
                    if !matches!(step, Step::Review { .. }) {
                        super::edit_field(&mut view.composer, key);
                        self.error = None;
                    }
                    Outcome::Stay
                }
            },
        }
    }

    fn on_event(&mut self, ev: &AgentEvent, view: &mut View) {
        if let AgentEvent::McpStatus { servers } = ev {
            view.mcp_status = servers.iter().cloned().collect();
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}
