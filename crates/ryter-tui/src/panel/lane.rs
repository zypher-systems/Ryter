//! One crew lane's transcript: everything its builder and auditor said they
//! were doing, as it happened. `⏎` on a lane picked with `tab` opens it.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::{Body, Outcome, Panel};
use crate::chat::wrap;
use crate::theme::Theme;
use crate::view::View;

/// A lane's transcript.
#[derive(Debug, Clone)]
pub struct Lane {
    id: String,
    /// Rows scrolled up from the newest.
    back: usize,
}

impl Lane {
    /// The transcript of subagent `id`.
    pub fn new(id: String) -> Self {
        Self { id, back: 0 }
    }
}

impl Panel for Lane {
    fn kind(&self) -> &'static str {
        "lane"
    }

    fn title(&self, view: &View) -> String {
        match view.lane_logs.get(&self.id) {
            Some((task, _)) => format!("lane · {task}"),
            None => "lane".into(),
        }
    }

    fn status(&self, view: &View) -> String {
        let n = view.lane_logs.get(&self.id).map_or(0, |(_, l)| l.len());
        format!("{n} steps")
    }

    fn legend(&self, _view: &View) -> String {
        "↑↓ scroll · esc close".into()
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        (110, 24)
    }

    fn render(&self, view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let dim = Style::default().fg(theme.dim).bg(theme.panel_bg);
        let fg = Style::default().fg(theme.fg).bg(theme.panel_bg);
        let w = usize::from(width);
        let Some((_, lines)) = view.lane_logs.get(&self.id) else {
            return Body {
                lines: vec![Line::from(Span::styled(
                    "nothing recorded for this lane",
                    dim,
                ))],
                scroll: None,
            };
        };
        let rows: Vec<Line<'static>> = lines
            .iter()
            .flat_map(|l| {
                // `13:47  builder  edit src/greet.rs`: the time dim, the rest plain.
                let cut = l.char_indices().nth(7).map_or(l.len(), |(i, _)| i);
                let (at, rest) = l.split_at(cut);
                let mut first = true;
                wrap::wrap_plain(rest, w.saturating_sub(8))
                    .into_iter()
                    .map(move |part| {
                        let lead = if first { at.to_string() } else { " ".repeat(7) };
                        first = false;
                        Line::from(vec![Span::styled(lead, dim), Span::styled(part, fg)])
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let h = usize::from(height);
        let total = rows.len();
        let end = total.saturating_sub(self.back.min(total.saturating_sub(h)));
        let start = end.saturating_sub(h);
        Body {
            lines: rows[start..end].to_vec(),
            scroll: Some((start, total)),
        }
    }

    fn key(&mut self, key: KeyEvent, _view: &mut View) -> Outcome {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => Outcome::Close,
            KeyCode::Up | KeyCode::Char('k') => {
                self.back += 1;
                Outcome::Stay
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.back = self.back.saturating_sub(1);
                Outcome::Stay
            }
            KeyCode::PageUp => {
                self.back += 10;
                Outcome::Stay
            }
            KeyCode::PageDown => {
                self.back = self.back.saturating_sub(10);
                Outcome::Stay
            }
            _ => Outcome::Stay,
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}
