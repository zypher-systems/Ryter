//! `/models` — model picker (`R-POP-24..29`).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::text::{Line, Span};
use ryter_core::{ModelInfo, format_rates, format_tokens};

use super::{Body, Notice, Outcome, Panel, widgets};
use crate::action::Action;
use crate::activity::SPINNER;
use crate::chat::{short_model, wrap};
use crate::theme::Theme;
use crate::view::{HAT_ROLES, View};

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

/// Which side of `/models` the keys move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// The seats: the model every hat uses, then each hat.
    Seats,
    /// The models for the seat chosen.
    Models,
}

/// Model picker: the seats on the left, the models for the chosen seat on
/// the right. Enter sets the model and goes back to the seats, on the next
/// one, so every hat is set without leaving the panel.
#[derive(Debug, Clone)]
pub struct Models {
    /// Catalog rows (row 0 is the `default` sentinel).
    pub items: Vec<ModelInfo>,
    /// Waiting for `ModelsListed`.
    pub loading: bool,
    /// The seat chosen: `None` is the model every hat uses, `Some(hat)`
    /// one hat's own.
    pub assign_role: Option<String>,
    /// Every connection's models were asked for (a hat's list has them
    /// all; the first seat's has its connection's).
    all_listed: bool,
    /// Why the last Enter did nothing.
    refusal: Option<String>,
    selected: usize,
    sort: Sort,
    /// Which side the keys move.
    pub focus: Focus,
    /// Seats set since the panel opened (✓), by seat.
    changed: [bool; SEATS],
}

/// The model the rest follow, and each hat, which can have its own.
const SEATS: usize = HAT_ROLES.len() + 1;

/// The list's width with every column showing.
const LIST_WIDTH: usize = 80;

impl Models {
    /// Open on the seats, or on one hat's models.
    pub fn new(view: &mut View, assign_role: Option<String>) -> Self {
        let kind = view
            .connections
            .iter()
            .find(|c| c.name == view.connection)
            .map(|c| c.kind.clone())
            .unwrap_or_default();
        let _ = kind;
        // Every seat's rows: the first shows its connection's, a hat's all
        // of them and `default` (follows the others).
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
        // Opened on a hat: its models, at once.
        let focus = if assign_role.is_some() {
            Focus::Models
        } else {
            Focus::Seats
        };
        let mut p = Self {
            items,
            loading: true,
            all_listed: assign_role.is_some(),
            assign_role,
            refusal: None,
            selected: 0,
            sort: Sort::Relevance,
            focus,
            changed: [false; SEATS],
        };
        p.select_current(view);
        p
    }

