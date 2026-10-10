//! The sidebar as a popout: what `^b` opens on a screen too narrow to
//! hold it beside the conversation (`docs/sidebar-design.md`,
//! `R-LAYOUT-05`).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::text::Line;

use super::{Body, Outcome, Panel};
use crate::theme::Theme;
use crate::view::View;

/// Columns the sidebar's text takes in the panel.
const TEXT: usize = 28;

/// The sidebar, as a panel.
#[derive(Debug, Clone, Default)]
pub struct Sidebar {
    /// The first row on screen, on a screen too short for all of them.
    first: usize,
    /// The furthest that can go, as the last frame found it.
    last: std::cell::Cell<usize>,
}

impl Sidebar {
    fn rows(view: &View, width: usize, theme: Theme) -> Vec<Line<'static>> {
        crate::sidebar::panel_lines(view, theme, width.min(TEXT.max(width.saturating_sub(2))))
    }
}

impl Panel for Sidebar {
    fn kind(&self) -> &'static str {
        "sidebar"
    }

    fn title(&self, _view: &View) -> String {
        "sidebar".into()
    }

    fn legend(&self, _view: &View) -> String {
        if self.last.get() > 0 {
            "↑↓ scroll · esc close".into()
        } else {
            "esc close".into()
        }
    }

    fn size(&self, view: &View) -> (u16, u16) {
        let width = TEXT + 4;
        let rows = Self::rows(view, TEXT, Theme::truecolor_dark()).len();
        (width as u16, rows.min(60) as u16)
    }

    fn render(&self, view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let rows = Self::rows(view, usize::from(width), theme);
        let (total, h) = (rows.len(), usize::from(height).max(1));
        self.last.set(total.saturating_sub(h));
        let first = self.first.min(self.last.get());
        Body {
            lines: rows.into_iter().skip(first).take(h).collect(),
            scroll: (total > h).then_some((first, total)),
        }
    }

    fn key(&mut self, key: KeyEvent, _view: &mut View) -> Outcome {
        let last = self.last.get();
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => return Outcome::Close,
            KeyCode::Down | KeyCode::Char('j') => self.first = (self.first + 1).min(last),
            KeyCode::Up | KeyCode::Char('k') => self.first = self.first.saturating_sub(1),
            KeyCode::PageDown => self.first = (self.first + 8).min(last),
            KeyCode::PageUp => self.first = self.first.saturating_sub(8),
            KeyCode::Home => self.first = 0,
            KeyCode::End => self.first = last,
            _ => {}
        }
        Outcome::Stay
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}
