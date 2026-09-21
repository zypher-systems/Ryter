//! `/settings` — grouped settings form (`R-POP-47..50`).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ryter_core::UiConfig;

use super::widgets::{Field, Form, Kind};
use super::{Body, Outcome, Panel};
use crate::action::Action;
use crate::theme::Theme;
use crate::view::View;

/// Settings form.
#[derive(Debug, Clone)]
pub struct Settings {
    form: Form,
    /// Editing the focused text/number field via the composer.
    editing: bool,
    /// `Esc` with edits asks first (`R-POP-50`).
    confirm_discard: bool,
}

fn origin<T: PartialEq>(cur: &T, default: &T) -> &'static str {
    if cur == default { "default" } else { "config" }
}

#[allow(clippy::too_many_arguments)]
fn num(
    id: &'static str,
    label: &str,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
    int: bool,
    default: f64,
) -> Field {
    Field::new(
        id,
        label,
        Kind::Number {
            value,
            min,
            max,
            step,
            int,
        },
    )
    .origin(origin(&value, &default))
}

fn select(id: &'static str, label: &str, options: &[&str], cur: &str, default: &str) -> Field {
    let idx = options.iter().position(|o| *o == cur).unwrap_or(0);
    Field::new(
        id,
        label,
        Kind::Select {
            options: options.iter().map(|s| (*s).to_string()).collect(),
            idx,
        },
    )
    .origin(origin(&cur, &default))
}

impl Settings {
    /// Build from the live view.
    pub fn new(view: &View) -> Self {
        let d = UiConfig::default();
        let mut theme_opts: Vec<&str> = view.theme_names.iter().map(String::as_str).collect();
        if theme_opts.is_empty() {
            theme_opts = vec!["dark"];
        }
        let fields = vec![
            Field::new("g_spend", "spend", Kind::Header),
            num(
                "budget",
                "session budget usd",
                view.budget_usd,
                0.0,
                10_000.0,
                0.5,
                false,
                5.0,
            ),
            num(
                "warn",
                "warn usd",
                view.warn_usd,
                0.0,
                10_000.0,
                0.25,
                false,
                1.0,
            ),
            Field::new("g_agents", "agents", Kind::Header),
            num(
                "max",
                "subagents max",
                f64::from(view.max_crew),
                1.0,
                16.0,
                1.0,
                true,
                4.0,
            ),
            Field::new("auditor", "auditor", Kind::Toggle(view.auditor_on))
                .origin(origin(&view.auditor_on, &true)),
            Field::new("g_tools", "tools", Kind::Header),
            select(
                "perm",
                "permission mode",
                &["ask", "always"],
                &view.perm_mode,
                "ask",
            ),
            Field::new("web", "features.web", Kind::Toggle(view.web))
                .origin(origin(&view.web, &false)),
            Field::new("g_sandbox", "sandbox", Kind::Header),
            select(
                "sandbox",
                "profile",
                &["off", "workspace", "read-only"],
                &view.sandbox_profile,
                "off",
            ),
            Field::new("g_mcp", "mcp", Kind::Header),
            Field::new("inbound", "inbound", Kind::Toggle(view.mcp_inbound))
                .origin(origin(&view.mcp_inbound, &true)),
            Field::new(
                "bind",
                "bind address",
                Kind::Text(view.mcp_bind.clone().unwrap_or_default()),
            )
            .origin(if view.mcp_bind.is_none() {
                "default"
            } else {
                "config"
            }),
            Field::new("g_ui", "ui", Kind::Header),
            select("theme", "theme", &theme_opts, &view.theme_name, &d.theme),
            Field::new("username", "username", Kind::Text(view.ui.username.clone()))
                .origin(origin(&view.ui.username, &d.username)),
            select(
                "reasoning",
                "reasoning",
                &["collapsed", "expanded", "off"],
                &view.ui.reasoning,
                &d.reasoning,
            ),
            Field::new("mouse", "mouse", Kind::Toggle(view.ui.mouse))
                .origin(origin(&view.ui.mouse, &d.mouse)),
            Field::new("panel", "panel", Kind::Toggle(view.ui.panel))
                .origin(origin(&view.ui.panel, &d.panel)),
            Field::new("timestamps", "timestamps", Kind::Toggle(view.ui.timestamps))
                .origin(origin(&view.ui.timestamps, &d.timestamps)),
            Field::new(
                "line_numbers",
                "line numbers",
                Kind::Toggle(view.ui.line_numbers),
            )
            .origin(origin(&view.ui.line_numbers, &d.line_numbers)),
        ];
        Self {
            form: Form::new(fields),
            editing: false,
            confirm_discard: false,
        }
    }

