//! `/models` — model picker (`R-POP-24..29`).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::text::{Line, Span};
use ryter_core::{ModelInfo, format_rates, format_tokens};

use super::{Body, Notice, Outcome, Panel, widgets};
use crate::action::Action;
use crate::activity::SPINNER;
use crate::chat::{short_model, wrap};
use crate::theme::Theme;
use crate::view::{CREW_ROLES, View};

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
        // OpenRouter lists its routers (openrouter/auto, …) at -1: "varies".
        // Shown raw it read "$-1000000" and sorted as the cheapest model.
        Some(p) if p < 0.0 => "varies".into(),
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

/// Choosing who gives second opinions (`/second`): a model from the live
/// catalog, then the most one review may spend. Nothing is preselected;
/// Ryter shows prices and facts, and the user chooses.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewPick {
    /// Tokens the review starts with, to price each model for it.
    pub context_tokens: u64,
    /// Run the review once chosen.
    pub then_run: bool,
    /// The model chosen; the limit is typed next.
    pub chosen: Option<(String, String)>,
}

/// Model picker.
#[derive(Debug, Clone)]
pub struct Models {
    /// Catalog rows (row 0 is the `default` sentinel in crew mode).
    pub items: Vec<ModelInfo>,
    /// Waiting for `ModelsListed`.
    pub loading: bool,
    /// The tab on screen: `None` is the lead (the solo model in solo
    /// mode), `Some(role)` a crew role. `←→` moves between them.
    pub assign_role: Option<String>,
    /// Every connection's models were asked for (a role tab lists them all;
    /// the lead's tab lists its connection's).
    crew_listed: bool,
    /// `Some` when choosing the second-opinion reviewer.
    pub review: Option<ReviewPick>,
    /// Why the last Enter did nothing.
    refusal: Option<String>,
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
        let _ = kind;
        // Every tab's rows: the lead's tab shows its connection's, a role's
        // tab all of them and `default` (follows the lead).
        let mut items = vec![default_row(&view.connection)];
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
        view.composer.clear();
        let mut p = Self {
            items,
            loading: true,
            crew_listed: assign_role.is_some(),
            assign_role,
            review: None,
            refusal: None,
            selected: 0,
            sort: Sort::Relevance,
        };
        p.select_current(view);
        p
    }

    /// Open to choose the second-opinion reviewer, from every connection
    /// with a key, priced for the review at hand.
    pub fn for_review(view: &mut View, context_tokens: u64, then_run: bool) -> Self {
        let mut p = Self::new(view, Some(String::new()));
        p.items.retain(|m| !m.id.is_empty());
        p.assign_role = None;
        p.review = Some(ReviewPick {
            context_tokens,
            then_run,
            chosen: None,
        });
        p.sort = Sort::Price;
        p
    }

    fn local(view: &View, m: &ModelInfo) -> bool {
        let conn = m.connection.as_deref().unwrap_or(&view.connection);
        view.connections
            .iter()
            .any(|c| c.name == conn && c.kind == "local")
    }

    /// What reviewing the work at hand with `m` should cost.
    fn review_price(view: &View, m: &ModelInfo, context_tokens: u64) -> Option<(f64, f64)> {
        if Self::local(view, m) {
            return Some((0.0, 0.0));
        }
        let rates = match (m.input_per_million, m.output_per_million) {
            (Some(i), Some(o)) if i >= 0.0 && o >= 0.0 => {
                Some(ryter_core::Rates::per_million(i, o))
            }
            _ => None,
        };
        ryter_core::second::price_range(rates, context_tokens.max(2_000))
    }

    fn filtered(&self, view: &View) -> Vec<&ModelInfo> {
        let f = view.composer.text().trim().to_ascii_lowercase();
        let mut v: Vec<&ModelInfo> = self
            .items
            .iter()
            .filter(|m| {
                // A second opinion from the model doing the work isn't one,
                // and a reviewer reads files and runs tests through tools.
                if self.review.is_some()
                    && (ryter_core::crew::same_model(&m.id, &view.model) || m.tools == Some(false))
                {
                    return false;
                }
                // The lead's tab: its connection's models, no `default`.
                if self.review.is_none() && self.assign_role.is_none() {
                    if m.id.is_empty() {
                        return false;
                    }
                    if m.connection
                        .as_deref()
                        .is_some_and(|c| c != view.connection)
                    {
                        return false;
                    }
                }
                // `default` stays unless the filter rules it out: typing a
                // model's name and pressing enter picked `default` above it.
                if m.id.is_empty() {
                    return f.is_empty() || "default follows the lead".contains(&f);
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
                // Unknown and "varies" (negative) go last, not first.
                let known = |p: Option<f64>| p.filter(|p| *p >= 0.0).unwrap_or(f64::MAX);
                let pa = known(a.input_per_million);
                let pb = known(b.input_per_million);
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
            if self.review.is_none() {
                v.push(default_row(&view.connection));
            }
            v.extend(models.iter().cloned());
            self.items = v;
            self.selected = self.selected.min(self.items.len().saturating_sub(1));
        }
        self.loading = false;
        self.select_current(view);
    }

    /// Put the cursor on the model in use, as the list opens: with hundreds
    /// in a catalog, starting at the top meant scrolling to find it.
    fn select_current(&mut self, view: &View) {
        if self.review.is_some() || !view.composer.text().is_empty() {
            return;
        }
        let now = match &self.assign_role {
            None => view.model.clone(),
            // A role that follows the lead is on the `default` row.
            Some(role) => view
                .specialists
                .get(role)
                .filter(|r| r.is_override())
                .and_then(|r| r.model.clone())
                .unwrap_or_default(),
        };
        if let Some(i) = self.filtered(view).iter().position(|m| m.id == now) {
            self.selected = i;
        }
    }

    /// The tabs: the lead (the solo model in solo mode), then the roles.
    fn tabs(view: &View) -> Vec<(Option<&'static str>, String)> {
        let lead = if view.crew_mode() { "Lead" } else { "Solo" };
        let mut v = vec![(None, lead.to_string())];
        for r in CREW_ROLES {
            let mut label = r.to_string();
            label[..1].make_ascii_uppercase();
            v.push((Some(*r), label));
        }
        v
    }

    fn tab(&self) -> usize {
        match &self.assign_role {
            None => 0,
            Some(r) => CREW_ROLES.iter().position(|c| c == r).map_or(0, |i| i + 1),
        }
    }

    /// `←→`: the next or previous tab. The first move to a role asks for
    /// every connection's models.
    fn switch(&mut self, view: &View, forward: bool) -> Outcome {
        let n = CREW_ROLES.len() + 1;
        let t = if forward {
            (self.tab() + 1) % n
        } else {
            (self.tab() + n - 1) % n
        };
        self.assign_role = (t > 0).then(|| CREW_ROLES[t - 1].to_string());
        self.selected = 0;
        self.refusal = None;
        self.select_current(view);
        match &self.assign_role {
            Some(role) if !self.crew_listed => {
                self.crew_listed = true;
                self.loading = true;
                Outcome::Act(Action::ListCrewModels { role: role.clone() })
            }
            _ => Outcome::Stay,
        }
    }

    /// Which seats use `id`: `lead`, `arch`, `build`, `audit`.
    fn used_by(view: &View, id: &str) -> String {
        let mut who = Vec::new();
        if id == view.model {
            who.push("lead");
        }
        for (role, short) in [
            ("architect", "arch"),
            ("builder", "build"),
            ("auditor", "audit"),
        ] {
            let own = view
                .specialists
                .get(role)
                .filter(|r| r.is_override())
                .and_then(|r| r.model.as_deref());
            if own == Some(id) {
                who.push(short);
            }
        }
        who.join(" ")
    }

    /// The tabs row and what the tab's seat runs on now.
    fn header(&self, view: &View, theme: Theme) -> Vec<Line<'static>> {
        let on = self.tab();
        let mut spans = vec![Span::styled(" ", theme.panel_muted())];
        for (i, (_, label)) in Self::tabs(view).into_iter().enumerate() {
            if i == on {
                spans.push(Span::styled(
                    format!("‹ {label} ›"),
                    theme
                        .on_panel(theme.accent)
                        .add_modifier(ratatui::style::Modifier::BOLD),
                ));
            } else {
                spans.push(Span::styled(format!("  {label}  "), theme.panel_muted()));
            }
            spans.push(Span::styled(" ", theme.panel_muted()));
        }
        spans.push(Span::styled("  ←→", theme.on_panel(theme.accent)));
        let now = match &self.assign_role {
            None => view.model.clone(),
            Some(role) => match view
                .specialists
                .get(role)
                .filter(|r| r.is_override())
                .and_then(|r| r.model.clone())
            {
                Some(m) => m,
                None => format!("follows the lead ({})", short_model(&view.model)),
            },
        };
        vec![
            Line::from(spans),
            Line::from(vec![
                Span::styled(" now: ", theme.panel_muted()),
                Span::styled(now, theme.on_panel(theme.fg)),
            ]),
        ]
    }
}

