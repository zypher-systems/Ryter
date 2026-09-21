//! `/hooks` — lifecycle hooks (`R-POP-56`, `R-POP-57`, `R-POP-61`).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::text::Line;
use ryter_core::{HookConfig, HookEvent};

use super::{Body, Outcome, Panel, widgets};
use crate::action::Action;
use crate::chat::wrap;
use crate::theme::Theme;
use crate::view::View;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Pane {
    Home,
    /// Event is a Select (R-POP-61).
    Event {
        idx: usize,
    },
    Target {
        event: String,
    },
    Matcher {
        event: String,
        target: String,
    },
    Review {
        hook: HookConfig,
    },
    ConfirmRemove(usize),
}

/// Hooks panel.
#[derive(Debug, Clone)]
pub struct Hooks {
    pane: Pane,
    selected: usize,
    error: Option<String>,
}

impl Default for Hooks {
    fn default() -> Self {
        Self {
            pane: Pane::Home,
            selected: 0,
            error: None,
        }
    }
}

fn describe_event(e: HookEvent) -> &'static str {
    match e {
        HookEvent::PreToolUse => "before a tool runs · exit 2 / 4xx denies",
        HookEvent::PostToolUse => "after a tool returns",
        HookEvent::SessionStart => "when the agent starts a session",
        HookEvent::Handoff => "on every phase handoff",
    }
}

fn target_of(h: &HookConfig) -> String {
    h.command
        .clone()
        .or_else(|| h.url.clone())
        .unwrap_or_default()
}

fn step_of(p: &Pane) -> Option<(usize, &'static str)> {
    match p {
        Pane::Event { .. } => Some((1, "event")),
        Pane::Target { .. } => Some((2, "command or url")),
        Pane::Matcher { .. } => Some((3, "tool matcher (optional)")),
        Pane::Review { .. } => Some((4, "review")),
        _ => None,
    }
}

impl Hooks {
    fn go(&mut self, view: &mut View, pane: Pane) {
        view.composer.clear();
        self.error = None;
        self.pane = pane;
    }
}

