//! `/phase` — pick a handoff target (`R-POP-62`, `R-POP-63`).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::text::Line;
use ryter_core::Phase;

use super::{Body, Outcome, Panel, widgets};
use crate::action::Action;
use crate::theme::Theme;
use crate::view::View;

/// All phases in display order.
pub const PHASES: [Phase; 4] = [Phase::Plan, Phase::Architect, Phase::Build, Phase::Audit];

/// One-line description per phase.
pub fn describe(p: Phase) -> &'static str {
    match p {
        Phase::Plan => "planner breaks the goal into a task list and open questions",
        Phase::Architect => "architect decides structure, interfaces, and trade-offs",
        Phase::Build => "builders implement tasks in parallel; auditor gates merges",
        Phase::Audit => "auditor reviews the work and reports findings",
    }
}

/// Phase picker.
#[derive(Debug, Clone)]
pub struct PhasePicker {
    selected: usize,
}

impl PhasePicker {
    /// Start on the current phase.
    pub fn new(view: &View) -> Self {
        Self {
            selected: PHASES.iter().position(|p| *p == view.phase).unwrap_or(0),
        }
    }
}

impl Panel for PhasePicker {
    fn kind(&self) -> &'static str {
        "phase"
    }

    fn title(&self, _view: &View) -> String {
        "handoff to".into()
    }

    fn legend(&self, _view: &View) -> String {
        "↑↓ move · enter handoff · esc".into()
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        (64, (PHASES.len() * 2 + 1) as u16)
    }

    fn render(&self, view: &View, width: u16, _height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let mut lines: Vec<Line<'static>> = Vec::new();
        for (i, p) in PHASES.iter().enumerate() {
            let sel = i == self.selected;
            let status = if *p == view.phase { "current" } else { "" };
            lines.push(widgets::list_row(
                "●",
                &p.to_string(),
                "",
                status,
                sel,
                w,
                theme,
                Some(theme.phase(*p)),
            ));
            lines.push(widgets::note(&format!("  {}", describe(*p)), theme));
        }
        Body {
            lines,
            scroll: None,
        }
    }

    fn key(&mut self, key: KeyEvent, _view: &mut View) -> Outcome {
        match key.code {
            KeyCode::Esc => Outcome::Close,
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = super::step(self.selected, -1, PHASES.len());
                Outcome::Stay
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = super::step(self.selected, 1, PHASES.len());
                Outcome::Stay
            }
            KeyCode::Enter => Outcome::CloseAct(Action::BeginHandoff(PHASES[self.selected])),
            _ => Outcome::Stay,
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}
