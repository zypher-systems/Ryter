//! The specialists picker (`docs/sidebar-design.md` R-KEY-02): what
//! `Shift+Tab` opens from a primary hat. One row a specialist, what it
//! does, what it may do, and its model; typing filters, `Enter` puts the
//! hat on.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ryter_core::Role;

use super::{Body, Outcome, Panel};
use crate::action::Action;
use crate::chat::{short_model, wrap};
use crate::theme::Theme;
use crate::view::View;

/// The picker.
#[derive(Debug, Clone)]
pub struct Specialists {
    selected: usize,
}

impl Specialists {
    /// Open on the specialist last worn this session, audit when none.
    pub fn new(view: &mut View) -> Self {
        view.composer.clear();
        let selected = crate::sidebar::SPECIALISTS
            .iter()
            .position(|h| *h == view.last_specialist)
            .unwrap_or(0);
        let me = Self { selected };
        view.picker_hover = me.hovered(view);
        me
    }

    /// The specialists that match what has been typed.
    fn filtered(&self, view: &View) -> Vec<Role> {
        let f = view.composer.text().trim().to_ascii_lowercase();
        crate::sidebar::SPECIALISTS
            .iter()
            .copied()
            .filter(|h| f.is_empty() || h.as_str().contains(&f) || h.describe().contains(&f))
            .collect()
    }

    /// The hat under the cursor, for the sidebar to mark.
    fn hovered(&self, view: &View) -> Option<Role> {
        let list = self.filtered(view);
        list.get(self.selected.min(list.len().saturating_sub(1)))
            .copied()
    }
}

impl Panel for Specialists {
    fn kind(&self) -> &'static str {
        "specialists"
    }

    fn title(&self, _view: &View) -> String {
        "specialists".into()
    }

    fn legend(&self, _view: &View) -> String {
        "↑↓ move · enter choose · esc close".into()
    }

    fn input(&self, _view: &View) -> Option<String> {
        Some("filter".into())
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        (84, (crate::sidebar::SPECIALISTS.len() + 2) as u16)
    }

    fn render(&self, view: &View, width: u16, _height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let list = self.filtered(view);
        let sel = self.selected.min(list.len().saturating_sub(1));
        let mut lines = Vec::new();
        if list.is_empty() {
            lines.push(Line::from(Span::styled(
                " no specialist matches",
                theme.panel_muted(),
            )));
        }
        for (i, hat) in list.iter().enumerate() {
            let on = i == sel;
            let bg = if on {
                theme.selection_bg
            } else {
                theme.panel_bg
            };
            let name = Style::default().fg(theme.mode(*hat)).bg(bg);
            let body = Style::default().fg(theme.fg).bg(bg);
            let dim = Style::default().fg(theme.dim).bg(bg);
            let model = short_model(crate::sidebar::hat_model(view, *hat)).to_string();
            let may = crate::sidebar::hat_may(*hat);
            let mut spans = vec![
                Span::styled(if on { " › " } else { "   " }, body),
                Span::styled(
                    format!("{:<10}", hat.as_str()),
                    if on {
                        name.add_modifier(Modifier::BOLD)
                    } else {
                        name
                    },
                ),
                Span::styled(format!("{:<36}", hat.describe()), body),
                Span::styled(format!("{may:<14}"), dim),
                Span::styled(wrap::truncate(&model, w.saturating_sub(63)), dim),
            ];
            let used: usize = spans.iter().map(|s| wrap::width(&s.content)).sum();
            spans.push(Span::styled(" ".repeat(w.saturating_sub(used)), body));
            lines.push(Line::from(spans));
        }
        Body {
            lines,
            scroll: None,
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        let list = self.filtered(view);
        let n = list.len();
        let sel = self.selected.min(n.saturating_sub(1));
        let out = match key.code {
            KeyCode::Esc => {
                view.composer.clear();
                Outcome::Close
            }
            KeyCode::Up => {
                self.selected = super::step(sel, -1, n);
                Outcome::Stay
            }
            KeyCode::Down | KeyCode::Tab => {
                self.selected = super::step(sel, 1, n);
                Outcome::Stay
            }
            KeyCode::Enter => match list.get(sel) {
                Some(hat) => {
                    view.composer.clear();
                    Outcome::CloseAct(Action::SetMode(*hat))
                }
                None => Outcome::Stay,
            },
            _ => {
                super::edit_field(&mut view.composer, key);
                self.selected = 0;
                Outcome::Stay
            }
        };
        view.picker_hover = match out {
            Outcome::Stay => self.hovered(view),
            _ => None,
        };
        out
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}