    fn filtered(&self, view: &View) -> Vec<&ModelInfo> {
        let f = view.composer.text().trim().to_ascii_lowercase();
        let mut v: Vec<&ModelInfo> = self
            .items
            .iter()
            .filter(|m| {
                // The first seat: its connection's models, no `default`.
                if self.assign_role.is_none() {
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
                    return f.is_empty() || "default follows all hats".contains(&f);
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
            let mut v = vec![default_row(&view.connection)];
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
        if !view.composer.text().is_empty() {
            return;
        }
        let (model, connection) = match &self.assign_role {
            None => (view.model.clone(), Some(view.connection.clone())),
            // A hat that follows the others is on the `default` row.
            Some(role) => view
                .specialists
                .get(role)
                .filter(|r| r.is_override())
                .map(|r| (r.model.clone().unwrap_or_default(), r.connection.clone()))
                .unwrap_or_default(),
        };
        self.highlight(view, &model, connection.as_deref());
    }

    /// Put the highlight on `model` from `connection`: one model can be on
    /// two connections, and the connection decides which provider runs it.
    /// Matching the id alone put it on the first connection's row, and
    /// enter then moved the seat there.
    fn highlight(&mut self, view: &View, model: &str, connection: Option<&str>) {
        let list = self.filtered(view);
        let same = |m: &&ModelInfo| {
            m.id == model
                && connection
                    .is_none_or(|c| m.connection.as_deref().unwrap_or(&view.connection) == c)
        };
        if let Some(i) = list.iter().position(same) {
            self.selected = i;
        } else if let Some(i) = list.iter().position(|m| m.id == model) {
            self.selected = i;
        }
    }

    /// The seats: the model the rest follow ("All hats"), then each hat.
    fn seats(view: &View) -> Vec<(Option<&'static str>, String)> {
        let mut v = vec![(None, "All hats".to_string())];
        for r in view.seat_roles() {
            let mut label = r.to_string();
            label[..1].make_ascii_uppercase();
            v.push((Some(*r), label));
        }
        v
    }

    /// The seat chosen: 0 is the model every hat uses.
    fn seat(&self) -> usize {
        match &self.assign_role {
            None => 0,
            Some(r) => HAT_ROLES.iter().position(|c| c == r).map_or(0, |i| i + 1),
        }
    }

    /// Choose seat `to`. The first role chosen asks for every connection's
    /// models (the first seat's list is its own connection's).
    fn choose_seat(&mut self, view: &View, to: usize) -> Option<Action> {
        let to = to.min(SEATS - 1);
        self.assign_role = (to > 0).then(|| view.seat_roles()[to - 1].to_string());
        self.selected = 0;
        self.refusal = None;
        self.select_current(view);
        match &self.assign_role {
            Some(role) if !self.all_listed => {
                self.all_listed = true;
                self.loading = true;
                Some(Action::ListAllModels { role: role.clone() })
            }
            _ => None,
        }
    }

    /// What a seat runs on now, short, and whether it follows the others.
    fn seat_model(view: &View, role: Option<&str>) -> (String, bool) {
        match role {
            None => (short_model(&view.model).to_string(), false),
            Some(role) => match view
                .specialists
                .get(role)
                .filter(|r| r.is_override())
                .and_then(|r| r.model.as_deref())
            {
                Some(m) => (short_model(m).to_string(), false),
                None => ("follows all hats".into(), true),
            },
        }
    }

    /// The seats column's width in a body `width` wide.
    /// The list comes first: it gets 80 columns, enough for a long model
    /// name, both prices, and its connection, before the seats take more
    /// than their narrowest.
    fn seats_width(width: usize) -> usize {
        width.saturating_sub(LIST_WIDTH + 1).clamp(22, 34)
    }

    /// The seats column, a line per row of the body.
    fn seat_lines(
        &self,
        view: &View,
        width: usize,
        h: usize,
        theme: Theme,
    ) -> Vec<Vec<Span<'static>>> {
        use ratatui::style::Modifier;
        let on = self.seat();
        let pad = |s: String, w: usize| wrap::pad_right(&wrap::truncate(&s, w), w);
        let mut rows: Vec<Vec<Span<'static>>> = vec![vec![Span::styled(
            pad(" SEATS".into(), width),
            theme.panel_muted().add_modifier(Modifier::BOLD),
        )]];
        // Narrow, each seat's model goes on a line of its own, so its
        // name isn't cut to a few letters.
        let stacked = width < 30;
        for (i, (role, label)) in Self::seats(view).into_iter().enumerate() {
            let here = i == on;
            let cursor = if here { "›" } else { " " };
            let mark = if self.changed[i] { "✓" } else { " " };
            let (model, follows) = Self::seat_model(view, role);
            let head = format!("{cursor}{mark} {label:<9} ");
            let room = width.saturating_sub(wrap::width(&head));
            let label_style = match (here, self.focus) {
                (true, Focus::Seats) => theme.on_panel(theme.accent).add_modifier(Modifier::BOLD),
                (true, Focus::Models) => theme.on_panel(theme.accent),
                _ => theme.on_panel(theme.fg),
            };
            let mark_style = theme.on_panel(theme.success);
            let model_style = if follows {
                theme.panel_muted()
            } else {
                theme.on_panel(theme.fg)
            };
            if stacked {
                rows.push(vec![
                    Span::styled(cursor.to_string(), label_style),
                    Span::styled(mark.to_string(), mark_style),
                    Span::styled(
                        pad(format!(" {label}"), width.saturating_sub(2)),
                        label_style,
                    ),
                ]);
                rows.push(vec![Span::styled(
                    pad(format!("   {model}"), width),
                    model_style,
                )]);
            } else {
                rows.push(vec![
                    Span::styled(cursor.to_string(), label_style),
                    Span::styled(mark.to_string(), mark_style),
                    Span::styled(format!(" {label:<9} "), label_style),
                    Span::styled(pad(model, room), model_style),
                ]);
            }
        }
        while rows.len() < h.saturating_sub(1) {
            rows.push(vec![Span::styled(" ".repeat(width), theme.panel())]);
        }
        rows.truncate(h.saturating_sub(1));
        rows.push(hints(&[("↑↓", "seat"), ("→", "models")], width, theme));
        rows
    }
}