    /// Copy form values back into the view (the loop persists).
    fn apply(&self, view: &mut View) {
        let f = &self.form;
        let number = |id: &str| -> Option<f64> {
            match f.get(id).map(|x| &x.kind) {
                Some(Kind::Number { value, .. }) => Some(*value),
                _ => None,
            }
        };
        let toggle = |id: &str| -> Option<bool> {
            match f.get(id).map(|x| &x.kind) {
                Some(Kind::Toggle(b)) => Some(*b),
                _ => None,
            }
        };
        let text = |id: &str| -> Option<String> {
            match f.get(id).map(|x| &x.kind) {
                Some(Kind::Text(s)) => Some(s.clone()),
                _ => None,
            }
        };
        let sel = |id: &str| -> Option<String> { f.get(id).map(Field::value_text) };
        if let Some(v) = number("budget") {
            view.budget_usd = v;
        }
        if let Some(v) = number("warn") {
            view.warn_usd = v;
        }
        if let Some(v) = number("max") {
            view.max_crew = v.round().clamp(1.0, 16.0) as u32;
        }
        if let Some(v) = toggle("auditor") {
            view.auditor_on = v;
        }
        if let Some(v) = sel("perm") {
            view.perm_mode = v;
        }
        if let Some(v) = toggle("web") {
            view.web = v;
        }
        if let Some(v) = sel("sandbox") {
            view.sandbox_profile = v;
        }
        if let Some(v) = toggle("inbound") {
            view.mcp_inbound = v;
        }
        if let Some(v) = text("bind") {
            view.mcp_bind = (!v.trim().is_empty()).then(|| v.trim().to_string());
        }
        if let Some(v) = sel("theme") {
            view.ui.theme = v;
        }
        if let Some(v) = text("username") {
            view.ui.username = v;
        }
        if let Some(v) = sel("reasoning") {
            view.ui.reasoning = v;
        }
        if let Some(v) = toggle("mouse") {
            view.ui.mouse = v;
        }
        if let Some(v) = toggle("panel") {
            view.ui.panel = v;
        }
        if let Some(v) = toggle("timestamps") {
            view.ui.timestamps = v;
        }
        if let Some(v) = toggle("line_numbers") {
            view.ui.line_numbers = v;
        }
    }
}

impl Panel for Settings {
    fn kind(&self) -> &'static str {
        "settings"
    }

    fn title(&self, _view: &View) -> String {
        "settings".into()
    }

    fn status(&self, _view: &View) -> String {
        if self.form.dirty {
            "unsaved".into()
        } else {
            String::new()
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
        (72, 26)
    }

    fn render(&self, _view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let rows = self.form.render(usize::from(width), theme, true);
        let total = rows.len();
        let h = usize::from(height).max(1);
        // Keep the selected row visible: estimate its row index.
        let sel_row = self
            .form
            .fields
            .iter()
            .take(self.form.selected)
            .map(|f| if matches!(f.kind, Kind::Header) { 2 } else { 1 } + usize::from(f.error.is_some()))
            .sum::<usize>();
        let first = super::window(sel_row, total, h);
        let mut lines: Vec<_> = rows.into_iter().skip(first).take(h).collect();
        if self.confirm_discard {
            lines.pop();
            lines.push(super::widgets::colored(
                "unsaved changes · y discard · n keep editing",
                theme.warn,
                theme,
            ));
        }
        Body {
            lines,
            scroll: (total > h).then_some((first, total)),
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
                    let text = view.composer.text().to_string();
                    if self.form.set_from_text(&text) {
                        self.editing = false;
                        view.composer.clear();
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
                self.apply(view);
                self.form.dirty = false;
                Outcome::Act(Action::SaveSettings)
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
                    Some(Kind::Text(s)) => {
                        view.composer.set_text(s);
                        self.editing = true;
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
                    Some(Kind::Select { .. }) => {
                        self.form.adjust(1);
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
