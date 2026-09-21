//! `/context` — context inspector (`R-POP-64..66`).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::text::Line;
use ryter_core::{AgentEvent, format_tokens};

use super::{Body, Outcome, Panel, widgets};
use crate::action::Action;
use crate::chat::wrap;
use crate::theme::Theme;
use crate::view::View;

/// Context inspector.
#[derive(Debug, Clone, Default)]
pub struct Context {
    requested: bool,
}

impl Panel for Context {
    fn kind(&self) -> &'static str {
        "context"
    }

    fn title(&self, _view: &View) -> String {
        "context".into()
    }

    fn status(&self, view: &View) -> String {
        format!("{}%", (view.ctx_frac() * 100.0).round() as u32)
    }

    fn legend(&self, _view: &View) -> String {
        "c compact · r refresh · esc".into()
    }

    fn size(&self, view: &View) -> (u16, u16) {
        (64, (8 + view.ctx_breakdown.len()).min(24) as u16)
    }

    fn render(&self, view: &View, width: u16, _height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let used = view.ctx_tokens.unwrap_or(0);
        let window = view.ctx_window_or_default();
        let frac = view.ctx_frac();
        let mut lines: Vec<Line<'static>> = Vec::new();
        lines.push(widgets::gauge(
            "used",
            frac,
            &format!(
                "{}%  {} / {} tokens",
                (frac * 100.0).round() as u32,
                format_tokens(used),
                format_tokens(window)
            ),
            w.saturating_sub(40).clamp(12, 24),
            theme.panel_bg,
            theme,
            widgets::gauge_color(frac, theme),
        ));
        match view.ctx_messages {
            Some(n) => lines.push(widgets::note(&format!("{n} transcript messages"), theme)),
            None if !self.requested => lines.push(widgets::note("asking the agent…", theme)),
            None => lines.push(widgets::note("message count unavailable", theme)),
        }
        lines.push(widgets::blank(theme));
        if view.ctx_breakdown.is_empty() {
            lines.push(widgets::note("no breakdown yet · r to refresh", theme));
        } else {
            let total: u64 = view
                .ctx_breakdown
                .iter()
                .map(|(_, n)| *n)
                .sum::<u64>()
                .max(1);
            let rows: Vec<Vec<String>> = view
                .ctx_breakdown
                .iter()
                .map(|(name, n)| {
                    vec![
                        name.clone(),
                        format_tokens(*n),
                        format!("{}%", (*n * 100 / total).min(100)),
                    ]
                })
                .collect();
            lines.extend(widgets::table(
                &["contributor", "tokens", "share"],
                &rows,
                &[widgets::Al::L, widgets::Al::R, widgets::Al::R],
                None,
                w,
                theme,
            ));
        }
        lines.push(widgets::blank(theme));
        let transcript = view
            .ctx_breakdown
            .iter()
            .filter(|(n, _)| n.contains("transcript") || n.contains("tool"))
            .map(|(_, n)| *n)
            .sum::<u64>();
        let reclaim = transcript.saturating_sub(transcript / 4);
        if reclaim > 0 {
            lines.push(widgets::colored(
                &wrap::truncate(
                    &format!(
                        "c compact · would reclaim about {} tokens",
                        format_tokens(reclaim)
                    ),
                    w.saturating_sub(2),
                ),
                if frac >= 0.85 {
                    theme.warn
                } else {
                    theme.accent
                },
                theme,
            ));
        } else {
            lines.push(widgets::note("c compact · summarizes older turns", theme));
        }
        Body {
            lines,
            scroll: None,
        }
    }

    fn key(&mut self, key: KeyEvent, _view: &mut View) -> Outcome {
        match key.code {
            KeyCode::Esc => Outcome::Close,
            KeyCode::Char('c') => Outcome::CloseAct(Action::Compact),
            KeyCode::Char('r') | KeyCode::Enter => {
                self.requested = true;
                Outcome::Act(Action::Context)
            }
            _ => Outcome::Stay,
        }
    }

    fn on_event(&mut self, ev: &AgentEvent, _view: &mut View) {
        if matches!(ev, AgentEvent::Context { .. }) {
            self.requested = true;
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}
