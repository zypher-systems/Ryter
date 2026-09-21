//! `/budget` — spend limits: where spend stands, and the caps that stop it.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ryter_core::format_usd;

use super::widgets::{self, Field, Form, Kind};
use super::{Body, Outcome, Panel};
use crate::action::Action;
use crate::chat::wrap;
use crate::theme::Theme;
use crate::view::View;

const ON: &str = "the crew stops when session spend reaches the cap, says what finished and what \
                  didn't, and waits. raise the cap and tell the lead to continue.";
const OFF: &str = "nothing stops on cost. the spend card keeps count; watch it yourself.";
const TASK: &str = "one task stops at this, budget or not, and keeps its work on its branch. \
                    it catches a single runaway task.";

/// Budget form with a live status header.
#[derive(Debug, Clone)]
pub struct Budget {
    form: Form,
    /// Editing the focused number via the composer.
    editing: bool,
    /// `Esc` with edits asks first.
    confirm_discard: bool,
}

fn num(id: &'static str, label: &str, value: f64, min: f64, default: f64) -> Field {
    Field::new(
        id,
        label,
        Kind::Number {
            value,
            min,
            max: 10_000.0,
            step: if id == "task" { 0.25 } else { 0.5 },
            int: false,
        },
    )
    .origin(if (value - default).abs() < f64::EPSILON {
        "default"
    } else {
        "config"
    })
}

impl Budget {
    /// Build from the live view.
    pub fn new(view: &View) -> Self {
        let on = view.budget_usd > 0.0;
        let amount = if on {
            view.budget_usd
        } else {
            view.budget_last.max(0.5)
        };
        let fields = vec![
            Field::new("g_session", "session", Kind::Header),
            Field::new("on", "cap spend", Kind::Toggle(on)).origin(if on {
                "default"
            } else {
                "config"
            }),
            num("amount", "cap usd", amount, 0.5, 5.0),
            num("warn", "warn at usd", view.warn_usd, 0.0, 1.0),
            Field::new("g_task", "per task", Kind::Header),
            num("task", "task cap usd", view.task_budget_usd, 0.25, 1.0),
        ];
        Self {
            form: Form::new(fields),
            editing: false,
            confirm_discard: false,
        }
    }

    fn number(&self, id: &str) -> f64 {
        match self.form.get(id).map(|f| &f.kind) {
            Some(Kind::Number { value, .. }) => *value,
            _ => 0.0,
        }
    }

    fn on(&self) -> bool {
        matches!(
            self.form.get("on").map(|f| &f.kind),
            Some(Kind::Toggle(true))
        )
    }

    fn save(&self) -> Action {
        Action::SaveBudget {
            usd: if self.on() {
                self.number("amount")
            } else {
                0.0
            },
            warn: self.number("warn"),
            task: self.number("task"),
        }
    }

    /// Where spend stands, from the live view (not the unsaved form).
    fn status_lines(view: &View, w: usize, theme: Theme) -> Vec<ratatui::text::Line<'static>> {
        let mut lines = Vec::new();
        let cells = w.saturating_sub(44).clamp(10, 24);
        if view.budget_usd > 0.0 {
            match view.spend {
                Some(spent) => {
                    let frac = spent / view.budget_usd;
                    let color = if spent >= view.budget_usd {
                        theme.error
                    } else if spent >= view.warn_usd && view.warn_usd > 0.0 {
                        theme.warn
                    } else {
                        theme.success
                    };
                    let left = (view.budget_usd - spent).max(0.0);
                    lines.push(widgets::gauge(
                        "spent",
                        frac,
                        &format!(
                            "{}%  {} of {} · {} left",
                            (frac * 100.0).round().min(999.0) as u32,
                            format_usd(Some(spent)),
                            format_usd(Some(view.budget_usd)),
                            format_usd(Some(left))
                        ),
                        cells,
                        theme.panel_bg,
                        theme,
                        color,
                    ));
                }
                None => lines.push(widgets::note(
                    &format!(
                        "spent {} of {} · a price is unknown, so no percentage",
                        format_usd(None),
                        format_usd(Some(view.budget_usd))
                    ),
                    theme,
                )),
            }
        } else {
            lines.push(widgets::text(
                &format!("spent {} · no cap", format_usd(view.spend)),
                theme,
            ));
        }
        let mut by_role: Vec<(&String, &f64)> = view.spend_by_role.iter().collect();
        by_role.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
        if !by_role.is_empty() {
            let parts: Vec<String> = by_role
                .iter()
                .map(|(r, v)| format!("{r} {}", format_usd(Some(**v))))
                .collect();
            lines.push(widgets::note(&parts.join(" · "), theme));
        }
        lines
    }
}

