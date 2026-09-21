//! `/doctor` — diagnostics panel (`R-POP-71..74`).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::{Body, Notice, Outcome, Panel, widgets};
use crate::action::Action;
use crate::activity::SPINNER;
use crate::chat::wrap;
use crate::theme::Theme;
use crate::view::View;

/// One check row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Check name.
    pub name: String,
    /// `ok` / `warn` / `fail`.
    pub status: String,
    /// Detail / remedy text.
    pub detail: String,
}

/// Diagnostics.
#[derive(Debug, Clone)]
pub struct Doctor {
    rows: Vec<Row>,
    running: bool,
    selected: usize,
    expanded: Option<usize>,
    saved: Option<String>,
    /// `trusted · sandbox workspace` context for the status slot.
    context: String,
}

impl Doctor {
    /// Start in the running state; the loop runs checks off-thread.
    pub fn new(env: &super::PanelEnv) -> Self {
        Self {
            rows: Vec::new(),
            running: true,
            selected: 0,
            expanded: None,
            saved: None,
            context: format!(
                "{} · sandbox {}",
                if env.trusted { "trusted" } else { "untrusted" },
                env.sandbox
            ),
        }
    }
}

impl Panel for Doctor {
    fn kind(&self) -> &'static str {
        "doctor"
    }

    fn title(&self, _view: &View) -> String {
        "doctor".into()
    }

    fn status(&self, _view: &View) -> String {
        if self.running {
            format!("running… · {}", self.context)
        } else {
            let fails = self.rows.iter().filter(|r| r.status == "fail").count();
            let warns = self.rows.iter().filter(|r| r.status == "warn").count();
            let verdict = match (fails, warns) {
                (0, 0) => "all ok".to_string(),
                (0, w) => format!("{w} warn"),
                (f, 0) => format!("{f} fail"),
                (f, w) => format!("{f} fail · {w} warn"),
            };
            format!("{verdict} · {}", self.context)
        }
    }

    fn legend(&self, _view: &View) -> String {
        match &self.saved {
            Some(p) => format!("saved {p} · esc"),
            None => "↑↓ move · enter details · c copy/save report · esc".into(),
        }
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        (72, (self.rows.len().max(4) + 3).min(24) as u16)
    }

    fn render(&self, view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let h = usize::from(height).max(1);
        let mut lines: Vec<Line<'static>> = Vec::new();
        if self.running && self.rows.is_empty() {
            let frame = SPINNER[(view.now_ms / 80) as usize % SPINNER.len()];
            lines.push(Line::from(vec![
                Span::styled(format!(" {frame} "), theme.on_panel(theme.accent)),
                Span::styled("running checks…", theme.panel()),
            ]));
        }
        let n = self.rows.len();
        let first = super::window(self.selected, n, h);
        for (i, r) in self.rows.iter().enumerate().skip(first).take(h) {
            let (glyph, color) = match r.status.as_str() {
                "ok" => ("✓", theme.success),
                "warn" => ("!", theme.warn),
                _ => ("✕", theme.error),
            };
            let sel = i == self.selected;
            let detail = if self.expanded == Some(i) {
                String::new()
            } else {
                wrap::truncate(&r.detail, w.saturating_sub(24))
            };
            lines.push(widgets::list_row(
                glyph,
                &r.name,
                &detail,
                "",
                sel,
                w,
                theme,
                Some(color),
            ));
            if self.expanded == Some(i) {
                for row in wrap::wrap_plain(&r.detail, w.saturating_sub(6)) {
                    lines.push(Line::from(Span::styled(
                        format!("     {row}"),
                        Style::default().fg(color).bg(theme.panel_bg),
                    )));
                }
            }
        }
        Body {
            lines,
            scroll: (n > h).then_some((first, n)),
        }
    }

    fn key(&mut self, key: KeyEvent, _view: &mut View) -> Outcome {
        let n = self.rows.len();
        match key.code {
            KeyCode::Esc => Outcome::Close,
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = super::step(self.selected, -1, n);
                Outcome::Stay
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = super::step(self.selected, 1, n);
                Outcome::Stay
            }
            KeyCode::Enter => {
                if n > 0 {
                    self.expanded = if self.expanded == Some(self.selected) {
                        None
                    } else {
                        Some(self.selected)
                    };
                }
                Outcome::Stay
            }
            KeyCode::Char('c') if !self.running => Outcome::Act(Action::SaveDoctorReport),
            _ => Outcome::Stay,
        }
    }

    fn on_notice(&mut self, n: &Notice, _view: &mut View) {
        match n {
            Notice::Doctor(rows) => {
                self.rows = rows
                    .iter()
                    .map(|(name, status, detail)| Row {
                        name: name.clone(),
                        status: status.clone(),
                        detail: detail.clone(),
                    })
                    .collect();
                self.running = false;
                self.selected = self.rows.iter().position(|r| r.status != "ok").unwrap_or(0);
            }
            Notice::Exported(p) => self.saved = Some(p.clone()),
            _ => {}
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}