/// `key label` pairs, padded to `width`.
fn hints(keys: &[(&str, &str)], width: usize, theme: Theme) -> Vec<Span<'static>> {
    use ratatui::style::Modifier;
    let mut spans = vec![Span::styled(" ", theme.panel())];
    let mut used = 1;
    for (i, (key, label)) in keys.iter().enumerate() {
        let gap = if i > 0 { "   " } else { "" };
        let piece = wrap::width(gap) + wrap::width(key) + 1 + wrap::width(label);
        if used + piece > width {
            break;
        }
        let color = match *key {
            "enter" => theme.success,
            "esc" => theme.error,
            _ => theme.accent,
        };
        spans.push(Span::styled(gap.to_string(), theme.panel()));
        spans.push(Span::styled(
            key.to_string(),
            theme.on_panel(color).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(" {label}"), theme.panel_muted()));
        used += piece;
    }
    spans.push(Span::styled(
        " ".repeat(width.saturating_sub(used)),
        theme.panel(),
    ));
    spans
}

impl Models {
    /// A key for the filter. The highlight goes back to the first match only
    /// when the filter's text changed: a key that only moves within it (→,
    /// home, end) leaves the highlight where it was. → used to reset it, and
    /// on a role the first row is `default`, so → then enter dropped the
    /// role's model.
    fn filter_key(&mut self, key: KeyEvent, view: &mut View) {
        let before = view.composer.text().to_string();
        super::edit_field(&mut view.composer, key);
        if view.composer.text() != before {
            self.selected = 0;
        }
    }

