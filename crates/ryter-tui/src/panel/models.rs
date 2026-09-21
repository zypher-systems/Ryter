//! `/models` — model picker (`R-POP-24..29`).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::text::{Line, Span};
use ryter_core::{ModelInfo, format_rates, format_tokens};

use super::{Body, Notice, Outcome, Panel, widgets};
use crate::action::Action;
use crate::activity::SPINNER;
use crate::chat::{short_model, wrap};
use crate::theme::Theme;
use crate::view::View;

/// Sort order (`R-POP-26`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    /// Filter relevance / catalog order.
    Relevance,
    /// Id.
    Name,
    /// Context window, descending.
    Context,
    /// Input price, ascending.
    Price,
}

impl Sort {
    fn next(self) -> Self {
        match self {
            Sort::Relevance => Sort::Name,
            Sort::Name => Sort::Context,
            Sort::Context => Sort::Price,
            Sort::Price => Sort::Relevance,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Sort::Relevance => "relevance",
            Sort::Name => "name",
            Sort::Context => "context",
            Sort::Price => "price",
        }
    }
}

fn price(v: Option<f64>) -> String {
    match v {
        Some(p) => format!("${}", trim(p)),
        None => "?".into(),
    }
}

fn trim(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() {
        "0".into()
    } else {
        s.to_string()
    }
}

/// Model picker.
#[derive(Debug, Clone)]
pub struct Models {
    /// Catalog rows (row 0 is the `default` sentinel in crew mode).
    pub items: Vec<ModelInfo>,
    /// Waiting for `ModelsListed`.
    pub loading: bool,
    /// `Some(role)` when opened from `/crew` (`R-POP-28`).
    pub assign_role: Option<String>,
    selected: usize,
    sort: Sort,
}

impl Models {
    /// Open for the active connection (or a crew role).
    pub fn new(view: &mut View, assign_role: Option<String>) -> Self {
        let kind = view
            .connections
            .iter()
            .find(|c| c.name == view.connection)
            .map(|c| c.kind.clone())
            .unwrap_or_default();
        let mut items = Vec::new();
        if assign_role.is_some() {
            items.push(default_row(&view.connection));
            for c in view.connections.clone() {
                if !c.has_key && c.name != view.connection {
                    continue;
                }
                let mut fb = crate::view::fallback_models(&c.kind, &c.model);
                for m in &mut fb {
                    m.connection = Some(c.name.clone());
                }
                items.extend(fb);
            }
        } else {
            items.extend(crate::view::fallback_models(&kind, &view.model));
        }
        view.composer.clear();
        Self {
            items,
            loading: true,
            assign_role,
            selected: 0,
            sort: Sort::Relevance,
        }
    }

    fn filtered(&self, view: &View) -> Vec<&ModelInfo> {
        let f = view.composer.text().trim().to_ascii_lowercase();
        let mut v: Vec<&ModelInfo> = self
            .items
            .iter()
            .filter(|m| {
                if m.id.is_empty() {
                    return true;
                }
                if f.is_empty() {
                    return true;
                }
                m.id.to_ascii_lowercase().contains(&f)
                    || short_model(&m.id).to_ascii_lowercase().contains(&f)
                    || m.connection
                        .as_deref()
                        .is_some_and(|c| c.to_ascii_lowercase().contains(&f))
            })
            .collect();
        match self.sort {
            Sort::Relevance => {}
            Sort::Name => v.sort_by(|a, b| {
                a.id.is_empty()
                    .cmp(&b.id.is_empty())
                    .reverse()
                    .then(a.id.cmp(&b.id))
            }),
            Sort::Context => v.sort_by(|a, b| {
                a.id.is_empty().cmp(&b.id.is_empty()).reverse().then(
                    b.context_length
                        .unwrap_or(0)
                        .cmp(&a.context_length.unwrap_or(0)),
                )
            }),
            Sort::Price => v.sort_by(|a, b| {
                let pa = a.input_per_million.unwrap_or(f64::MAX);
                let pb = b.input_per_million.unwrap_or(f64::MAX);
                a.id.is_empty()
                    .cmp(&b.id.is_empty())
                    .reverse()
                    .then(pa.partial_cmp(&pb).unwrap_or(std::cmp::Ordering::Equal))
            }),
        }
        v
    }

    /// Replace the catalog from a `ModelsListed` event.
    pub fn set_models(&mut self, view: &View, models: &[ModelInfo]) {
        if !models.is_empty() {
            let mut v = Vec::new();
            if self.assign_role.is_some() {
                v.push(default_row(&view.connection));
            }
            v.extend(models.iter().cloned());
            self.items = v;
            self.selected = self.selected.min(self.items.len().saturating_sub(1));
        }
        self.loading = false;
    }
}

fn default_row(connection: &str) -> ModelInfo {
    ModelInfo {
        id: String::new(),
        context_length: None,
        input_per_million: None,
        output_per_million: None,
        connection: Some(connection.to_string()),
    }
}

