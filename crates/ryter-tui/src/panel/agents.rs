//! `/agents` — running specialists (`R-POP-34..37`).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::text::Line;
use ryter_core::format_usd;

use super::{Body, Outcome, Panel, widgets};
use crate::action::Action;
use crate::chat::fmt_elapsed;
use crate::theme::Theme;
use crate::view::View;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Confirm {
    None,
    /// `y/n` for one specialist.
    One(String),
    /// Typed `kill all`.
    All,
}

/// Running specialists panel.
#[derive(Debug, Clone)]
pub struct Agents {
    selected: usize,
    confirm: Confirm,
}

impl Default for Agents {
    fn default() -> Self {
        Self {
            selected: 0,
            confirm: Confirm::None,
        }
    }
}

impl Panel for Agents {
    fn kind(&self) -> &'static str {
        "agents"
    }

    fn title(&self, _view: &View) -> String {
        "running specialists".into()
    }

    fn status(&self, view: &View) -> String {
        format!("{}/{}", view.crew.len(), view.max_crew)
    }

    fn legend(&self, _view: &View) -> String {
        match &self.confirm {
            Confirm::None => "↑↓ move · enter/k kill · K kill all · esc".into(),
            Confirm::One(_) => "kill this specialist? y / n".into(),
            Confirm::All => "type `kill all` then enter · esc cancels".into(),
        }
    }

    fn input(&self, _view: &View) -> Option<String> {
        matches!(self.confirm, Confirm::All).then(|| "confirm".to_string())
    }

    fn size(&self, view: &View) -> (u16, u16) {
        (72, (view.crew.len() * 2 + 2).clamp(4, 20) as u16)
    }

    fn render(&self, view: &View, width: u16, _height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let mut lines: Vec<Line<'static>> = Vec::new();
        if view.crew.is_empty() {
            lines.push(widgets::note("no specialists running", theme));
            lines.push(widgets::note(
                "the lead starts architects, builders, and auditors as your requests need them",
                theme,
            ));
        }
        for (i, c) in view.crew.iter().enumerate() {
            let elapsed = fmt_elapsed(view.now_ms.saturating_sub(c.started_ms) / 1000);
            let spend = c.spend.map(|s| format_usd(Some(s))).unwrap_or_default();
            let status = format!("{}  {elapsed}  {spend}", c.status)
                .trim()
                .to_string();
            lines.push(widgets::list_row(
                "●",
                &c.role,
                &c.label,
                &status,
                i == self.selected,
                w,
                theme,
                Some(theme.role(&c.role)),
            ));
        }
        if let Confirm::One(id) = &self.confirm {
            if let Some(c) = view.crew.iter().find(|c| c.id == *id) {
                lines.push(widgets::blank(theme));
                lines.push(widgets::colored(
                    &format!("kill {} · {}?  y / n", c.role, c.label),
                    theme.warn,
                    theme,
                ));
            }
        }
        if matches!(self.confirm, Confirm::All) {
            lines.push(widgets::blank(theme));
            lines.push(widgets::colored(
                &format!("type `kill all` to stop {} specialists", view.crew.len()),
                theme.warn,
                theme,
            ));
        }
        Body {
            lines,
            scroll: None,
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        let n = view.crew.len();
        match &self.confirm {
            Confirm::One(id) => {
                let id = id.clone();
                return match key.code {
                    KeyCode::Char('y' | 'Y') => {
                        self.confirm = Confirm::None;
                        Outcome::Act(Action::KillAgent(id))
                    }
                    KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                        self.confirm = Confirm::None;
                        Outcome::Stay
                    }
                    _ => Outcome::Stay,
                };
            }
            Confirm::All => {
                return match key.code {
                    KeyCode::Esc => {
                        self.confirm = Confirm::None;
                        view.composer.clear();
                        Outcome::Stay
                    }
                    KeyCode::Enter => {
                        if view.composer.text().trim() == "kill all" {
                            self.confirm = Confirm::None;
                            view.composer.clear();
                            Outcome::Act(Action::KillAllAgents)
                        } else {
                            Outcome::Stay
                        }
                    }
                    _ => {
                        super::edit_field(&mut view.composer, key);
                        Outcome::Stay
                    }
                };
            }
            Confirm::None => {}
        }
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
            KeyCode::Enter | KeyCode::Char('k') if n > 0 => {
                if let Some(c) = view.crew.get(self.selected) {
                    self.confirm = Confirm::One(c.id.clone());
                }
                Outcome::Stay
            }
            KeyCode::Char('K') if n > 0 => {
                self.confirm = Confirm::All;
                view.composer.clear();
                Outcome::Stay
            }
            _ => Outcome::Stay,
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}
