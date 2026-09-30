//! `$` on the ledger: what the work has cost, this turn, this session and
//! this project, side by side, by role, against the budget. A drawer above
//! the composer; `/spend` is the full table.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::{Body, Outcome, Panel};
use crate::action::{Action, PanelId};
use crate::chat::{turn_usd, wrap};
use crate::theme::Theme;
use crate::view::{View, role_label};

/// The spend drawer.
#[derive(Debug, Clone, Default)]
pub struct SpendDrawer;

/// Rows of role bars shown per scope.
const ROLES: usize = 4;

impl Panel for SpendDrawer {
    fn kind(&self) -> &'static str {
        "spend-drawer"
    }

    fn title(&self, _view: &View) -> String {
        "spend".into()
    }

    fn status(&self, view: &View) -> String {
        match &view.project_spend {
            Some(p) if p.unpriced_calls > 0 => format!("{} calls had no price", p.unpriced_calls),
            _ => String::new(),
        }
    }

    fn legend(&self, _view: &View) -> String {
        "b set a budget · ⏎ full table · esc close".into()
    }

    fn size(&self, view: &View) -> (u16, u16) {
        let roles = view.spend_rows_role.len().clamp(1, ROLES);
        let project_roles = view
            .project_spend
            .as_ref()
            .map_or(0, |p| p.by_role.len().clamp(1, ROLES));
        (110, (3 + 2 + roles.max(project_roles) + 2) as u16)
    }

    fn docked(&self) -> bool {
        true
    }

    fn render(&self, view: &View, width: u16, _height: u16, theme: Theme) -> Body {
        let w = usize::from(width).saturating_sub(2);
        let bg = theme.panel_bg;
        let dim = Style::default().fg(theme.dim).bg(bg);
        let fg = Style::default().fg(theme.fg).bg(bg);
        let big = fg.add_modifier(Modifier::BOLD);
        let col = (w / 3).max(20);
        let cell = |s: String, style: Style| {
            let s = wrap::truncate(&s, col.saturating_sub(2));
            let pad = col.saturating_sub(wrap::width(&s));
            vec![Span::styled(s, style), Span::styled(" ".repeat(pad), dim)]
        };
        let row = |cells: Vec<Vec<Span<'static>>>| {
            Line::from(cells.into_iter().flatten().collect::<Vec<_>>())
        };

        let turn = view
            .spend
            .map(|now| turn_usd(now - view.turn_spend_from.unwrap_or(0.0)))
            .unwrap_or_else(|| "$?.??".into());
        let title = if view.session_title.trim().is_empty() {
            "session".to_string()
        } else {
            format!("session · {}", view.session_title)
        };
        // Named for the folder the total is counted in.
        let project = std::path::Path::new(view.project_root.as_deref().unwrap_or(&view.cwd))
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let (project_total, project_note) = match &view.project_spend {
            Some(p) => (
                if p.unpriced_calls > 0 {
                    format!("≥{}", turn_usd(p.total_usd))
                } else {
                    turn_usd(p.total_usd)
                },
                format!(
                    "{} session{}",
                    p.sessions,
                    if p.sessions == 1 { "" } else { "s" }
                ),
            ),
            None => ("$?.??".into(), "not loaded".into()),
        };
        let calls: u32 = view.spend_rows_role.values().map(|r| r.calls).sum();
        let mut lines = vec![
            row(vec![
                cell("this turn".into(), dim),
                cell(title, dim),
                cell(format!("project · {project}"), dim),
            ]),
            row(vec![
                cell(turn, big),
                cell(
                    match view.spend {
                        Some(s) if !view.spend_unknown => turn_usd(s),
                        None if !view.spend_unknown => "$0".into(),
                        _ => view.spend_label(),
                    },
                    big,
                ),
                cell(project_total, big),
            ]),
            row(vec![
                cell(format!("{} tools", view.activity.tools), dim),
                cell(format!("{calls} model calls"), dim),
                cell(project_note, dim),
            ]),
            Line::from(Span::styled(" ".repeat(w), dim)),
        ];

        // By role: the session's and the project's, side by side.
        let bar_w = 12;
        let bars = |pairs: Vec<(String, f64)>| -> Vec<Vec<Span<'static>>> {
            let top = pairs
                .iter()
                .map(|(_, v)| *v)
                .fold(0.0_f64, f64::max)
                .max(1e-9);
            pairs
                .into_iter()
                .take(ROLES)
                .map(|(role, usd)| {
                    let filled = ((usd / top) * bar_w as f64).round() as usize;
                    let name = format!("{:<10}", wrap::truncate(role_label(&role), 10));
                    let money = format!(" {:>8}", turn_usd(usd));
                    let used = 10 + bar_w + wrap::width(&money);
                    vec![
                        Span::styled(name, fg),
                        Span::styled(
                            "▰".repeat(filled.min(bar_w)),
                            Style::default().fg(theme.role(&role)).bg(bg),
                        ),
                        Span::styled("▱".repeat(bar_w - filled.min(bar_w)), dim),
                        Span::styled(money, dim),
                        Span::styled(" ".repeat((w / 2).saturating_sub(used)), dim),
                    ]
                })
                .collect()
        };
        let sort = |mut v: Vec<(String, f64)>| {
            v.sort_by(|a, b| b.1.total_cmp(&a.1));
            v
        };
        let session: Vec<(String, f64)> = sort(
            view.spend_rows_role
                .iter()
                .map(|(r, row)| (r.clone(), row.usd))
                .collect(),
        );
        let project_roles: Vec<(String, f64)> = sort(
            view.project_spend
                .as_ref()
                .map(|p| p.by_role.iter().map(|(r, v)| (r.clone(), *v)).collect())
                .unwrap_or_default(),
        );
        let half = |s: &str| {
            let pad = (w / 2).saturating_sub(wrap::width(s));
            vec![
                Span::styled(s.to_string(), dim),
                Span::styled(" ".repeat(pad), dim),
            ]
        };
        lines.push(Line::from(
            [half("this session, by role"), half("project, by role")].concat(),
        ));
        let left = bars(session);
        let right = bars(project_roles);
        for i in 0..left.len().max(right.len()).max(1) {
            let l = left
                .get(i)
                .cloned()
                .unwrap_or_else(|| half(if i == 0 { "nothing yet" } else { "" }));
            let r = right.get(i).cloned().unwrap_or_else(|| half(""));
            lines.push(Line::from([l, r].concat()));
        }
        lines.push(Line::from(Span::styled(" ".repeat(w), dim)));

        let mut budget = vec![Span::styled("budget ", dim)];
        if view.budget_usd > 0.0 {
            budget.push(Span::styled(format!("${:.2}", view.budget_usd), fg));
            if let Some(s) = view.spend {
                budget.push(Span::styled(
                    format!(" · ${:.2} left", (view.budget_usd - s).max(0.0)),
                    dim,
                ));
            }
        } else {
            budget.push(Span::styled("off", fg));
        }
        if view.task_budget_usd > 0.0 {
            budget.push(Span::styled(
                format!("   task cap ${:.2}", view.task_budget_usd),
                dim,
            ));
        }
        if view.warn_usd > 0.0 {
            budget.push(Span::styled(
                format!("   warn at ${:.2}", view.warn_usd),
                dim,
            ));
        }
        lines.push(Line::from(budget));
        // One column in from the border.
        let lines = lines
            .into_iter()
            .map(|l| {
                let mut spans = vec![Span::styled(" ", dim)];
                spans.extend(l.spans);
                Line::from(spans)
            })
            .collect();
        Body {
            lines,
            scroll: None,
        }
    }

    fn key(&mut self, key: KeyEvent, _view: &mut View) -> Outcome {
        match key.code {
            KeyCode::Esc | KeyCode::Char('$') | KeyCode::Char('q') => Outcome::Close,
            KeyCode::Enter => Outcome::CloseAct(Action::OpenPanel(PanelId::Spend)),
            KeyCode::Char('b') => Outcome::CloseAct(Action::OpenPanel(PanelId::Budget)),
            _ => Outcome::Stay,
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}