impl Panel for Hooks {
    fn kind(&self) -> &'static str {
        "hooks"
    }

    fn title(&self, _view: &View) -> String {
        match &self.pane {
            Pane::Home | Pane::ConfirmRemove(_) => "hooks".into(),
            _ => "hooks · add".into(),
        }
    }

    fn status(&self, view: &View) -> String {
        match step_of(&self.pane) {
            Some((s, _)) => format!("step {s} of 4"),
            None => format!("{}", view.hooks.len()),
        }
    }

    fn legend(&self, _view: &View) -> String {
        match &self.pane {
            Pane::Home => "enter details · a add · d remove · esc".into(),
            Pane::Event { .. } => "↑↓ choose · enter next · esc back".into(),
            Pane::Review { .. } => "enter write ~/.ryter/hooks.toml · esc back".into(),
            Pane::ConfirmRemove(_) => "type `remove` · enter · esc".into(),
            _ => "type · enter next · esc back".into(),
        }
    }

    fn input(&self, _view: &View) -> Option<String> {
        match &self.pane {
            Pane::Target { .. } => Some("command or url".into()),
            Pane::Matcher { .. } => Some("matcher".into()),
            Pane::ConfirmRemove(_) => Some("confirm".into()),
            _ => None,
        }
    }

    fn size(&self, view: &View) -> (u16, u16) {
        (78, (view.hooks.len() + 6).clamp(8, 20) as u16)
    }

    fn render(&self, view: &View, width: u16, _height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let mut lines: Vec<Line<'static>> = Vec::new();
        match &self.pane {
            Pane::Home | Pane::ConfirmRemove(_) => {
                if view.hooks.is_empty() {
                    lines.push(widgets::note("no hooks configured", theme));
                }
                let rows: Vec<Vec<String>> = view
                    .hooks
                    .iter()
                    .map(|h| {
                        vec![
                            h.event.clone(),
                            h.matcher.clone().unwrap_or_else(|| "*".into()),
                            if h.url.is_some() {
                                "url".into()
                            } else {
                                "command".into()
                            },
                            target_of(h),
                        ]
                    })
                    .collect();
                if !rows.is_empty() {
                    lines.extend(widgets::table(
                        &["event", "matcher", "kind", "target"],
                        &rows,
                        &[
                            widgets::Al::L,
                            widgets::Al::L,
                            widgets::Al::L,
                            widgets::Al::L,
                        ],
                        Some(self.selected.min(view.hooks.len().saturating_sub(1))),
                        w,
                        theme,
                    ));
                }
                lines.push(widgets::blank(theme));
                lines.push(widgets::list_row(
                    "+",
                    "add hook",
                    "a add · appended to ~/.ryter/hooks.toml",
                    "",
                    self.selected == view.hooks.len(),
                    w,
                    theme,
                    Some(theme.accent),
                ));
                if let Pane::ConfirmRemove(i) = &self.pane {
                    if let Some(h) = view.hooks.get(*i) {
                        lines.push(widgets::blank(theme));
                        lines.push(widgets::colored(
                            &format!("remove {} → {}?", h.event, target_of(h)),
                            theme.warn,
                            theme,
                        ));
                    }
                }
            }
            Pane::Event { idx } => {
                lines.push(widgets::step_line(1, 4, "event", theme));
                lines.push(widgets::blank(theme));
                for (i, e) in HookEvent::all().iter().enumerate() {
                    lines.push(widgets::list_row(
                        "◇",
                        e.as_str(),
                        describe_event(*e),
                        "",
                        i == *idx,
                        w,
                        theme,
                        None,
                    ));
                }
            }
            Pane::Target { .. } => {
                lines.push(widgets::step_line(2, 4, "command or url", theme));
                lines.push(widgets::blank(theme));
                lines.push(widgets::text(
                    "shell command (receives JSON on stdin) or an http(s):// URL to POST to",
                    theme,
                ));
            }
            Pane::Matcher { .. } => {
                lines.push(widgets::step_line(3, 4, "tool matcher", theme));
                lines.push(widgets::blank(theme));
                lines.push(widgets::text(
                    "glob on the tool name for Pre/Post hooks, e.g. `bash*`; empty = all",
                    theme,
                ));
            }
            Pane::Review { hook } => {
                lines.push(widgets::step_line(4, 4, "review", theme));
                lines.push(widgets::blank(theme));
                lines.push(widgets::note("will append to ~/.ryter/hooks.toml:", theme));
                let mut rows = vec![
                    "[[hooks]]".to_string(),
                    format!("event = \"{}\"", hook.event),
                ];
                if let Some(c) = &hook.command {
                    rows.push(format!("command = \"{c}\""));
                }
                if let Some(u) = &hook.url {
                    rows.push(format!("url = \"{u}\""));
                }
                if let Some(m) = &hook.matcher {
                    rows.push(format!("matcher = \"{m}\""));
                }
                for r in rows {
                    lines.push(widgets::colored(
                        &wrap::truncate(&r, w.saturating_sub(2)),
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
        Body {
            lines,
            scroll: None,
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        match self.pane.clone() {
            Pane::Home => {
                let n = view.hooks.len() + 1;
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
                    KeyCode::Char('a') => {
                        self.go(view, Pane::Event { idx: 0 });
                        Outcome::Stay
                    }
                    KeyCode::Enter => {
                        if self.selected >= view.hooks.len() {
                            self.go(view, Pane::Event { idx: 0 });
                        } else if let Some(h) = view.hooks.get(self.selected) {
                            let mut line = h.event.clone();
                            if let Some(c) = &h.command {
                                line.push_str(&format!("  command={c}"));
                            }
                            if let Some(u) = &h.url {
                                line.push_str(&format!("  url={u}"));
                            }
                            if let Some(m) = &h.matcher {
                                line.push_str(&format!("  matcher={m}"));
                            }
                            view.system(line);
                        }
                        Outcome::Stay
                    }
                    KeyCode::Char('d') if self.selected < view.hooks.len() => {
                        self.go(view, Pane::ConfirmRemove(self.selected));
                        Outcome::Stay
                    }
                    _ => Outcome::Stay,
                }
            }
            Pane::ConfirmRemove(i) => match key.code {
                KeyCode::Esc => {
                    self.go(view, Pane::Home);
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    if view.composer.text().trim() == "remove" && i < view.hooks.len() {
                        view.hooks.remove(i);
                        self.go(view, Pane::Home);
                        self.selected = 0;
                        Outcome::Act(Action::SaveHooks)
                    } else {
                        Outcome::Stay
                    }
                }
                _ => {
                    super::edit_field(&mut view.composer, key);
                    Outcome::Stay
                }
            },
            Pane::Event { idx } => match key.code {
                KeyCode::Esc => {
                    self.go(view, Pane::Home);
                    Outcome::Stay
                }
                KeyCode::Up | KeyCode::Left => {
                    self.pane = Pane::Event {
                        idx: super::step(idx, -1, HookEvent::all().len()),
                    };
                    Outcome::Stay
                }
                KeyCode::Down | KeyCode::Right => {
                    self.pane = Pane::Event {
                        idx: super::step(idx, 1, HookEvent::all().len()),
                    };
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    let event = HookEvent::all()[idx].as_str().to_string();
                    self.go(view, Pane::Target { event });
                    Outcome::Stay
                }
                _ => Outcome::Stay,
            },
            Pane::Target { event } => match key.code {
                KeyCode::Esc => {
                    let idx = HookEvent::all()
                        .iter()
                        .position(|e| e.as_str() == event)
                        .unwrap_or(0);
                    self.go(view, Pane::Event { idx });
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    let target = view.composer.text().trim().to_string();
                    if target.is_empty() {
                        self.error = Some("command or url required".into());
                        return Outcome::Stay;
                    }
                    self.go(view, Pane::Matcher { event, target });
                    Outcome::Stay
                }
                _ => {
                    super::edit_field(&mut view.composer, key);
                    self.error = None;
                    Outcome::Stay
                }
            },
            Pane::Matcher { event, target } => match key.code {
                KeyCode::Esc => {
                    self.go(view, Pane::Target { event });
                    view.composer.set_text(&target);
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    let m = view.composer.text().trim().to_string();
                    let matcher = (!m.is_empty()).then_some(m);
                    let (command, url) =
                        if target.starts_with("http://") || target.starts_with("https://") {
                            (None, Some(target))
                        } else {
                            (Some(target), None)
                        };
                    self.go(
                        view,
                        Pane::Review {
                            hook: HookConfig {
                                event,
                                command,
                                url,
                                matcher,
                            },
                        },
                    );
                    Outcome::Stay
                }
                _ => {
                    super::edit_field(&mut view.composer, key);
                    Outcome::Stay
                }
            },
            Pane::Review { hook } => match key.code {
                KeyCode::Esc => {
                    let target = target_of(&hook);
                    let matcher = hook.matcher.clone().unwrap_or_default();
                    self.go(
                        view,
                        Pane::Matcher {
                            event: hook.event,
                            target,
                        },
                    );
                    view.composer.set_text(&matcher);
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    view.hooks.push(hook);
                    self.go(view, Pane::Home);
                    self.selected = view.hooks.len().saturating_sub(1);
                    Outcome::Act(Action::SaveHooks)
                }
                _ => Outcome::Stay,
            },
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}
