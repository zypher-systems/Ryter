//! The hat rack and the instruments as a popout: what `^b` opens on a
//! screen too narrow to hold them beside the conversation
//! (`docs/hat-rack-design.md`, `R-LAYOUT-05`).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::{Body, Outcome, Panel};
use crate::chat::wrap;
use crate::theme::Theme;
use crate::view::View;

/// Columns the rack's text takes in the panel.
const RACK_TEXT: usize = 27;
/// Columns between the rack and the instruments.
const GAP: usize = 3;
/// Columns the instruments' text takes beside it.
const INSTRUMENTS_TEXT: usize = 31;

/// The side columns, as a panel.
#[derive(Debug, Clone, Default)]
pub struct Rack {
    /// The first row on screen, on a screen too short for all of them.
    first: usize,
    /// The furthest that can go, as the last frame found it.
    last: std::cell::Cell<usize>,
}

impl Rack {
    /// Whether `width` holds the two side by side.
    fn beside(width: usize) -> bool {
        width >= RACK_TEXT + GAP + INSTRUMENTS_TEXT
    }

    fn rows(view: &View, width: usize, theme: Theme) -> Vec<Line<'static>> {
        let bg = theme.sidebar_bg;
        let beside = Self::beside(width);
        let rack_w = if beside { RACK_TEXT } else { width };
        let inst_w = if beside { INSTRUMENTS_TEXT } else { width };
        // Beside the conversation both lead with a blank row, and the rack
        // with its name: here the panel's border and title are those.
        let rack: Vec<Line<'static>> = crate::rail::lines(view, theme, rack_w, usize::MAX)
            .unwrap_or_default()
            .into_iter()
            .skip(3)
            .collect();
        let inst: Vec<Line<'static>> =
            crate::instruments::lines(view, theme, inst_w, usize::MAX, false)
                .into_iter()
                .skip(1)
                .collect();
        if !beside {
            let mut rows = rack;
            rows.push(Line::from(""));
            rows.extend(inst);
            return rows;
        }
        let mut rows = Vec::new();
        for i in 0..rack.len().max(inst.len()) {
            let mut spans = rack.get(i).map(|l| l.spans.clone()).unwrap_or_default();
            let used: usize = spans.iter().map(|s| wrap::width(&s.content)).sum();
            spans.push(Span::styled(
                " ".repeat((RACK_TEXT + GAP).saturating_sub(used)),
                Style::default().bg(bg),
            ));
            if let Some(l) = inst.get(i) {
                spans.extend(l.spans.clone());
            }
            rows.push(Line::from(spans));
        }
        rows
    }
}

impl Panel for Rack {
    fn kind(&self) -> &'static str {
        "rack"
    }

    fn title(&self, _view: &View) -> String {
        "hat rack".into()
    }

    fn legend(&self, _view: &View) -> String {
        if self.last.get() > 0 {
            "↑↓ scroll · esc close".into()
        } else {
            "esc close".into()
        }
    }

    fn size(&self, view: &View) -> (u16, u16) {
        let width = RACK_TEXT + GAP + INSTRUMENTS_TEXT + 2;
        let rows = Self::rows(view, width, Theme::truecolor_dark()).len();
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
