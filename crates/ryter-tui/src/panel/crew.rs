//! `/crew` — specialist routing (`R-POP-30..33`).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::text::Line;

use super::{Body, Outcome, Panel, PanelEnv, widgets};
use crate::action::Action;
use crate::theme::Theme;
use crate::view::{CREW_ROLES, View, crew_role_label};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    Browse,
    /// Composer holds the preset name.
    SaveName,
    /// Typed preset-name confirmation for delete.
    ConfirmDelete(String),
}

/// Crew panel.
#[derive(Debug, Clone)]
pub struct Crew {
    selected: usize,
    presets: Vec<String>,
    mode: Mode,
}

impl Crew {
    /// Load presets from disk.
    pub fn new(env: &PanelEnv) -> Self {
        Self {
            selected: 0,
            presets: ryter_core::list_crew_presets(&env.home),
            mode: Mode::Browse,
        }
    }

    /// Rows: roles, `save preset`, presets.
    fn len(&self) -> usize {
        CREW_ROLES.len() + 1 + self.presets.len()
    }
}

impl Panel for Crew {
    fn kind(&self) -> &'static str {
        "crew"
    }

    fn title(&self, _view: &View) -> String {
        "crew".into()
    }

    fn status(&self, view: &View) -> String {
        format!("max concurrent: {}", view.max_crew)
    }

    fn legend(&self, _view: &View) -> String {
        match &self.mode {
            Mode::Browse => "enter assign/load · r reset · d delete preset · esc".into(),
            Mode::SaveName => "type a preset name · enter save · esc cancel".into(),
            Mode::ConfirmDelete(n) => format!("type `{n}` to delete · enter · esc"),
        }
    }

    fn input(&self, _view: &View) -> Option<String> {
        match &self.mode {
            Mode::Browse => None,
            Mode::SaveName => Some("preset name".into()),
            Mode::ConfirmDelete(_) => Some("confirm".into()),
        }
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        (68, (self.len() + 6).min(22) as u16)
    }

    fn render(&self, view: &View, width: u16, _height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let mut lines: Vec<Line<'static>> = Vec::new();
        lines.push(widgets::note("roles", theme));
        for (i, role) in CREW_ROLES.iter().enumerate() {
            let label = crew_role_label(view, role);
            let status = if view.specialists.get(*role).is_some_and(|r| r.is_override()) {
                "override"
            } else {
                ""
            };
            lines.push(widgets::list_row(
                "●",
                role,
                &label,
                status,
                i == self.selected,
                w,
                theme,
                Some(theme.role(role)),
            ));
        }
        lines.push(widgets::blank(theme));
        lines.push(widgets::note("presets", theme));
        lines.push(widgets::list_row(
            "+",
            "save preset",
            "current assignment → ~/.ryter/crews/<name>.toml",
            "",
            self.selected == CREW_ROLES.len(),
            w,
            theme,
            Some(theme.accent),
        ));
        for (i, p) in self.presets.iter().enumerate() {
            lines.push(widgets::list_row(
                "◇",
                p,
                "",
                "enter load · d delete",
                self.selected == CREW_ROLES.len() + 1 + i,
                w,
                theme,
                None,
            ));
        }
        if self.presets.is_empty() {
            lines.push(widgets::note("  no presets yet", theme));
        }
        lines.push(widgets::blank(theme));
        lines.push(widgets::note(
            &format!(
                "max concurrent specialists: {} · change in /settings",
                view.max_crew
            ),
            theme,
        ));
        Body {
            lines,
            scroll: None,
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        match self.mode.clone() {
            Mode::SaveName => {
                return match key.code {
                    KeyCode::Esc => {
                        self.mode = Mode::Browse;
                        view.composer.clear();
                        Outcome::Stay
                    }
                    KeyCode::Enter => {
                        let name = sanitize(view.composer.text());
                        if name.is_empty() {
                            return Outcome::Stay;
                        }
                        view.composer.clear();
                        self.mode = Mode::Browse;
                        Outcome::Act(Action::SaveCrewPreset(name))
                    }
                    _ => {
                        super::edit_field(&mut view.composer, key);
                        Outcome::Stay
                    }
                };
            }
            Mode::ConfirmDelete(name) => {
                return match key.code {
                    KeyCode::Esc => {
                        self.mode = Mode::Browse;
                        view.composer.clear();
                        Outcome::Stay
                    }
                    KeyCode::Enter => {
                        if view.composer.text().trim() == name {
                            view.composer.clear();
                            self.mode = Mode::Browse;
                            Outcome::Act(Action::DeleteCrewPreset(name))
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
            Mode::Browse => {}
        }
        let n = self.len();
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
                if self.selected < CREW_ROLES.len() {
                    let role = CREW_ROLES[self.selected].to_string();
                    let child = super::models::Models::new(view, Some(role.clone()));
                    return Outcome::PushAct(Box::new(child), Action::ListCrewModels { role });
                }
                if self.selected == CREW_ROLES.len() {
                    self.mode = Mode::SaveName;
                    view.composer.clear();
                    return Outcome::Stay;
                }
                let i = self.selected - CREW_ROLES.len() - 1;
                match self.presets.get(i) {
                    Some(p) => Outcome::Act(Action::LoadCrewPreset(p.clone())),
                    None => Outcome::Stay,
                }
            }
            KeyCode::Char('r') if self.selected < CREW_ROLES.len() => {
                Outcome::Act(Action::ResetCrewRole(CREW_ROLES[self.selected].to_string()))
            }
            KeyCode::Char('d') if self.selected > CREW_ROLES.len() => {
                let i = self.selected - CREW_ROLES.len() - 1;
                if let Some(p) = self.presets.get(i) {
                    self.mode = Mode::ConfirmDelete(p.clone());
                    view.composer.clear();
                }
                Outcome::Stay
            }
            _ => Outcome::Stay,
        }
    }

    fn on_notice(&mut self, n: &super::Notice, _view: &mut View) {
        if let super::Notice::PresetsChanged(list) = n {
            self.presets = list.clone();
            self.selected = self.selected.min(self.len().saturating_sub(1));
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
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
        .to_ascii_lowercase()
}