    /// Keys on the seats side: ↑↓ chooses a seat, → or enter goes to its
    /// models, and typing starts a filter there.
    fn seat_key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        let seat = self.seat();
        let act = |list: Option<Action>| list.map_or(Outcome::Stay, Outcome::Act);
        match key.code {
            KeyCode::Up => act(self.choose_seat(view, seat.saturating_sub(1))),
            KeyCode::Down => act(self.choose_seat(view, seat + 1)),
            KeyCode::Right | KeyCode::Enter => {
                self.focus = Focus::Models;
                Outcome::Stay
            }
            KeyCode::Char(_) => {
                self.focus = Focus::Models;
                self.filter_key(key, view);
                Outcome::Stay
            }
            KeyCode::Backspace if !view.composer.is_empty() => {
                self.focus = Focus::Models;
                self.filter_key(key, view);
                Outcome::Stay
            }
            _ => Outcome::Stay,
        }
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
        "models".into()
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
        match self.focus {
            Focus::Seats => "↑↓ seat · → models · esc done".into(),
            Focus::Models => {
                "↑↓ move · enter set · ← seats · tab reasoning · s sort · esc done".into()
            }
        }
    }

    fn inline_input(&self) -> bool {
        true
    }

    fn input_indent(&self, width: u16) -> u16 {
        // `width` is the body's, as `render` gets it.
        u16::try_from(Self::seats_width(usize::from(width)) + 1).unwrap_or(0)
    }

    fn keys_in_body(&self) -> bool {
        true
    }

    fn input(&self, _view: &View) -> Option<String> {
        Some("filter".into())
    }

    fn render(&self, view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let h = usize::from(height);
        let left_w = Self::seats_width(w);
        let right_w = w.saturating_sub(left_w + 1);
        let seats = self.seat_lines(view, left_w, h, theme);
        // The right side: the search (drawn over its first row), the table,
        // the highlighted model's facts, then its keys.
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
                if m.id.is_empty() {
                    return vec![
                        "default (follows all hats)".into(),
                        String::new(),
                        String::new(),
                        String::new(),
                        String::new(),
                        m.connection.clone().unwrap_or_default(),
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
        // The cursor shows on the side the keys move.
        let cursor = (self.focus == Focus::Models).then(|| sel.saturating_sub(first));
        // The model's name and its prices show at every width. Short of
        // room, reasoning and context go first, then the connection; the
        // facts line below gives all three for the one highlighted.
        let shown: &[usize] = if right_w >= LIST_WIDTH {
            &[0, 1, 2, 3, 4, 5]
        } else if right_w >= 60 {
            &[0, 2, 3, 5]
        } else {
            &[0, 2, 3]
        };
        let pick = |all: &[&'static str]| shown.iter().map(|&i| all[i]).collect::<Vec<_>>();
        let rows: Vec<Vec<String>> = rows
            .into_iter()
            .map(|r| shown.iter().map(|&i| r[i].clone()).collect())
            .collect();
        let aligns = [
            widgets::Al::L,
            widgets::Al::R,
            widgets::Al::R,
            widgets::Al::R,
            widgets::Al::L,
            widgets::Al::L,
        ];
        let mut table = widgets::table(
            &pick(&[
                "model",
                "context",
                "in/M",
                "out/M",
                "reasoning",
                "connection",
            ]),
            &rows,
            &shown.iter().map(|&i| aligns[i]).collect::<Vec<_>>(),
            cursor,
            right_w,
            theme,
        );
        for (i, off) in toolless.iter().enumerate() {
            if *off && Some(i) != cursor {
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
        let mut right: Vec<Line<'static>> = vec![widgets::blank(theme)];
        right.extend(table);
        while right.len() < h.saturating_sub(2) {
            right.push(widgets::blank(theme));
        }
        right.truncate(h.saturating_sub(2));
        // The highlighted model's facts (R-POP-29).
        // The connection leads, so no cut at the edge hides the provider,
        // however long the model's id.
        let facts = list.get(sel).map(|m| {
            if m.id.is_empty() {
                format!("{} · follows all hats: {}", view.connection, view.model)
            } else {
                let rates = match (m.input_per_million, m.output_per_million) {
                    // A router's price varies by where it routes (listed as
                    // -1): it read "$-1000000/M input".
                    (Some(i), Some(o)) if i < 0.0 || o < 0.0 => "price varies by route".into(),
                    (Some(i), Some(o)) => format_rates(Some(ryter_core::Rates::per_million(i, o))),
                    _ => "price unknown".into(),
                };
                let reasoning = match view.reasoning_label(&m.id) {
                    "auto" => format!(
                        "auto ({} in {})",
                        view.reasoning_effective(view.mode, &m.id),
                        view.mode_label()
                    ),
                    l => l.to_string(),
                };
                let conn = m.connection.as_deref().unwrap_or(&view.connection);
                format!(
                    "{conn} · {} · reasoning {reasoning} · {} · {rates}",
                    m.id,
                    m.context_length
                        .map(|c| format!("{} ctx", format_tokens(c)))
                        .unwrap_or_else(|| "ctx ?".into()),
                )
            }
        });
        right.push(match facts {
            Some(text) => widgets::note(&wrap::truncate(&text, right_w.saturating_sub(2)), theme),
            None => widgets::blank(theme),
        });
        right.push(Line::from(hints(
            &[
                ("enter", "set"),
                ("←", "seats"),
                ("esc", "done"),
                ("tab", "reasoning"),
                ("s", "sort"),
            ],
            right_w,
            theme,
        )));
        let rule = Span::styled("│", theme.panel_muted());
        let lines = seats
            .into_iter()
            .zip(right)
            .map(|(mut l, r)| {
                l.push(rule.clone());
                l.extend(r.spans);
                Line::from(l)
            })
            .collect();
        Body {
            lines,
            scroll: (n > rows_h).then_some((first, n)),
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        if key.code == KeyCode::Esc {
            view.composer.clear();
            return Outcome::Close;
        }
        if self.focus == Focus::Seats {
            return self.seat_key(key, view);
        }
        let n = self.filtered(view).len();
        match key.code {
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
            // Back to the seats, the filter kept.
            KeyCode::Left => {
                self.focus = Focus::Seats;
                Outcome::Stay
            }
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
            // Set the seat's model, and go back to the seats, on the next
            // one: the panel stays open until esc.
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
                let set = match &self.assign_role {
                    Some(role) if m.id.is_empty() => Action::ResetHatModel(role.clone()),
                    Some(role) => Action::SetHatModel {
                        role: role.clone(),
                        connection: m.connection.unwrap_or_else(|| view.connection.clone()),
                        model: m.id,
                    },
                    None => Action::SetModel(m.id),
                };
                let seat = self.seat();
                self.changed[seat] = true;
                self.focus = Focus::Seats;
                if seat + 1 == SEATS {
                    // The last seat stays chosen. Its list must open on the
                    // model just set: reading the seat now would find the
                    // one before, as the set hasn't been applied yet.
                    let (model, connection) = match &set {
                        Action::SetModel(id) => (id.clone(), Some(view.connection.clone())),
                        Action::SetHatModel {
                            model, connection, ..
                        } => (model.clone(), Some(connection.clone())),
                        _ => (String::new(), None),
                    };
                    self.highlight(view, &model, connection.as_deref());
                    return Outcome::Act(set);
                }
                match self.choose_seat(view, seat + 1) {
                    Some(list) => Outcome::Act(Action::Many(vec![set, list])),
                    None => Outcome::Act(set),
                }
            }
            _ => {
                self.filter_key(key, view);
                Outcome::Stay
            }
        }
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        // Two panes: room for the seats' models and the list's names.
        (124, 22)
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

    fn text(p: &Models, v: &View) -> String {
        // The panel's full width: seats on one line each.
        p.render(v, 122, 20, Theme::truecolor_dark())
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
    }

    fn hats_view() -> View {
        let mut v = View::new("openrouter".into(), "x-ai/grok-4.7".into(), "/tmp".into());
        v.mode = ryter_core::Role::SoloBuild;
        v.specialists.insert(
            "review".into(),
            ryter_core::RoleModel {
                connection: Some("openrouter".into()),
                model: Some("qwen/qwen3.7-max".into()),
            },
        );
        v
    }

    fn catalog() -> Vec<ModelInfo> {
        vec![
            row("x-ai/grok-4.7", "openrouter", Some((2.0, 6.0))),
            row("qwen/qwen3.7-max", "openrouter", Some((1.6, 6.4))),
            row("minimax/minimax-m2.7", "openrouter", Some((0.3, 1.2))),
            row("grok-4.6", "spacexai", Some((2.0, 6.0))),
        ]
    }

    /// In the one mode the seats are the hats: "All hats", then Plan, Build
    /// and Review, each following it until it has a model of its own. One
    /// visit sets them all, and a hat's seat names the hat when it is set.
    #[test]
    fn the_hats_are_the_seats() {
        let mut v = hats_view();
        v.specialists.clear();
        let mut p = Models::new(&mut v, None);
        p.set_models(&v, &catalog());
        let t = text(&p, &v);
        assert!(t.contains("›  All hats  grok-4.7"), "{t}");
        for hat in ["Plan", "Build", "Review"] {
            assert!(
                t.contains(&format!("{hat:<9} follows all hats")),
                "{hat}:\n{t}"
            );
        }
        assert!(!t.contains("Architect") && !t.contains("Auditor"), "{t}");
        assert_eq!(p.title(&v), "models");

        // Down to Plan, across to its models, and enter gives it one: the
        // cursor goes on to Build.
        key(&mut p, &mut v, KeyCode::Down);
        p.set_models(&v, &catalog());
        assert_eq!(p.seat(), 1);
        key(&mut p, &mut v, KeyCode::Right);
        v.composer.set_text("qwen");
        match key(&mut p, &mut v, KeyCode::Enter) {
            Outcome::Act(set) => assert!(
                matches!(
                    &set,
                    Action::SetHatModel { role, model, connection }
                        if role == "plan" && model == "qwen/qwen3.7-max" && connection == "openrouter"
                ),
                "{set:?}"
            ),
            _ => panic!("set the plan hat's model"),
        }
        assert_eq!((p.focus, p.seat()), (Focus::Seats, 2));
        // As the loop applies it, the seat shows it, and the hat it is on
        // says so wherever the model is named.
        v.specialists.insert(
            "plan".into(),
            ryter_core::RoleModel {
                connection: Some("openrouter".into()),
                model: Some("qwen/qwen3.7-max".into()),
            },
        );
        let t = text(&p, &v);
        assert!(t.contains("Plan      qwen3.7-max"), "{t}");
        assert!(t.contains("Build     follows all hats"), "{t}");
        assert_eq!(v.hat_model(), "x-ai/grok-4.7", "the build hat follows");
        v.mode = ryter_core::Role::SoloPlan;
        assert_eq!(v.hat_model(), "qwen/qwen3.7-max");
    }

    /// Every seat is set in one visit: the seats beside the list, and
    /// enter sets the seat's model and goes back to the seats, on the next
    /// one. It used to close after every seat, so four seats took four
    /// visits.
    #[test]
    fn every_seat_is_set_without_leaving_the_panel() {
        let mut v = hats_view();
        let mut p = Models::new(&mut v, None);
        p.set_models(&v, &catalog());
        let t = text(&p, &v);
        assert!(
            t.contains("SEATS") && t.contains("›  All hats  grok-4.7"),
            "{t}"
        );
        assert!(
            t.contains("Plan      follows all hats") && t.contains("Review    qwen3.7-max"),
            "{t}"
        );
        assert_eq!(p.focus, Focus::Seats);
        assert_eq!(p.title(&v), "models");

        // All hats: → to its models, enter sets it, and the cursor is back
        // on the seats, on the architect, whose list (every connection's)
        // is asked for in the same breath.
        assert!(matches!(key(&mut p, &mut v, KeyCode::Right), Outcome::Stay));
        assert_eq!(p.focus, Focus::Models);
        v.composer.set_text("grok-4.7");
        match key(&mut p, &mut v, KeyCode::Enter) {
            Outcome::Act(Action::Many(acts)) => {
                assert!(matches!(&acts[0], Action::SetModel(m) if m == "x-ai/grok-4.7"));
                assert!(matches!(&acts[1], Action::ListAllModels { role } if role == "plan"));
            }
            _ => panic!("set the first seat and list the plan hat's models"),
        }
        assert_eq!((p.focus, p.seat()), (Focus::Seats, 1));
        assert!(v.composer.is_empty(), "the filter is cleared");
        p.set_models(&v, &catalog());

        // The architect: enter goes to its models, typing filters there.
        key(&mut p, &mut v, KeyCode::Enter);
        for c in "minimax".chars() {
            key(&mut p, &mut v, KeyCode::Char(c));
        }
        assert!(matches!(
            key(&mut p, &mut v, KeyCode::Enter),
            Outcome::Act(Action::SetHatModel { role, model, .. })
                if role == "plan" && model == "minimax/minimax-m2.7"
        ));
        assert_eq!((p.focus, p.seat()), (Focus::Seats, 2));

        // The builder: typing on the seats side starts a filter at once.
        for c in "grok-4.6".chars() {
            key(&mut p, &mut v, KeyCode::Char(c));
        }
        assert_eq!(p.focus, Focus::Models);
        assert!(
            text(&p, &v).contains("grok-4.6"),
            "every connection for a role"
        );
        assert!(matches!(
            key(&mut p, &mut v, KeyCode::Enter),
            Outcome::Act(Action::SetHatModel { role, connection, .. })
                if role == "build" && connection == "spacexai"
        ));

        // The auditor, the last seat: ← goes back without setting, and the
        // cursor stays on the last seat after a set.
        assert_eq!(p.seat(), 3);
        key(&mut p, &mut v, KeyCode::Right);
        assert!(matches!(key(&mut p, &mut v, KeyCode::Left), Outcome::Stay));
        assert_eq!(p.focus, Focus::Seats);
        key(&mut p, &mut v, KeyCode::Right);
        v.composer.set_text("qwen");
        assert!(matches!(
            key(&mut p, &mut v, KeyCode::Enter),
            Outcome::Act(Action::SetHatModel { role, model, .. })
                if role == "review" && model == "qwen/qwen3.7-max"
        ));
        assert_eq!((p.focus, p.seat()), (Focus::Seats, 3));

        // Each seat set is ticked; esc closes, keeping them.
        let t = text(&p, &v);
        for seat in ["✓ All hats", "✓ Plan", "✓ Build", "✓ Review"] {
            assert!(t.contains(seat), "{seat}: {t}");
        }
        assert!(matches!(key(&mut p, &mut v, KeyCode::Esc), Outcome::Close));
    }

    /// ↑↓ moves between seats without wrapping, the list follows the seat
    /// and opens on its model, and opening on a hat
    /// starts in that role's models.
    #[test]
    fn the_list_follows_the_seat() {
        let mut v = hats_view();
        let mut p = Models::new(&mut v, None);
        p.set_models(&v, &catalog());
        assert!(matches!(key(&mut p, &mut v, KeyCode::Up), Outcome::Stay));
        assert_eq!(p.seat(), 0);
        // The first seat's list is its own connection's, with no `default`.
        let t = text(&p, &v);
        assert!(!t.contains("grok-4.6 ") && !t.contains("default"), "{t}");
        assert!(matches!(
            key(&mut p, &mut v, KeyCode::Down),
            Outcome::Act(Action::ListAllModels { role }) if role == "plan"
        ));
        key(&mut p, &mut v, KeyCode::Down);
        key(&mut p, &mut v, KeyCode::Down);
        assert!(matches!(key(&mut p, &mut v, KeyCode::Down), Outcome::Stay));
        assert_eq!(p.seat(), 3);
        p.set_models(&v, &catalog());
        let on = p.filtered(&v)[p.selected].id.clone();
        assert_eq!(
            on, "qwen/qwen3.7-max",
            "the auditor's list opens on its model"
        );

        let p = Models::new(&mut v, Some("build".into()));
        assert_eq!((p.focus, p.seat()), (Focus::Models, 2));
    }

    /// Keys that only move within the filter (→, home, end) leave the
    /// highlight where it is. → used to send it to the first row, which on
    /// a role is `default`: → then enter dropped the role's model.
    #[test]
    fn moving_in_the_filter_keeps_the_highlight() {
        let mut v = hats_view();
        let mut p = Models::new(&mut v, Some("build".into()));
        p.set_models(&v, &catalog());
        let minimax = p
            .filtered(&v)
            .iter()
            .position(|m| m.id == "minimax/minimax-m2.7")
            .unwrap();
        p.selected = minimax;
        for code in [KeyCode::Right, KeyCode::Home, KeyCode::End] {
            assert!(matches!(key(&mut p, &mut v, code), Outcome::Stay));
            assert_eq!(p.selected, minimax, "{code:?}");
        }
        assert!(matches!(
            key(&mut p, &mut v, KeyCode::Enter),
            Outcome::Act(Action::SetHatModel { role, model, .. })
                if role == "build" && model == "minimax/minimax-m2.7"
        ));
        // With a filter, typing goes to the first match, and moving within
        // the text doesn't.
        let mut p = Models::new(&mut v, Some("build".into()));
        p.set_models(&v, &catalog());
        key(&mut p, &mut v, KeyCode::Char('o'));
        assert_eq!(p.selected, 0);
        key(&mut p, &mut v, KeyCode::Down);
        let on = p.selected;
        assert!(on > 0);
        key(&mut p, &mut v, KeyCode::Right);
        key(&mut p, &mut v, KeyCode::Home);
        assert_eq!(p.selected, on);
        key(&mut p, &mut v, KeyCode::Left);
        assert_eq!(p.focus, Focus::Seats, "← on the list goes to the seats");
    }

    /// Setting the last seat keeps it chosen, and its list opens on the
    /// model just set: it used to read the seat before the set landed, so
    /// enter again saved the old model back.
    #[test]
    fn the_last_seat_opens_on_the_model_just_set() {
        let mut v = hats_view();
        let mut p = Models::new(&mut v, Some("review".into()));
        p.set_models(&v, &catalog());
        assert_eq!(
            p.filtered(&v)[p.selected].id,
            "qwen/qwen3.7-max",
            "its model now"
        );
        v.composer.set_text("minimax");
        assert!(matches!(
            key(&mut p, &mut v, KeyCode::Enter),
            Outcome::Act(Action::SetHatModel { role, model, .. })
                if role == "review" && model == "minimax/minimax-m2.7"
        ));
        assert_eq!((p.focus, p.seat()), (Focus::Seats, 3));
        assert_eq!(p.filtered(&v)[p.selected].id, "minimax/minimax-m2.7");
        key(&mut p, &mut v, KeyCode::Right);
        assert!(matches!(
            key(&mut p, &mut v, KeyCode::Enter),
            Outcome::Act(Action::SetHatModel { model, .. }) if model == "minimax/minimax-m2.7"
        ));
    }

    /// Narrow, the list drops its connection and reasoning columns, and the
    /// facts line leads with them.
    #[test]
    fn narrow_the_facts_line_names_the_connection() {
        let mut v = hats_view();
        let mut p = Models::new(&mut v, Some("build".into()));
        p.set_models(&v, &catalog());
        p.selected = p
            .filtered(&v)
            .iter()
            .position(|m| m.id == "grok-4.6")
            .unwrap();
        let narrow: String = p
            .render(&v, 78, 16, Theme::truecolor_dark())
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
        assert!(!narrow.contains("connection"), "no column: {narrow}");
        assert!(
            narrow.contains("spacexai · grok-4.6 · reasoning"),
            "{narrow}"
        );
        let wide = text(&p, &v);
        assert!(
            wide.contains("connection") && wide.contains("spacexai"),
            "{wide}"
        );
    }

    /// One model can be on two connections. The highlight follows the
    /// connection as well as the id, when a seat opens on its model and
    /// after the last seat is set; by id alone it went to the first
    /// connection's row, and enter then moved the seat there. A reviewer's
    /// case: `shared-model` on openrouter and on spacexai.
    #[test]
    fn a_model_on_two_connections_keeps_its_connection() {
        let mut v = hats_view();
        let mut models = catalog();
        models.push(row("shared-model", "openrouter", Some((1.0, 2.0))));
        models.push(row("shared-model", "spacexai", Some((1.0, 2.0))));
        let mut p = Models::new(&mut v, Some("review".into()));
        p.set_models(&v, &models);
        for c in "shared".chars() {
            key(&mut p, &mut v, KeyCode::Char(c));
        }
        assert_eq!(p.selected, 0, "openrouter's copy first");
        key(&mut p, &mut v, KeyCode::Down);
        assert!(matches!(
            key(&mut p, &mut v, KeyCode::Enter),
            Outcome::Act(Action::SetHatModel { connection, model, .. })
                if connection == "spacexai" && model == "shared-model"
        ));
        let on = p.filtered(&v)[p.selected];
        assert_eq!(
            (on.id.as_str(), on.connection.as_deref()),
            ("shared-model", Some("spacexai"))
        );
        key(&mut p, &mut v, KeyCode::Right);
        assert!(matches!(
            key(&mut p, &mut v, KeyCode::Enter),
            Outcome::Act(Action::SetHatModel { connection, .. }) if connection == "spacexai"
        ));
        // Opening on a seat set to the second connection's copy.
        v.specialists.insert(
            "review".into(),
            ryter_core::RoleModel {
                connection: Some("spacexai".into()),
                model: Some("shared-model".into()),
            },
        );
        let mut p = Models::new(&mut v, Some("review".into()));
        p.set_models(&v, &models);
        assert_eq!(
            p.filtered(&v)[p.selected].connection.as_deref(),
            Some("spacexai")
        );
    }

    /// The model's name and its prices show at every width; the facts line
    /// leads with the connection, so a long id can't push it out of sight.
    #[test]
    fn names_and_prices_show_at_every_width() {
        let mut v = hats_view();
        let models = vec![
            row(
                "deepseek/deepseek-v4.1-flash",
                "openrouter",
                Some((0.02, 0.6)),
            ),
            row(
                "anthropic/claude-3.7-sonnet:thinking",
                "openrouter",
                Some((3.0, 15.0)),
            ),
        ];
        let mut p = Models::new(&mut v, Some("build".into()));
        p.set_models(&v, &models);
        p.selected = p
            .filtered(&v)
            .iter()
            .position(|m| m.id.starts_with("anthropic"))
            .unwrap();
        // The body widths of an 80, 100 and 120 column terminal.
        for width in [70u16, 90, 110] {
            let t: String = p
                .render(&v, width, 16, Theme::truecolor_dark())
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
            assert!(t.contains("deepseek/deepseek-v4.1-flash "), "{width}: {t}");
            assert!(
                t.contains("$0.02") && t.contains("$0.6") && t.contains("$15"),
                "{width}: {t}"
            );
            assert!(
                t.contains("openrouter · anthropic/claude-3.7-sonnet"),
                "{width}: {t}"
            );
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
        p.focus = Focus::Models;
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
