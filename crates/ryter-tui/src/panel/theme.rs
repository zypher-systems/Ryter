//! `/theme` — picker with live preview and swatches (`R-POP-51..53`).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::{Body, Outcome, Panel, widgets};
use crate::action::Action;
use crate::theme::Theme;
use crate::view::View;

/// Theme picker.
#[derive(Debug, Clone)]
pub struct ThemePicker {
    selected: usize,
    original: String,
}

impl ThemePicker {
    /// Start on the active theme.
    pub fn new(view: &View) -> Self {
        Self {
            selected: view
                .theme_names
                .iter()
                .position(|n| *n == view.theme_name)
                .unwrap_or(0),
            original: view.theme_name.clone(),
        }
    }

    fn name(&self, view: &View) -> Option<String> {
        view.theme_names.get(self.selected).cloned()
    }
}

impl Panel for ThemePicker {
    fn kind(&self) -> &'static str {
        "theme"
    }

    fn title(&self, _view: &View) -> String {
        "theme".into()
    }

    fn status(&self, view: &View) -> String {
        format!("{}", view.theme_names.len())
    }

    fn legend(&self, _view: &View) -> String {
        "↑↓ preview · enter apply · esc revert".into()
    }

    fn size(&self, view: &View) -> (u16, u16) {
        (56, (view.theme_names.len() + 3).min(20) as u16)
    }

    fn render(&self, view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let list_h = usize::from(height).saturating_sub(3).max(1);
        let n = view.theme_names.len();
        let first = super::window(self.selected, n, list_h);
        let mut lines: Vec<Line<'static>> = Vec::new();
        for (i, name) in view.theme_names.iter().enumerate().skip(first).take(list_h) {
            let status = if *name == self.original {
                "current"
            } else {
                ""
            };
            let builtin = crate::theme::BUILTIN_THEMES.contains(&name.as_str());
            lines.push(widgets::list_row(
                if builtin { "◆" } else { "◇" },
                name,
                if builtin {
                    "built-in"
                } else {
                    "~/.ryter/themes"
                },
                status,
                i == self.selected,
                w,
                theme,
                Some(theme.accent),
            ));
        }
        lines.push(widgets::blank(theme));
        // Swatch strip (R-POP-53) from the theme currently previewed.
        let sw = |c| Span::styled("██", Style::default().fg(c).bg(theme.panel_bg));
        lines.push(Line::from(vec![
            Span::styled(" ", theme.panel()),
            sw(theme.fg),
            sw(theme.dim),
            sw(theme.accent),
            sw(theme.user),
            sw(theme.assistant),
            sw(theme.tool),
            sw(theme.plan),
            sw(theme.architect),
            sw(theme.build),
            sw(theme.audit),
            sw(theme.success),
            sw(theme.warn),
            sw(theme.error),
            Span::styled(
                format!("  {:?}", theme.mode).to_ascii_lowercase(),
                theme.panel_muted(),
            ),
        ]));
        Body {
            lines,
            scroll: (n > list_h).then_some((first, n)),
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        let n = view.theme_names.len();
        match key.code {
            KeyCode::Esc => Outcome::CloseAct(Action::RevertTheme),
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = super::step(self.selected, -1, n);
                self.name(view)
                    .map_or(Outcome::Stay, |t| Outcome::Act(Action::PreviewTheme(t)))
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = super::step(self.selected, 1, n);
                self.name(view)
                    .map_or(Outcome::Stay, |t| Outcome::Act(Action::PreviewTheme(t)))
            }
            KeyCode::Enter => self
                .name(view)
                .map_or(Outcome::Close, |t| Outcome::CloseAct(Action::SetTheme(t))),
            _ => Outcome::Stay,
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}