impl Panel for Budget {
    fn kind(&self) -> &'static str {
        "budget"
    }

    fn title(&self, _view: &View) -> String {
        "budget".into()
    }

    fn status(&self, view: &View) -> String {
        if self.form.dirty {
            "unsaved".into()
        } else if view.budget_usd > 0.0 {
            format!("on · {}", format_usd(Some(view.budget_usd)))
        } else {
            "off".into()
        }
    }

    fn legend(&self, _view: &View) -> String {
        if self.confirm_discard {
            "discard changes? y / n".into()
        } else if self.editing {
            "type · enter apply · esc cancel".into()
        } else {
            "↑↓ move · space/←→ change · enter edit · ^s save · esc".into()
        }
    }

    fn input(&self, _view: &View) -> Option<String> {
        self.editing.then(|| {
            self.form
                .current()
                .map(|f| f.label.clone())
                .unwrap_or_else(|| "value".into())
        })
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        (72, 17)
    }

    fn render(&self, view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let mut lines = Self::status_lines(view, w, theme);
        lines.push(widgets::blank(theme));
        lines.extend(self.form.render(w, theme, true));
        lines.push(widgets::blank(theme));
        // Explain the field under the cursor's group, in words.
        let prose = match self.form.current().map(|f| f.id) {
            Some("task") => TASK,
            _ if self.on() => ON,
            _ => OFF,
        };
        for row in wrap::wrap_plain(prose, w.saturating_sub(2)) {
            lines.push(widgets::note(&row, theme));
        }
        if self.confirm_discard {
            lines.push(widgets::colored(
                "unsaved changes · y discard · n keep editing",
                theme.warn,
                theme,
            ));
        }
        lines.truncate(usize::from(height).max(1));
        Body {
            lines,
            scroll: None,
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        if self.confirm_discard {
            return match key.code {
                KeyCode::Char('y' | 'Y') => Outcome::Close,
                KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                    self.confirm_discard = false;
                    Outcome::Stay
                }
                _ => Outcome::Stay,
            };
        }
        if self.editing {
            return match key.code {
                KeyCode::Esc => {
                    self.editing = false;
                    view.composer.clear();
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    let text = view
                        .composer
                        .text()
                        .trim()
                        .trim_start_matches('$')
                        .to_string();
                    if self.form.set_from_text(&text) {
                        self.editing = false;
                        view.composer.clear();
                        // Typing an amount means you want a cap.
                        if self.form.current().is_some_and(|f| f.id == "amount") && !self.on() {
                            if let Some(f) = self.form.get_mut("on") {
                                f.kind = Kind::Toggle(true);
                            }
                        }
                    }
                    Outcome::Stay
                }
                _ => {
                    super::edit_field(&mut view.composer, key);
                    Outcome::Stay
                }
            };
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                if self.form.dirty {
                    self.confirm_discard = true;
                    Outcome::Stay
                } else {
                    Outcome::Close
                }
            }
            KeyCode::Char('s') if ctrl => {
                if self.form.has_errors() {
                    return Outcome::Stay;
                }
                self.form.dirty = false;
                Outcome::Act(self.save())
            }
            KeyCode::Up | KeyCode::BackTab => {
                self.form.step(-1);
                Outcome::Stay
            }
            KeyCode::Down | KeyCode::Tab => {
                self.form.step(1);
                Outcome::Stay
            }
            KeyCode::Left => {
                self.form.adjust(-1);
                Outcome::Stay
            }
            KeyCode::Right | KeyCode::Char(' ') => {
                self.form.adjust(1);
                Outcome::Stay
            }
            KeyCode::Enter => {
                match self.form.current().map(|f| &f.kind) {
                    Some(Kind::Toggle(_)) => {
                        self.form.adjust(1);
                    }
                    Some(Kind::Number { .. }) => {
                        let v = self
                            .form
                            .current()
                            .map(Field::value_text)
                            .unwrap_or_default();
                        view.composer.set_text(&v);
                        self.editing = true;
                    }
                    _ => {}
                }
                Outcome::Stay
            }
            _ => Outcome::Stay,
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

    fn view() -> View {
        View::new(
            ryter_core::Phase::Build,
            "c".into(),
            "m".into(),
            "/tmp".into(),
        )
    }

    fn press(p: &mut Budget, v: &mut View, code: KeyCode) -> Outcome {
        p.key(KeyEvent::new(code, KeyModifiers::NONE), v)
    }

    fn save(p: &mut Budget, v: &mut View) -> Action {
        match p.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL), v) {
            Outcome::Act(a) => a,
            _ => panic!("^s did not save"),
        }
    }

    /// Off keeps the amount, so switching back on restores it.
    #[test]
    fn switching_off_and_on_keeps_the_amount() {
        let mut v = view();
        v.budget_usd = 7.5;
        let mut p = Budget::new(&v);
        // Cursor starts on "cap spend".
        press(&mut p, &mut v, KeyCode::Char(' '));
        assert_eq!(
            save(&mut p, &mut v),
            Action::SaveBudget {
                usd: 0.0,
                warn: 1.0,
                task: 1.0
            }
        );
        v.budget_usd = 0.0;
        v.budget_last = 7.5;
        let mut p = Budget::new(&v);
        press(&mut p, &mut v, KeyCode::Char(' '));
        assert!(matches!(save(&mut p, &mut v), Action::SaveBudget { usd, .. } if usd == 7.5));
    }

    /// Typing a cap while the budget is off means "turn it on at this".
    #[test]
    fn typing_an_amount_turns_the_cap_on() {
        let mut v = view();
        v.budget_usd = 0.0;
        let mut p = Budget::new(&v);
        press(&mut p, &mut v, KeyCode::Down);
        press(&mut p, &mut v, KeyCode::Enter);
        v.composer.set_text("$12");
        press(&mut p, &mut v, KeyCode::Enter);
        assert!(matches!(save(&mut p, &mut v), Action::SaveBudget { usd, .. } if usd == 12.0));
    }
}