fn default_row(connection: &str) -> ModelInfo {
    ModelInfo {
        id: String::new(),
        context_length: None,
        input_per_million: None,
        output_per_million: None,
        connection: Some(connection.to_string()),
        created: None,
        tools: None,
    }
}

impl Panel for Models {
    fn kind(&self) -> &'static str {
        "models"
    }

    fn title(&self, _view: &View) -> String {
        match (&self.review, &self.assign_role) {
            (
                Some(ReviewPick {
                    chosen: Some(_), ..
                }),
                _,
            ) => "audit · your limit".into(),
            (Some(_), _) => "audit · who audits?".into(),
            (None, _) => "models".into(),
        }
    }

    fn status(&self, view: &View) -> String {
        let n = self.filtered(view).len();
        if let Some(note) = &view.models_note {
            format!("{n} · {note}")
        } else if self.loading {
            format!("{n} · loading")
        } else {
            format!("{n} · sort {}", self.sort.label())
        }
    }

    fn legend(&self, _view: &View) -> String {
        match &self.review {
            Some(ReviewPick {
                chosen: Some(_), ..
            }) => "type dollars · enter save · esc back".into(),
            Some(_) => "↑↓ move · enter choose · s sort · esc close".into(),
            None => {
                "↑↓ move · enter set · ←→ role · tab reasoning · b guided setup · s sort · esc close"
                    .into()
            }
        }
    }

    fn inline_input(&self) -> bool {
        true
    }

    fn input(&self, _view: &View) -> Option<String> {
        match &self.review {
            Some(ReviewPick {
                chosen: Some(_), ..
            }) => Some("limit in dollars".into()),
            _ => Some("filter".into()),
        }
    }

    fn render(&self, view: &View, width: u16, height: u16, theme: Theme) -> Body {
        if let Some(r) = &self.review {
            return self.render_review(r, view, width, height, theme);
        }
        let w = usize::from(width);
        let h = usize::from(height);
        let rows_h = h.saturating_sub(5).max(1); // tabs, now, header, footer
        let list = self.filtered(view);
        let n = list.len();
        let sel = self.selected.min(n.saturating_sub(1));
        let first = super::window(sel, n, rows_h);
        let mut lines: Vec<Line<'static>> = self.header(view, theme);
        let rows: Vec<Vec<String>> = list
            .iter()
            .skip(first)
            .take(rows_h)
            .map(|m| {
                if m.id.is_empty() {
                    return vec![
                        "default (follows the lead)".into(),
                        String::new(),
                        String::new(),
                        String::new(),
                        String::new(),
                        m.connection.clone().unwrap_or_default(),
                        String::new(),
                    ];
                }
                vec![
                    m.id.clone(),
                    m.context_length
                        .map(format_tokens)
                        .unwrap_or_else(|| "?".into()),
                    price(m.input_per_million),
                    price(m.output_per_million),
                    view.reasoning_label(&m.id).to_string(),
                    m.connection
                        .clone()
                        .unwrap_or_else(|| view.connection.clone()),
                    Self::used_by(view, &m.id),
                ]
            })
            .collect();
        // Models that can't call tools can't read a file or run a test:
        // listed, but dimmed.
        let toolless: Vec<bool> = list
            .iter()
            .skip(first)
            .take(rows_h)
            .map(|m| m.tools == Some(false))
            .collect();
        let mut table = widgets::table(
            &[
                "model",
                "context",
                "in/M",
                "out/M",
                "reasoning",
                "connection",
                "used by",
            ],
            &rows,
            &[
                widgets::Al::L,
                widgets::Al::R,
                widgets::Al::R,
                widgets::Al::R,
                widgets::Al::L,
                widgets::Al::L,
                widgets::Al::L,
            ],
            Some(sel.saturating_sub(first)),
            w,
            theme,
        );
        for (i, off) in toolless.iter().enumerate() {
            if *off && i != sel.saturating_sub(first) {
                if let Some(line) = table.get_mut(i + 1) {
                    for span in &mut line.spans {
                        span.style = span.style.fg(theme.dim);
                    }
                }
            }
        }
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
                format!("follows the lead: {} · {}", view.model, view.connection)
            } else {
                let rates = match (m.input_per_million, m.output_per_million) {
                    // A router's price varies by where it routes (listed as
                    // -1): it read "$-1000000/M input".
                    (Some(i), Some(o)) if i < 0.0 || o < 0.0 => "price varies by route".into(),
                    (Some(i), Some(o)) => format_rates(Some(ryter_core::Rates::per_million(i, o))),
                    _ => "price unknown".into(),
                };
                format!(
                    "{} · {} · {rates} · reasoning {}",
                    m.id,
                    m.context_length
                        .map(|c| format!("{} ctx", format_tokens(c)))
                        .unwrap_or_else(|| "ctx ?".into()),
                    match view.reasoning_label(&m.id) {
                        "auto" => format!(
                            "auto ({} in {})",
                            view.reasoning_effective(view.mode, &m.id),
                            view.mode_label()
                        ),
                        l => l.to_string(),
                    }
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
        if self.review.is_some() {
            if let Some(out) = self.review_key(key, view) {
                return out;
            }
        }
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
            KeyCode::Left => self.switch(view, false),
            KeyCode::Right => self.switch(view, true),
            // The guided crew setup, one key away.
            KeyCode::Char('b') if view.composer.is_empty() => Outcome::PushAct(
                Box::new(super::crew_builder::CrewBuilder::new(view, false)),
                Action::ListCrewModels {
                    role: String::new(),
                },
            ),
            // Tab / Shift+Tab: how hard this model reasons, wherever it runs.
            KeyCode::Tab | KeyCode::BackTab => {
                let Some(m) = self
                    .filtered(view)
                    .get(self.selected.min(n.saturating_sub(1)))
                    .map(|m| m.id.clone())
                    .filter(|id| !id.is_empty())
                else {
                    return Outcome::Stay;
                };
                let level = ryter_core::config::cycle_reasoning(
                    view.model_reasoning.get(&m).map(String::as_str),
                    key.code == KeyCode::Tab,
                );
                Outcome::Act(Action::SetModelReasoning { model: m, level })
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

    fn size(&self, _view: &View) -> (u16, u16) {
        if self.review.is_some() {
            (96, 20)
        } else {
            (104, 22)
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

impl Models {
    fn render_review(
        &self,
        r: &ReviewPick,
        view: &View,
        width: u16,
        height: u16,
        theme: Theme,
    ) -> Body {
        let w = usize::from(width);
        let h = usize::from(height);
        let mut lines: Vec<Line<'static>> = Vec::new();
        if let Some((connection, model)) = &r.chosen {
            let price = self
                .items
                .iter()
                .find(|m| &m.id == model)
                .and_then(|m| Self::review_price(view, m, r.context_tokens))
                .map(ryter_core::second::format_range)
                .unwrap_or_else(|| "$?.??".into());
            for t in [
                format!("{model} on {connection}"),
                format!("this audit: about {price}"),
                String::new(),
                "The most one audit may spend, in dollars. Each audit asks before it".into(),
                "runs and shows its estimate against this. Near the limit the auditor".into(),
                "writes up what it has; a step that would pass it is never sent.".into(),
                String::new(),
                "A large change audited by a strong model can cost $20 or more.".into(),
            ] {
                lines.push(widgets::text(
                    &wrap::truncate(&t, w.saturating_sub(2)),
                    theme,
                ));
            }
            if let Some(why) = &self.refusal {
                lines.push(widgets::blank(theme));
                lines.push(widgets::colored(why, theme.warn, theme));
            }
            return Body {
                lines,
                scroll: None,
            };
        }
        let intro = if r.then_run {
            "Choose who audits your work. Nothing is preselected: prices are for this audit."
        } else {
            "Choose again. Prices are for the work there is to audit now."
        };
        lines.push(widgets::note(
            &wrap::truncate(intro, w.saturating_sub(2)),
            theme,
        ));
        let rows_h = h.saturating_sub(4).max(1);
        let list = self.filtered(view);
        let n = list.len();
        let sel = self.selected.min(n.saturating_sub(1));
        let first = super::window(sel, n, rows_h);
        let rows: Vec<Vec<String>> = list
            .iter()
            .skip(first)
            .take(rows_h)
            .map(|m| {
                let cost = match Self::review_price(view, m, r.context_tokens) {
                    Some(_) if Self::local(view, m) => "$0 · local".into(),
                    Some(range) => ryter_core::second::format_range(range),
                    None => "price unknown".into(),
                };
                vec![
                    m.id.clone(),
                    cost,
                    price(m.input_per_million),
                    price(m.output_per_million),
                    m.connection
                        .clone()
                        .unwrap_or_else(|| view.connection.clone()),
                ]
            })
            .collect();
        lines.extend(widgets::table(
            &["model", "this audit", "in/M", "out/M", "connection"],
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
        ));
        if self.loading && n <= 1 {
            let frame = SPINNER[(view.now_ms / 80) as usize % SPINNER.len()];
            lines.push(Line::from(vec![
                Span::styled(format!(" {frame} "), theme.on_panel(theme.accent)),
                Span::styled(
                    "loading every catalog you have a key for…",
                    theme.panel_muted(),
                ),
            ]));
        }
        while lines.len() < h.saturating_sub(1) {
            lines.push(widgets::blank(theme));
        }
        lines.truncate(h.saturating_sub(1));
        // Facts about the highlighted model, not opinions of it.
        let footer = if let Some(why) = &self.refusal {
            why.clone()
        } else if let Some(m) = list.get(sel) {
            let conn = m.connection.as_deref().unwrap_or(&view.connection);
            let same = ryter_core::tiering::family(conn, &m.id)
                == ryter_core::tiering::family(&view.connection, &view.model);
            match Self::review_price(view, m, r.context_tokens) {
                None => format!("{}: no price known, so no limit can hold it", m.id),
                Some(_) if same => format!(
                    "{}: same vendor as {}, so a less independent opinion",
                    m.id,
                    short_model(&view.model)
                ),
                Some(_) => format!(
                    "{}: a different vendor from {}",
                    m.id,
                    short_model(&view.model)
                ),
            }
        } else {
            String::new()
        };
        lines.push(widgets::note(
            &wrap::truncate(&footer, w.saturating_sub(2)),
            theme,
        ));
        Body {
            lines,
            scroll: (n > rows_h).then_some((first, n)),
        }
    }

    /// Keys that mean something else while choosing a reviewer; `None`
    /// falls through to the list's own keys.
    fn review_key(&mut self, key: KeyEvent, view: &mut View) -> Option<Outcome> {
        let r = self.review.clone()?;
        if let Some((connection, model)) = r.chosen {
            return Some(match key.code {
                KeyCode::Esc => {
                    if let Some(p) = self.review.as_mut() {
                        p.chosen = None;
                    }
                    self.refusal = None;
                    view.composer.clear();
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    let typed = view
                        .composer
                        .text()
                        .trim()
                        .trim_start_matches('$')
                        .to_string();
                    match typed.parse::<f64>() {
                        Ok(limit_usd) if limit_usd > 0.0 && limit_usd.is_finite() => {
                            view.composer.clear();
                            Outcome::CloseAct(Action::SetReviewer {
                                connection,
                                model,
                                limit_usd,
                                then_run: r.then_run,
                            })
                        }
                        _ => {
                            self.refusal = Some("type an amount in dollars, like 5 or 0.50".into());
                            Outcome::Stay
                        }
                    }
                }
                _ => {
                    super::edit_field(&mut view.composer, key);
                    self.refusal = None;
                    Outcome::Stay
                }
            });
        }
        match key.code {
            KeyCode::Tab | KeyCode::BackTab => Some(Outcome::Stay),
            KeyCode::Enter => {
                let list = self.filtered(view);
                let m = (*list.get(self.selected.min(list.len().saturating_sub(1)))?).clone();
                if Self::review_price(view, &m, r.context_tokens).is_none() {
                    self.refusal = Some(format!(
                        "{} has no known price, so no limit can hold it: choose another, or add it under [pricing]",
                        m.id
                    ));
                    return Some(Outcome::Stay);
                }
                let connection = m
                    .connection
                    .clone()
                    .unwrap_or_else(|| view.connection.clone());
                if let Some(p) = self.review.as_mut() {
                    p.chosen = Some((connection, m.id));
                }
                self.refusal = None;
                view.composer.clear();
                Some(Outcome::Stay)
            }
            _ => {
                self.refusal = None;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn row(id: &str, conn: &str, rates: Option<(f64, f64)>) -> ModelInfo {
        ModelInfo {
            id: id.into(),
            context_length: Some(200_000),
            input_per_million: rates.map(|r| r.0),
            output_per_million: rates.map(|r| r.1),
            connection: Some(conn.into()),
            created: None,
            tools: Some(true),
        }
    }

    fn key(p: &mut Models, v: &mut View, code: KeyCode) -> Outcome {
        p.key(KeyEvent::new(code, KeyModifiers::NONE), v)
    }

    /// `/models` is where every model is chosen: `←→` moves from the lead to
    /// each crew role, the first move asks for every connection's models,
    /// and `⏎` sets the model of the role on screen. It used to take `/crew`
    /// twice to reach a role's model.
    #[test]
    fn models_has_a_tab_per_role() {
        let mut v = View::new(
            ryter_core::Phase::Build,
            "openrouter".into(),
            "x-ai/grok-4.7".into(),
            "/tmp".into(),
        );
        v.specialists.insert(
            "auditor".into(),
            ryter_core::RoleModel {
                connection: Some("openrouter".into()),
                model: Some("qwen/qwen3.7-max".into()),
            },
        );
        let mut p = Models::new(&mut v, None);
        let models = [
            row("x-ai/grok-4.7", "openrouter", Some((2.0, 6.0))),
            row("qwen/qwen3.7-max", "openrouter", Some((1.6, 6.4))),
            row("minimax/minimax-m2.7", "openrouter", Some((0.3, 1.2))),
            row("grok-4.6", "spacexai", Some((2.0, 6.0))),
        ];
        p.set_models(&v, &models);
        let text = |p: &Models, v: &View| {
            p.render(v, 104, 20, Theme::truecolor_dark())
                .lines
                .iter()
                .map(|l| {
                    l.spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        // The lead's tab: its connection's models, no `default`, who uses what.
        let t = text(&p, &v);
        assert!(
            t.contains("‹ Solo ›") && t.contains("now: x-ai/grok-4.7"),
            "{t}"
        );
        assert!(!t.contains("grok-4.6") && !t.contains("default"), "{t}");
        assert!(t.contains("audit"), "{t}");
        // → the architect: every connection's models are asked for once.
        assert!(matches!(
            key(&mut p, &mut v, KeyCode::Right),
            Outcome::Act(Action::ListCrewModels { role }) if role == "architect"
        ));
        let t = text(&p, &v);
        assert!(
            t.contains("‹ Architect ›") && t.contains("now: follows the lead (grok-4.7)"),
            "{t}"
        );
        // → the builder: the list is already here.
        assert!(matches!(key(&mut p, &mut v, KeyCode::Right), Outcome::Stay));
        p.set_models(&v, &models);
        assert!(
            text(&p, &v).contains("grok-4.6"),
            "all connections for a role"
        );
        // ⏎ sets the builder, not the lead.
        v.composer.set_text("minimax");
        let out = key(&mut p, &mut v, KeyCode::Enter);
        assert!(matches!(
            &out,
            Outcome::CloseAct(Action::SetCrewRole { role, model, .. })
                if role == "builder" && model == "minimax/minimax-m2.7"
        ));
        // ← from the lead wraps to the auditor, which opens on its model.
        let mut p = Models::new(&mut v, None);
        p.set_models(&v, &models);
        key(&mut p, &mut v, KeyCode::Left);
        let t = text(&p, &v);
        assert!(
            t.contains("‹ Auditor ›") && t.contains("now: qwen/qwen3.7-max"),
            "{t}"
        );
    }

    /// The chooser: every catalog model but the one doing the work, each
    /// priced for this review; no price, no choice; then the user's limit.
    #[test]
    fn choosing_a_reviewer_is_the_users_choice_with_prices_and_a_limit() {
        let mut v = View::new(
            ryter_core::Phase::Build,
            "openrouter".into(),
            "deepseek/deepseek-v4.1-flash".into(),
            "/tmp".into(),
        );
        let mut p = Models::for_review(&mut v, 20_000, true);
        p.set_models(
            &v,
            &[
                row(
                    "deepseek/deepseek-v4.1-flash",
                    "openrouter",
                    Some((0.1, 0.6)),
                ),
                row("x-ai/grok-4.7", "openrouter", Some((3.0, 15.0))),
                row("mystery/unpriced", "openrouter", None),
            ],
        );
        let ids: Vec<String> = p.filtered(&v).iter().map(|m| m.id.clone()).collect();
        assert_eq!(
            ids,
            vec!["x-ai/grok-4.7", "mystery/unpriced"],
            "cheapest first, own model hidden"
        );
        let text = |p: &Models, v: &View| -> String {
            p.render(v, 96, 16, Theme::truecolor_dark())
                .lines
                .iter()
                .map(|l| {
                    l.spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect::<String>()
                        + "\n"
                })
                .collect()
        };
        let shown = text(&p, &v);
        assert!(
            shown.contains("this audit") && shown.contains("$0."),
            "{shown}"
        );
        assert!(shown.contains("price unknown"), "{shown}");
        // No price: refused, and said why.
        p.selected = 1;
        assert!(matches!(key(&mut p, &mut v, KeyCode::Enter), Outcome::Stay));
        assert!(text(&p, &v).contains("no known price"));
        // A priced model: on to the limit, which nobody preset.
        p.selected = 0;
        assert!(matches!(key(&mut p, &mut v, KeyCode::Enter), Outcome::Stay));
        assert_eq!(p.input(&v).as_deref(), Some("limit in dollars"));
        assert!(v.composer.is_empty());
        assert!(
            matches!(key(&mut p, &mut v, KeyCode::Enter), Outcome::Stay),
            "no amount, no save"
        );
        for c in "$7.5".chars() {
            key(&mut p, &mut v, KeyCode::Char(c));
        }
        match key(&mut p, &mut v, KeyCode::Enter) {
            Outcome::CloseAct(Action::SetReviewer {
                connection,
                model,
                limit_usd,
                then_run,
            }) => {
                assert_eq!(
                    (connection.as_str(), model.as_str()),
                    ("openrouter", "x-ai/grok-4.7")
                );
                assert!((limit_usd - 7.5).abs() < 1e-9 && then_run);
            }
            _ => panic!("the choice is saved"),
        }
    }

    #[test]
    fn a_varying_price_reads_varies_not_minus_a_million() {
        assert_eq!(price(Some(-1_000_000.0)), "varies");
        assert_eq!(price(Some(0.37)), "$0.37");
        assert_eq!(price(None), "?");
    }

    /// Tab steps the highlighted model through the reasoning choices, and
    /// the list shows each model's choice.
    #[test]
    fn tab_sets_the_highlighted_models_reasoning() {
        let mut v = View::new(
            ryter_core::Phase::Build,
            "openrouter".into(),
            "z-ai/glm-5.3-flashx".into(),
            "/tmp".into(),
        );
        let mut p = Models::new(&mut v, None);
        p.items = vec![ModelInfo {
            id: "z-ai/glm-5.3-flashx".into(),
            context_length: Some(1_000_000),
            input_per_million: Some(0.37),
            output_per_million: Some(1.25),
            connection: Some("openrouter".into()),
            created: None,
            tools: Some(true),
        }];
        p.loading = false;
        let tab = |p: &mut Models, v: &mut View| {
            p.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), v)
        };
        match tab(&mut p, &mut v) {
            Outcome::Act(Action::SetModelReasoning { model, level }) => {
                assert_eq!(model, "z-ai/glm-5.3-flashx");
                assert_eq!(level.as_deref(), Some("low"), "auto → low");
                v.model_reasoning.insert(model, "low".into());
            }
            _ => panic!("tab must set the reasoning level"),
        }
        match tab(&mut p, &mut v) {
            Outcome::Act(Action::SetModelReasoning { level, .. }) => {
                assert_eq!(level.as_deref(), Some("medium"))
            }
            _ => panic!("tab again"),
        }
        let text: String = p
            .render(&v, 100, 12, Theme::truecolor_dark())
            .lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
                    + "\n"
            })
            .collect();
        assert!(text.contains("reasoning") && text.contains("low"), "{text}");
    }
}