impl Panel for Models {
    fn kind(&self) -> &'static str {
        "models"
    }

    fn title(&self, _view: &View) -> String {
        match &self.assign_role {
            Some(r) => format!("model for {r}"),
            None => "models".into(),
        }
    }

    fn status(&self, view: &View) -> String {
        let n = self.filtered(view).len();
        if self.loading {
            format!("{n} · loading")
        } else {
            format!("{n} · sort {}", self.sort.label())
        }
    }

    fn legend(&self, _view: &View) -> String {
        "type to filter · ↑↓ move · enter select · s sort · esc".into()
    }

    fn input(&self, _view: &View) -> Option<String> {
        Some("filter".into())
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        (84, 18)
    }

    fn render(&self, view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let h = usize::from(height);
        let rows_h = h.saturating_sub(3).max(1); // header + footer
        let list = self.filtered(view);
        let n = list.len();
        let sel = self.selected.min(n.saturating_sub(1));
        let first = super::window(sel, n, rows_h);
        let mut lines: Vec<Line<'static>> = Vec::new();
        let rows: Vec<Vec<String>> = list
            .iter()
            .skip(first)
            .take(rows_h)
            .map(|m| {
                if m.id.is_empty() {
                    return vec![
                        "default (follows orchestrator)".into(),
                        String::new(),
                        String::new(),
                        String::new(),
                        m.connection.clone().unwrap_or_default(),
                    ];
                }
                let mut id = m.id.clone();
                if m.id == view.model && self.assign_role.is_none() {
                    id.push_str("  ●");
                }
                vec![
                    id,
                    m.context_length
                        .map(format_tokens)
                        .unwrap_or_else(|| "?".into()),
                    price(m.input_per_million),
                    price(m.output_per_million),
                    m.connection
                        .clone()
                        .unwrap_or_else(|| view.connection.clone()),
                ]
            })
            .collect();
        let mut table = widgets::table(
            &["model", "context", "in/M", "out/M", "connection"],
            &rows,
            &[
                widgets::Al::L,
                widgets::Al::R,
                widgets::Al::R,
                widgets::Al::R,
                widgets::Al::L,
            ],
            Some(sel.saturating_sub(first)),
            w,
            theme,
        );
        if self.loading && n <= 1 {
            let frame = SPINNER[(view.now_ms / 80) as usize % SPINNER.len()];
            table.push(Line::from(vec![
                Span::styled(format!(" {frame} "), theme.on_panel(theme.accent)),
                Span::styled("loading models…", theme.panel_muted()),
            ]));
        }
        lines.extend(table);
        while lines.len() < h.saturating_sub(1) {
            lines.push(widgets::blank(theme));
        }
        // Detail footer (R-POP-29).
        if let Some(m) = list.get(sel) {
            let text = if m.id.is_empty() {
                format!(
                    "follows the orchestrator: {} · {}",
                    view.model, view.connection
                )
            } else {
                let rates = match (m.input_per_million, m.output_per_million) {
                    (Some(i), Some(o)) => format_rates(Some(ryter_core::Rates::per_million(i, o))),
                    _ => "price unknown".into(),
                };
                format!(
                    "{} · {} · {rates}",
                    m.id,
                    m.context_length
                        .map(|c| format!("{} ctx", format_tokens(c)))
                        .unwrap_or_else(|| "ctx ?".into())
                )
            };
            lines.truncate(h.saturating_sub(1));
            lines.push(widgets::note(
                &wrap::truncate(&text, w.saturating_sub(2)),
                theme,
            ));
        }
        Body {
            lines,
            scroll: (n > rows_h).then_some((first, n)),
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        let n = self.filtered(view).len();
        match key.code {
            KeyCode::Esc => {
                view.composer.clear();
                Outcome::Close
            }
            KeyCode::Up => {
                self.selected = super::step(self.selected.min(n.saturating_sub(1)), -1, n);
                Outcome::Stay
            }
            KeyCode::Down => {
                self.selected = super::step(self.selected.min(n.saturating_sub(1)), 1, n);
                Outcome::Stay
            }
            KeyCode::PageUp => {
                self.selected = self.selected.saturating_sub(8);
                Outcome::Stay
            }
            KeyCode::PageDown => {
                self.selected = (self.selected + 8).min(n.saturating_sub(1));
                Outcome::Stay
            }
            KeyCode::Char('s') if view.composer.is_empty() => {
                self.sort = self.sort.next();
                Outcome::Stay
            }
            KeyCode::Enter => {
                let Some(m) = self
                    .filtered(view)
                    .get(self.selected.min(n.saturating_sub(1)))
                    .cloned()
                    .cloned()
                else {
                    return Outcome::Stay;
                };
                view.composer.clear();
                match &self.assign_role {
                    Some(role) => {
                        if m.id.is_empty() {
                            Outcome::CloseAct(Action::ResetCrewRole(role.clone()))
                        } else {
                            Outcome::CloseAct(Action::SetCrewRole {
                                role: role.clone(),
                                connection: m.connection.unwrap_or_else(|| view.connection.clone()),
                                model: m.id,
                            })
                        }
                    }
                    None => Outcome::CloseAct(Action::SetModel(m.id)),
                }
            }
            _ => {
                if super::edit_field(&mut view.composer, key) {
                    self.selected = 0;
                }
                Outcome::Stay
            }
        }
    }

    fn on_notice(&mut self, n: &Notice, view: &mut View) {
        if let Notice::Models(models) = n {
            self.set_models(view, models);
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}
