//! `/crew` — specialist routing (`R-POP-30..33`).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::text::Line;

use super::{Body, Outcome, Panel, PanelEnv, widgets};
use crate::action::Action;
use crate::theme::Theme;
use crate::view::{CREW_ROLES, View, crew_role_label};
use ryter_core::tiering::Tier;

/// Rows between the roles and `save preset`: the ready-made crews.
const TIERS: usize = Tier::ALL.len();

#[derive(Debug, Clone, PartialEq)]
enum Mode {
    Browse,
    /// Fetching every reachable catalog for a suggestion.
    Suggesting(Tier),
    /// A tiered crew to accept with `y`.
    Suggested(Tier, ryter_core::tiering::Tiering),
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

    /// Rows: roles, ready-made crews, `save preset`, presets.
    fn len(&self) -> usize {
        CREW_ROLES.len() + TIERS + 1 + self.presets.len()
    }

    /// Row of `save preset`.
    fn save_row(&self) -> usize {
        CREW_ROLES.len() + TIERS
    }
}

impl Crew {
    /// Read every catalog, then show `tier`'s crew for `y`.
    fn preview(&mut self, tier: Tier) -> Outcome {
        self.mode = Mode::Suggesting(tier);
        Outcome::Act(Action::ListCrewModels {
            role: String::new(),
        })
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
            Mode::Browse => "enter assign/apply · r reset · d delete · esc".into(),
            Mode::Suggesting(_) => "reading every model catalog you can reach… · esc".into(),
            Mode::Suggested(_, t) if t.auditor.is_some() => {
                "y apply (current crew kept as a preset) · esc".into()
            }
            Mode::Suggested(..) => "no independent auditor available · esc".into(),
            Mode::SaveName => "type a preset name · enter save · esc cancel".into(),
            Mode::ConfirmDelete(n) => format!("type `{n}` to delete · enter · esc"),
        }
    }

    fn input(&self, _view: &View) -> Option<String> {
        match &self.mode {
            Mode::Browse | Mode::Suggesting(_) | Mode::Suggested(..) => None,
            Mode::SaveName => Some("preset name".into()),
            Mode::ConfirmDelete(_) => Some("confirm".into()),
        }
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        (68, (self.len() + 10).min(28) as u16)
    }

    fn render(&self, view: &View, width: u16, _height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let mut lines: Vec<Line<'static>> = Vec::new();
        match &self.mode {
            Mode::Suggesting(tier) => {
                lines.push(widgets::note(tier.name(), theme));
                lines.push(widgets::note("  reading catalogs…", theme));
                return Body {
                    lines,
                    scroll: None,
                };
            }
            Mode::Suggested(tier, t) => {
                lines.push(widgets::note(
                    &format!("{} — {}", tier.name(), tier.cost()),
                    theme,
                ));
                for l in crate::chat::wrap::wrap_plain(tier.tagline(), w.saturating_sub(4)) {
                    lines.push(widgets::note(&format!("  {l}"), theme));
                }
                lines.push(widgets::blank(theme));
                lines.push(widgets::list_row(
                    "●",
                    "lead",
                    crate::chat::short_model(&view.model),
                    "unchanged",
                    false,
                    w,
                    theme,
                    None,
                ));
                for (role, pick) in [
                    ("builder", &t.builder),
                    ("auditor", &t.auditor),
                    ("architect", &t.architect),
                ] {
                    let label = pick
                        .as_ref()
                        .map(|p| p.label())
                        .unwrap_or_else(|| "(no suitable model)".into());
                    lines.push(widgets::list_row(
                        "●",
                        role,
                        &label,
                        "",
                        false,
                        w,
                        theme,
                        Some(theme.role(role)),
                    ));
                }
                lines.push(widgets::blank(theme));
                for n in &t.notes {
                    for l in crate::chat::wrap::wrap_plain(n, w.saturating_sub(4)) {
                        lines.push(widgets::note(&format!("  {l}"), theme));
                    }
                }
                return Body {
                    lines,
                    scroll: None,
                };
            }
            _ => {}
        }
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
        lines.push(widgets::note(
            "ready-made crews · picked from the models you can reach",
            theme,
        ));
        for (i, tier) in Tier::ALL.iter().enumerate() {
            lines.push(widgets::list_row(
                "▸",
                tier.name(),
                tier.cost(),
                "enter preview",
                self.selected == CREW_ROLES.len() + i,
                w,
                theme,
                Some(theme.accent),
            ));
        }
        lines.push(widgets::blank(theme));
        lines.push(widgets::note("presets", theme));
        lines.push(widgets::list_row(
            "+",
            "save preset",
            "current assignment → ~/.ryter/crews/<name>.toml",
            "",
            self.selected == self.save_row(),
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
                self.selected == self.save_row() + 1 + i,
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
            Mode::Suggesting(_) => {
                if key.code == KeyCode::Esc {
                    self.mode = Mode::Browse;
                }
                return Outcome::Stay;
            }
            Mode::Suggested(_, t) => {
                return match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') if t.auditor.is_some() => {
                        self.mode = Mode::Browse;
                        Outcome::Act(Action::ApplyCrewTiering(t.as_specialists()))
                    }
                    KeyCode::Esc | KeyCode::Char('n') => {
                        self.mode = Mode::Browse;
                        Outcome::Stay
                    }
                    _ => Outcome::Stay,
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
                if let Some(&tier) = self
                    .selected
                    .checked_sub(CREW_ROLES.len())
                    .and_then(|i| Tier::ALL.get(i))
                {
                    return self.preview(tier);
                }
                if self.selected == self.save_row() {
                    self.mode = Mode::SaveName;
                    view.composer.clear();
                    return Outcome::Stay;
                }
                let i = self.selected - self.save_row() - 1;
                match self.presets.get(i) {
                    Some(p) => Outcome::Act(Action::LoadCrewPreset(p.clone())),
                    None => Outcome::Stay,
                }
            }
            // The balanced crew, as before the tiers existed.
            KeyCode::Char('s') => self.preview(Tier::Schooner),
            KeyCode::Char('r') if self.selected < CREW_ROLES.len() => {
                Outcome::Act(Action::ResetCrewRole(CREW_ROLES[self.selected].to_string()))
            }
            KeyCode::Char('d') if self.selected > self.save_row() => {
                let i = self.selected - self.save_row() - 1;
                if let Some(p) = self.presets.get(i) {
                    self.mode = Mode::ConfirmDelete(p.clone());
                    view.composer.clear();
                }
                Outcome::Stay
            }
            _ => Outcome::Stay,
        }
    }

    fn on_notice(&mut self, n: &super::Notice, view: &mut View) {
        match n {
            super::Notice::PresetsChanged(list) => {
                self.presets = list.clone();
                self.selected = self.selected.min(self.len().saturating_sub(1));
            }
            super::Notice::Models(models) if matches!(self.mode, Mode::Suggesting(_)) => {
                let Mode::Suggesting(tier) = self.mode else {
                    return;
                };
                let local = view
                    .connections
                    .iter()
                    .filter(|c| c.kind == "local")
                    .map(|c| c.name.clone())
                    .collect();
                self.mode = Mode::Suggested(
                    tier,
                    ryter_core::tiering::suggest_tier(
                        tier,
                        &view.connection,
                        &view.model,
                        models,
                        &local,
                    ),
                );
            }
            _ => {}
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
