//! `/spend` — spend detail (`R-POP-43..46`).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::text::Line;
use ryter_core::{format_tokens, format_usd};

use super::{Body, Notice, Outcome, Panel, widgets};
use crate::action::Action;
use crate::theme::Theme;
use crate::view::{SpendRow, View};

/// Spend panel.
#[derive(Debug, Clone, Default)]
pub struct Spend {
    exported: Option<String>,
    scroll: usize,
}

fn rows(map: &std::collections::BTreeMap<String, SpendRow>) -> Vec<Vec<String>> {
    let mut v: Vec<(&String, &SpendRow)> = map.iter().collect();
    v.sort_by(|a, b| {
        b.1.usd
            .partial_cmp(&a.1.usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    v.into_iter()
        .map(|(name, r)| {
            vec![
                name.clone(),
                r.calls.to_string(),
                format_tokens(r.input),
                format_tokens(r.output),
                format_tokens(r.cached),
                if r.unpriced && r.usd == 0.0 {
                    "$?.??".into()
                } else if r.unpriced {
                    format!("{}+", format_usd(Some(r.usd)))
                } else {
                    format_usd(Some(r.usd))
                },
            ]
        })
        .collect()
}

const HEADERS: [&str; 6] = ["", "calls", "input", "output", "cached", "usd"];
const ALIGNS: [widgets::Al; 6] = [
    widgets::Al::L,
    widgets::Al::R,
    widgets::Al::R,
    widgets::Al::R,
    widgets::Al::R,
    widgets::Al::R,
];

/// CSV body for the export (`R-POP-46`).
pub fn csv(view: &View) -> String {
    let mut s =
        String::from("group,name,calls,input_tokens,output_tokens,cached_tokens,usd,unpriced\n");
    for (group, map) in [
        ("role", &view.spend_rows_role),
        ("connection", &view.spend_rows_conn),
    ] {
        for (name, r) in map {
            s.push_str(&format!(
                "{group},{name},{},{},{},{},{:.6},{}\n",
                r.calls, r.input, r.output, r.cached, r.usd, r.unpriced
            ));
        }
    }
    s
}

impl Panel for Spend {
    fn kind(&self) -> &'static str {
        "spend"
    }

    fn title(&self, _view: &View) -> String {
        "spend".into()
    }

    fn status(&self, view: &View) -> String {
        view.spend_label()
    }

    fn legend(&self, _view: &View) -> String {
        match &self.exported {
            Some(p) => format!("wrote {p} · esc"),
            None => "e export csv · ↑↓ scroll · esc".into(),
        }
    }

    fn size(&self, view: &View) -> (u16, u16) {
        let n = view.spend_rows_role.len() + view.spend_rows_conn.len();
        (78, (n + 12).min(28) as u16)
    }

    fn render(&self, view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let h = usize::from(height).max(1);
        let mut lines: Vec<Line<'static>> = Vec::new();
        // Total with budget gauge (R-POP-43).
        lines.push(Line::from(vec![
            ratatui::text::Span::styled(" total  ", theme.panel_muted()),
            ratatui::text::Span::styled(
                view.spend_label(),
                theme.panel().add_modifier(ratatui::style::Modifier::BOLD),
            ),
        ]));
        if view.budget_usd > 0.0 {
            // Same rule as the sidebar card: with no priced turn yet there is
            // no honest percentage, and `$0.00` would claim one (`RYTER.md`).
            let (frac, color, label) = match view.spend {
                Some(spent) => {
                    let frac = spent / view.budget_usd;
                    let color = if spent >= view.budget_usd {
                        theme.error
                    } else if spent >= view.warn_usd {
                        theme.warn
                    } else {
                        theme.success
                    };
                    (
                        frac,
                        color,
                        format!(
                            "{}%  {} of {}",
                            (frac * 100.0).round().min(999.0) as u32,
                            format_usd(Some(spent)),
                            format_usd(Some(view.budget_usd))
                        ),
                    )
                }
                None => (
                    0.0,
                    theme.dim,
                    format!(
                        "?%  {} of {}",
                        format_usd(None),
                        format_usd(Some(view.budget_usd))
                    ),
                ),
            };
            lines.push(widgets::gauge(
                "budget",
                frac,
                &label,
                w.saturating_sub(40).clamp(12, 24),
                theme.panel_bg,
                theme,
                color,
            ));
        }
        if view.budget_usd <= 0.0 {
            lines.push(widgets::colored(
                "budget off · nothing stops on cost · /budget <amount> sets a cap",
                theme.dim,
                theme,
            ));
        }
        if view.unpriced_calls > 0 {
            lines.push(widgets::colored(
                &format!(
                    "unpriced: {} calls  (no rate for the model · set [prices] in settings.toml)",
                    view.unpriced_calls
                ),
                theme.warn,
                theme,
            ));
        }
        lines.push(widgets::blank(theme));
        lines.push(widgets::note("by role", theme));
        if view.spend_rows_role.is_empty() {
            lines.push(widgets::note("  nothing spent yet", theme));
        } else {
            lines.extend(widgets::table(
                &HEADERS,
                &rows(&view.spend_rows_role),
                &ALIGNS,
                None,
                w,
                theme,
            ));
        }
        lines.push(widgets::blank(theme));
        lines.push(widgets::note("by connection", theme));
        if view.spend_rows_conn.is_empty() {
            lines.push(widgets::note("  nothing spent yet", theme));
        } else {
            lines.extend(widgets::table(
                &HEADERS,
                &rows(&view.spend_rows_conn),
                &ALIGNS,
                None,
                w,
                theme,
            ));
        }
        let total = lines.len();
        let first = self.scroll.min(total.saturating_sub(h));
        let lines: Vec<Line<'static>> = lines.into_iter().skip(first).take(h).collect();
        Body {
            lines,
            scroll: (total > h).then_some((first, total)),
        }
    }

    fn key(&mut self, key: KeyEvent, _view: &mut View) -> Outcome {
        match key.code {
            KeyCode::Esc => Outcome::Close,
            KeyCode::Char('e') => Outcome::Act(Action::ExportSpend),
            KeyCode::Up => {
                self.scroll = self.scroll.saturating_sub(1);
                Outcome::Stay
            }
            KeyCode::Down => {
                self.scroll += 1;
                Outcome::Stay
            }
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_sub(8);
                Outcome::Stay
            }
            KeyCode::PageDown => {
                self.scroll += 8;
                Outcome::Stay
            }
            _ => Outcome::Stay,
        }
    }

    fn on_notice(&mut self, n: &Notice, _view: &mut View) {
        if let Notice::Exported(p) = n {
            self.exported = Some(p.clone());
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}
