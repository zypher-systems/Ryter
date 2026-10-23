//! `/benchmark` — the bench's tasks on a model of your choice, from inside
//! the TUI: pick the connection, the model, the level and a budget, and
//! watch tasks solved per dollar come in. The run is `ryter bench`'s, in a
//! thread; its table lands under `~/.ryter/bench/` and its headline in the
//! chat. The model is picked from the connection's list, with context and
//! prices, as `/models` shows it.

use std::collections::{BTreeMap, BTreeSet};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::text::{Line, Span};
use ryter_bench::{Level, Progress, Report, TaskResult};
use ryter_core::{ModelInfo, format_tokens};

use super::models::price;
use super::widgets::{self, Al, Field, Form, Kind};
use super::{Body, Notice, Outcome, Panel};
use crate::action::Action;
use crate::activity::SPINNER;
use crate::chat::{short_model, wrap};
use crate::theme::Theme;
use crate::view::View;

const LEVELS: [&str; 3] = ["standard", "hard", "all"];

/// One task's row while a run is going.
#[derive(Debug, Clone, PartialEq)]
struct Row {
    name: String,
    level: Level,
    state: State,
    usd: Option<f64>,
    seconds: f64,
}

#[derive(Debug, Clone, PartialEq)]
enum State {
    Waiting,
    Running,
    Scoring,
    Done(TaskResult),
}

#[derive(Debug, Clone, PartialEq)]
enum Phase {
    /// The form.
    Setup,
    /// A run is going.
    Running {
        rows: Vec<Row>,
        spent: f64,
        activity: String,
        /// Esc asked to stop; waiting for the answer.
        confirm_stop: bool,
    },
    /// The run finished.
    Done(Box<Report>),
}

/// The benchmark panel.
#[derive(Debug, Clone)]
pub struct Benchmark {
    form: Form,
    /// Editing the budget via the composer.
    editing: bool,
    phase: Phase,
    /// The keyed connections, in the select's order.
    connections: Vec<String>,
    /// Every keyed connection's models, as `/models` lists them: the
    /// built-in rows until the lists come.
    catalog: Vec<ModelInfo>,
    /// Some keyed connection's list is still to come.
    loading: bool,
    /// Connections whose list has come, or won't.
    settled: BTreeSet<String>,
    /// The model list is open, with the cursor on this row.
    picking: Option<usize>,
}

impl Benchmark {
    /// The form, set to the build hat's connection and model.
    pub fn new(view: &View) -> Self {
        let build = view.hats.get("build");
        let connection = build
            .and_then(|h| h.connection.clone())
            .unwrap_or_else(|| view.connection.clone());
        let model = build
            .and_then(|h| h.model.clone())
            .unwrap_or_else(|| view.model.clone());
        let mut connections: Vec<String> = view
            .connections
            .iter()
            .filter(|c| c.has_key)
            .map(|c| c.name.clone())
            .collect();
        if connections.is_empty() {
            connections.push(connection.clone());
        }
        let idx = connections
            .iter()
            .position(|c| c == &connection)
            .unwrap_or(0);
        let mut catalog = Vec::new();
        for c in &view.connections {
            if !connections.contains(&c.name) {
                continue;
            }
            let mut fb = crate::view::fallback_models(&c.kind, &c.model);
            for m in &mut fb {
                m.connection = Some(c.name.clone());
            }
            catalog.extend(fb);
        }
        let loading = view.connections.iter().any(|c| c.has_key);
        let fields = vec![
            Field::new(
                "connection",
                "connection",
                Kind::Select {
                    options: connections.clone(),
                    idx,
                },
            ),
            Field::new("model", "model", Kind::Text(model)),
            Field::new(
                "level",
                "tasks",
                Kind::Select {
                    options: LEVELS.iter().map(|s| (*s).to_string()).collect(),
                    idx: 0,
                },
            ),
            Field::new(
                "budget",
                "budget usd",
                Kind::Number {
                    value: 5.0,
                    min: ryter_bench::RESERVE_USD + 0.01,
                    max: 50.0,
                    step: 0.5,
                    int: false,
                },
            ),
            Field::new(
                "sandbox",
                "sandbox",
                Kind::Select {
                    options: vec!["workspace".into(), "off".into()],
                    idx: usize::from(!cfg!(target_os = "linux")),
                },
            ),
            Field::new("start", "start the run", Kind::Action),
        ];
        Self {
            form: Form::new(fields),
            editing: false,
            phase: Phase::Setup,
            connections,
            catalog,
            loading,
            settled: BTreeSet::new(),
            picking: None,
        }
    }

    /// The connection chosen on the form.
    fn connection(&self, view: &View) -> String {
        self.connections
            .get(self.select_idx("connection"))
            .cloned()
            .unwrap_or_else(|| view.connection.clone())
    }

    /// The chosen connection's models, those the filter keeps when the list
    /// is open, in the catalog's order.
    fn listed(&self, view: &View) -> Vec<&ModelInfo> {
        let connection = self.connection(view);
        let f = if self.picking.is_some() {
            view.composer.text().trim().to_ascii_lowercase()
        } else {
            String::new()
        };
        self.catalog
            .iter()
            .filter(|m| m.connection.as_deref() == Some(connection.as_str()))
            .filter(|m| {
                f.is_empty()
                    || m.id.to_ascii_lowercase().contains(&f)
                    || short_model(&m.id).to_ascii_lowercase().contains(&f)
            })
            .collect()
    }

    fn set_model(&mut self, model: &str) {
        if let Some(Kind::Text(t)) = self.form.get_mut("model").map(|f| &mut f.kind) {
            *t = model.trim().to_string();
        }
    }

    /// The connection changed: its own default model goes in the model
    /// field, as a model from the last connection is not on this one.
    fn follow_connection(&mut self, view: &View) {
        let connection = self.connection(view);
        let own = view
            .connections
            .iter()
            .find(|c| c.name == connection)
            .map(|c| c.model.clone())
            .filter(|m| !m.is_empty())
            .or_else(|| self.listed(view).first().map(|m| m.id.clone()))
            .unwrap_or_default();
        self.set_model(&own);
    }

    /// Open the model list, the cursor on the model in the field.
    fn open_list(&mut self, view: &mut View) {
        view.composer.clear();
        self.picking = Some(0);
        let model = self.model();
        if let Some(i) = self.listed(view).iter().position(|m| m.id == model) {
            self.picking = Some(i);
        }
    }

    /// Take a `Models` notice as `/models` does: the rows of the
    /// connections it carries or covers are replaced, even to none; the
    /// rest stay; a connection that has lost its key goes.
    fn set_models(
        &mut self,
        view: &View,
        models: &[ModelInfo],
        covered: &[String],
        failed: &[String],
    ) {
        let on = |m: &ModelInfo| {
            m.connection
                .clone()
                .unwrap_or_else(|| view.connection.clone())
        };
        if !models.is_empty() || !covered.is_empty() {
            let keyed = |c: &str| self.connections.iter().any(|n| n == c);
            let mut replaced: BTreeSet<String> = models.iter().map(on).collect();
            replaced.extend(covered.iter().cloned());
            let mut by_connection: BTreeMap<String, Vec<ModelInfo>> = BTreeMap::new();
            for m in models {
                if keyed(&on(m)) {
                    by_connection.entry(on(m)).or_default().push(m.clone());
                }
            }
            for m in &self.catalog {
                let c = on(m);
                if !replaced.contains(&c) && keyed(&c) {
                    by_connection.entry(c).or_default().push(m.clone());
                }
            }
            self.catalog = by_connection.into_values().flatten().collect();
        }
        self.settled.extend(covered.iter().cloned());
        self.settled.extend(failed.iter().cloned());
        self.loading = view
            .connections
            .iter()
            .any(|c| c.has_key && !self.settled.contains(&c.name));
        if let Some(sel) = self.picking {
            let n = self.listed(view).len();
            self.picking = Some(sel.min(n.saturating_sub(1)));
        }
    }

    /// The open model list: the chosen connection's models with context
    /// and prices, the cursor on one.
    fn render_list(&self, view: &View, w: usize, h: usize, theme: Theme) -> Vec<Line<'static>> {
        let list = self.listed(view);
        let n = list.len();
        let sel = self.picking.unwrap_or(0).min(n.saturating_sub(1));
        let rows_h = h.saturating_sub(3).max(1);
        let first = super::window(sel, n, rows_h);
        let rows: Vec<Vec<String>> = list
            .iter()
            .skip(first)
            .take(rows_h)
            .map(|m| {
                vec![
                    m.id.clone(),
                    m.context_length
                        .map(format_tokens)
                        .unwrap_or_else(|| "?".into()),
                    price(m.input_per_million),
                    price(m.output_per_million),
                ]
            })
            .collect();
        let toolless: Vec<bool> = list
            .iter()
            .skip(first)
            .take(rows_h)
            .map(|m| m.tools == Some(false))
            .collect();
        let mut lines = vec![widgets::text(
            &format!("model on {} · type to filter", self.connection(view)),
            theme,
        )];
        let mut table = widgets::table(
            &["model", "context", "in/M", "out/M"],
            &rows,
            &[Al::L, Al::R, Al::R, Al::R],
            (n > 0).then_some(sel.saturating_sub(first)),
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
        lines.extend(table);
        if self.loading {
            let frame = SPINNER[(view.now_ms / 80) as usize % SPINNER.len()];
            lines.push(Line::from(vec![
                Span::styled(format!(" {frame} "), theme.on_panel(theme.accent)),
                Span::styled("loading models…", theme.panel_muted()),
            ]));
        } else if n == 0 {
            let f = view.composer.text().trim().to_string();
            lines.push(widgets::note(
                &if f.is_empty() {
                    "no model listed; type an id and press enter".to_string()
                } else {
                    format!("no listed model matches; enter uses \"{f}\" as the id")
                },
                theme,
            ));
        }
        if let Some(note) = &view.models_note {
            lines.push(widgets::note(note, theme));
        }
        lines
    }

    fn select_idx(&self, id: &str) -> usize {
        match self.form.get(id).map(|f| &f.kind) {
            Some(Kind::Select { idx, .. }) => *idx,
            _ => 0,
        }
    }

    fn level(&self) -> Option<Level> {
        match LEVELS[self.select_idx("level").min(2)] {
            "standard" => Some(Level::Standard),
            "hard" => Some(Level::Hard),
            _ => None,
        }
    }

    fn model(&self) -> String {
        match self.form.get("model").map(|f| &f.kind) {
            Some(Kind::Text(t)) => t.trim().to_string(),
            _ => String::new(),
        }
    }

    fn budget(&self) -> f64 {
        match self.form.get("budget").map(|f| &f.kind) {
            Some(Kind::Number { value, .. }) => *value,
            _ => 0.0,
        }
    }

    /// The action that starts the run, when the form says enough.
    fn start(&self, view: &mut View) -> Outcome {
        if view.bench_running {
            view.error("a benchmark run is already going; its result lands in the chat");
            return Outcome::Stay;
        }
        if self.model().is_empty() {
            view.error("name a model to run the bench on");
            return Outcome::Stay;
        }
        let connection = self.connection(view);
        if self.catalog.iter().any(|m| {
            m.id == self.model()
                && m.connection.as_deref() == Some(connection.as_str())
                && m.tools == Some(false)
        }) {
            view.error("that model can't call tools, so it can't read a file or run a test");
            return Outcome::Stay;
        }
        let tasks = ryter_bench::select(&[], self.level()).len();
        if tasks == 0 {
            view.error("no task at that level");
            return Outcome::Stay;
        }
        Outcome::Act(Action::StartBench {
            connection,
            model: self.model(),
            level: self.level(),
            budget_usd: self.budget(),
            sandbox: if self.select_idx("sandbox") == 0 {
                "workspace"
            } else {
                "off"
            }
            .to_string(),
        })
    }

    /// `←`/`→` on the focused field; a new connection brings its model.
    fn adjust(&mut self, view: &View, delta: i32) {
        let on_connection = self.form.current().is_some_and(|f| f.id == "connection");
        if self.form.adjust(delta) && on_connection {
            self.follow_connection(view);
        }
    }

    fn rows_for(tasks: &[&ryter_bench::Task]) -> Vec<Row> {
        tasks
            .iter()
            .map(|t| Row {
                name: t.name.clone(),
                level: t.level,
                state: State::Waiting,
                usd: None,
                seconds: 0.0,
            })
            .collect()
    }

    fn render_running(
        rows: &[Row],
        spent: f64,
        activity: &str,
        confirm_stop: bool,
        w: usize,
        theme: Theme,
    ) -> Vec<ratatui::text::Line<'static>> {
        let done = rows
            .iter()
            .filter(|r| matches!(r.state, State::Done(_)))
            .count();
        let solved = rows
            .iter()
            .filter(|r| matches!(&r.state, State::Done(t) if t.solved == Some(true)))
            .count();
        let mut lines = vec![widgets::text(
            &format!(
                "{done} of {} done · {solved} solved · spent {}",
                rows.len(),
                ryter_bench::report::usd(Some(spent))
            ),
            theme,
        )];
        if !activity.is_empty() {
            for row in wrap::wrap_plain(activity, w.saturating_sub(2)) {
                lines.push(widgets::note(&row, theme));
            }
        }
        lines.push(widgets::blank(theme));
        let table: Vec<Vec<String>> = rows
            .iter()
            .map(|r| {
                let (mark, state) = match &r.state {
                    State::Waiting => ("·", "waiting".to_string()),
                    State::Running => ("▸", "running".to_string()),
                    State::Scoring => ("▸", "scoring".to_string()),
                    State::Done(t) => mark_for(t),
                };
                vec![
                    format!("{mark} {}", r.name),
                    r.level.to_string(),
                    state,
                    ryter_bench::report::usd(r.usd),
                    if matches!(&r.state, State::Done(t) if t.solved.is_some()) {
                        format!("{:.1}m", r.seconds / 60.0)
                    } else {
                        String::new()
                    },
                ]
            })
            .collect();
        lines.extend(widgets::table(
            &["task", "level", "result", "cost", "time"],
            &table,
            &[Al::L, Al::L, Al::L, Al::R, Al::R],
            None,
            w,
            theme,
        ));
        if confirm_stop {
            lines.push(widgets::blank(theme));
            lines.push(widgets::text(
                "stop the run after the task in progress?  ⏎ yes · n no",
                theme,
            ));
        }
        lines
    }

    fn render_done(report: &Report, w: usize, theme: Theme) -> Vec<ratatui::text::Line<'static>> {
        let s = report.summary();
        let mut lines: Vec<ratatui::text::Line<'static>> =
            wrap::wrap_plain(&report.headline(), w.saturating_sub(2))
                .iter()
                .map(|row| widgets::text(row, theme))
                .collect();
        lines.push(widgets::note(
            &format!(
                "{} on {} ({}) · budget ${:.2} · {} s per task",
                report.ryter,
                report.model,
                report.connection,
                report.budget_usd,
                report.timeout_seconds
            ),
            theme,
        ));
        lines.push(widgets::blank(theme));
        let table: Vec<Vec<String>> = report
            .results
            .iter()
            .map(|t| {
                let (mark, state) = mark_for(t);
                vec![
                    format!("{mark} {}", t.name),
                    t.level.to_string(),
                    state,
                    ryter_bench::report::usd(t.usd),
                    if t.solved.is_some() {
                        format!("{:.1}m", t.seconds / 60.0)
                    } else {
                        String::new()
                    },
                ]
            })
            .collect();
        lines.extend(widgets::table(
            &["task", "level", "result", "cost", "time"],
            &table,
            &[Al::L, Al::L, Al::L, Al::R, Al::R],
            None,
            w,
            theme,
        ));
        lines.push(widgets::blank(theme));
        lines.push(widgets::text(
            &format!(
                "{}/{} solved · cost per solved {} · median {} min · timed out {}",
                s.solved,
                s.tasks,
                ryter_bench::report::usd(s.usd_per_solved),
                s.median_minutes
                    .map_or("—".to_string(), |m| format!("{m:.1}")),
                s.timed_out
            ),
            theme,
        ));
        for row in wrap::wrap_plain(
            &format!(
                "the table is in {}",
                report.directory.join("RESULTS.md").display()
            ),
            w.saturating_sub(2),
        ) {
            lines.push(widgets::note(&row, theme));
        }
        lines
    }
}

/// A finished task's mark and words.
fn mark_for(t: &TaskResult) -> (&'static str, String) {
    match (t.solved, &t.error) {
        (Some(true), _) => ("✓", "solved".into()),
        (Some(false), _) if t.timed_out => ("✗", "timed out".into()),
        (Some(false), _) if t.cancelled => ("✗", "stopped".into()),
        (Some(false), _) => ("✗", "failed".into()),
        (None, Some(why)) => ("–", format!("skipped: {why}")),
        (None, None) => ("–", "not run".into()),
    }
}

impl Panel for Benchmark {
    fn kind(&self) -> &'static str {
        "benchmark"
    }

    fn title(&self, _view: &View) -> String {
        "benchmark".into()
    }

    fn status(&self, view: &View) -> String {
        match &self.phase {
            Phase::Setup if view.bench_running => "a run is going".into(),
            Phase::Setup => format!("{} tasks", ryter_bench::select(&[], self.level()).len()),
            Phase::Running { rows, .. } => {
                let done = rows
                    .iter()
                    .filter(|r| matches!(r.state, State::Done(_)))
                    .count();
                format!("{done} of {}", rows.len())
            }
            Phase::Done(r) => format!("{}/{}", r.summary().solved, r.summary().tasks),
        }
    }

    fn legend(&self, _view: &View) -> String {
        match &self.phase {
            Phase::Setup if self.picking.is_some() => {
                "↑↓ move · type to filter · enter choose · esc back".into()
            }
            Phase::Setup if self.editing => "type · enter apply · esc cancel".into(),
            Phase::Setup => "↑↓ move · ←→ change · enter choose/start · esc close".into(),
            Phase::Running {
                confirm_stop: true, ..
            } => "⏎ stop · n keep running".into(),
            Phase::Running { .. } => "esc stop · c close (the run goes on)".into(),
            Phase::Done(_) => "r run again · esc close".into(),
        }
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        (84, 24)
    }

    fn input(&self, _view: &View) -> Option<String> {
        if self.picking.is_some() {
            return Some("filter".into());
        }
        self.editing
            .then(|| self.form.current().map(|f| f.label.clone()))
            .flatten()
    }

    fn prefill(&self, _view: &View) -> Option<String> {
        self.editing
            .then(|| self.form.current().map(Field::value_text))
            .flatten()
    }

    fn render(&self, view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let mut lines = match &self.phase {
            Phase::Setup if self.picking.is_some() => {
                self.render_list(view, w, usize::from(height), theme)
            }
            Phase::Setup => {
                let mut lines = Vec::new();
                if view.bench_running {
                    lines.push(widgets::text(
                        "a run is going; its headline lands in the chat when it is done",
                        theme,
                    ));
                    lines.push(widgets::blank(theme));
                }
                lines.extend(self.form.render(w, theme, true));
                lines.push(widgets::blank(theme));
                let prose = "each task runs in a fresh copy under the build hat with its own session \
                             cap, then its hidden tests score it; the cost is what the model charges. \
                             the sandbox is the build hat's (Landlock, Linux only; off where it \
                             can't apply). the table goes to ~/.ryter/bench/.";
                for row in wrap::wrap_plain(prose, w.saturating_sub(2)) {
                    lines.push(widgets::note(&row, theme));
                }
                lines
            }
            Phase::Running {
                rows,
                spent,
                activity,
                confirm_stop,
            } => Self::render_running(rows, *spent, activity, *confirm_stop, w, theme),
            Phase::Done(report) => Self::render_done(report, w, theme),
        };
        lines.truncate(usize::from(height).max(1));
        Body {
            lines,
            scroll: None,
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        match &mut self.phase {
            Phase::Running { confirm_stop, .. } => {
                if *confirm_stop {
                    return match key.code {
                        KeyCode::Enter | KeyCode::Char('y' | 'Y') => {
                            *confirm_stop = false;
                            Outcome::Act(Action::CancelBench)
                        }
                        _ => {
                            *confirm_stop = false;
                            Outcome::Stay
                        }
                    };
                }
                return match key.code {
                    KeyCode::Esc => {
                        *confirm_stop = true;
                        Outcome::Stay
                    }
                    KeyCode::Char('c') => Outcome::Close,
                    _ => Outcome::Stay,
                };
            }
            Phase::Done(_) => {
                return match key.code {
                    KeyCode::Esc => Outcome::Close,
                    KeyCode::Char('r') => {
                        self.phase = Phase::Setup;
                        Outcome::Stay
                    }
                    _ => Outcome::Stay,
                };
            }
            Phase::Setup => {}
        }
        if let Some(sel) = self.picking {
            return match key.code {
                KeyCode::Esc => {
                    self.picking = None;
                    view.composer.clear();
                    Outcome::Stay
                }
                KeyCode::Up => {
                    self.picking = Some(sel.saturating_sub(1));
                    Outcome::Stay
                }
                KeyCode::Down => {
                    let n = self.listed(view).len();
                    self.picking = Some((sel + 1).min(n.saturating_sub(1)));
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    let typed = view.composer.text().trim().to_string();
                    let chosen = self
                        .listed(view)
                        .get(sel)
                        .map(|m| m.id.clone())
                        .or_else(|| (!typed.is_empty()).then_some(typed));
                    if let Some(id) = chosen {
                        self.set_model(&id);
                        self.picking = None;
                        view.composer.clear();
                    }
                    Outcome::Stay
                }
                _ => {
                    if super::edit_field(&mut view.composer, key) {
                        self.picking = Some(0);
                    }
                    Outcome::Stay
                }
            };
        }
        if self.editing {
            return match key.code {
                KeyCode::Esc => {
                    self.editing = false;
                    view.composer.clear();
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    let text = view.composer.text().trim().to_string();
                    if self.form.set_from_text(&text) {
                        self.editing = false;
                        view.composer.clear();
                    }
                    Outcome::Stay
                }
                _ => {
                    super::edit_field(&mut view.composer, key);
                    Outcome::Stay
                }
            };
        }
        match key.code {
            KeyCode::Esc => Outcome::Close,
            KeyCode::Up | KeyCode::BackTab => {
                self.form.step(-1);
                Outcome::Stay
            }
            KeyCode::Down | KeyCode::Tab => {
                self.form.step(1);
                Outcome::Stay
            }
            KeyCode::Left => {
                self.adjust(view, -1);
                Outcome::Stay
            }
            KeyCode::Right | KeyCode::Char(' ') => {
                self.adjust(view, 1);
                Outcome::Stay
            }
            KeyCode::Enter => match self.form.current().map(|f| (f.id, &f.kind)) {
                Some(("start", _)) => self.start(view),
                Some(("model", _)) => {
                    self.open_list(view);
                    Outcome::Stay
                }
                Some((_, Kind::Select { .. })) => {
                    self.adjust(view, 1);
                    Outcome::Stay
                }
                Some((_, Kind::Text(_) | Kind::Number { .. })) => {
                    let v = self
                        .form
                        .current()
                        .map(Field::value_text)
                        .unwrap_or_default();
                    view.composer.set_text(&v);
                    self.editing = true;
                    Outcome::Stay
                }
                _ => Outcome::Stay,
            },
            _ => Outcome::Stay,
        }
    }

    fn on_notice(&mut self, n: &Notice, view: &mut View) {
        match n {
            Notice::Models {
                models,
                covered,
                failed,
            } => self.set_models(view, models, covered, failed),
            Notice::Bench(progress) => {
                if !matches!(self.phase, Phase::Running { .. }) {
                    if !matches!(progress, Progress::Started { .. } | Progress::Finished(_)) {
                        return;
                    }
                    // A run started from this or an earlier panel: follow it
                    // from here on with the tasks it will cover.
                    self.phase = Phase::Running {
                        rows: Self::rows_for(&ryter_bench::select(&[], self.level())),
                        spent: 0.0,
                        activity: String::new(),
                        confirm_stop: false,
                    };
                }
                let Phase::Running {
                    rows,
                    spent,
                    activity,
                    ..
                } = &mut self.phase
                else {
                    return;
                };
                let row = |rows: &mut Vec<Row>, name: &str| -> usize {
                    if let Some(i) = rows.iter().position(|r| r.name == name) {
                        return i;
                    }
                    rows.push(Row {
                        name: name.to_string(),
                        level: Level::Standard,
                        state: State::Waiting,
                        usd: None,
                        seconds: 0.0,
                    });
                    rows.len() - 1
                };
                match progress {
                    Progress::Started { name, .. } => {
                        let i = row(rows, name);
                        rows[i].state = State::Running;
                        activity.clear();
                    }
                    Progress::Activity { what, .. } => *activity = what.clone(),
                    Progress::Spent { name, usd } => {
                        let i = row(rows, name);
                        rows[i].usd = Some(*usd);
                    }
                    Progress::Scoring { name } => {
                        let i = row(rows, name);
                        rows[i].state = State::Scoring;
                        activity.clear();
                    }
                    Progress::Finished(result) => {
                        let i = row(rows, &result.name);
                        rows[i].level = result.level;
                        rows[i].usd = result.usd;
                        rows[i].seconds = result.seconds;
                        rows[i].state = State::Done(result.clone());
                        if let Some(u) = result.usd {
                            *spent += u;
                        }
                        activity.clear();
                    }
                    Progress::Stopped { why } => *activity = format!("stopped: {why}"),
                }
            }
            Notice::BenchDone(report) => self.phase = Phase::Done(report.clone()),
            _ => {}
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

    fn row(id: &str, conn: &str, rates: (f64, f64), tools: bool) -> ModelInfo {
        ModelInfo {
            id: id.into(),
            context_length: Some(200_000),
            input_per_million: Some(rates.0),
            output_per_million: Some(rates.1),
            cached_per_million: None,
            cache_write_per_million: None,
            connection: Some(conn.into()),
            created: None,
            tools: Some(tools),
        }
    }

    /// Two keyed connections; the session (and so the build hat) on
    /// spacexsi's grok-4.6.
    fn two_connections() -> View {
        let mut v = View::new("spacexsi".into(), "grok-4.6".into(), "/tmp".into());
        for (name, model) in [("spacexsi", "grok-4.6"), ("openrouter", "qwen/qwen3.7-max")] {
            v.connections.push(crate::view::ConnRow {
                name: name.into(),
                kind: name.into(),
                model: model.into(),
                has_key: true,
            });
        }
        v
    }

    fn lists(p: &mut Benchmark, v: &mut View) {
        p.set_models(
            v,
            &[
                row("grok-4.6", "spacexsi", (2.0, 6.0), true),
                row("grok-4.3", "spacexsi", (1.25, 2.5), true),
                row("qwen/qwen3.7-max", "openrouter", (1.6, 6.4), true),
                row("minimax/minimax-m2.7", "openrouter", (0.3, 1.2), true),
                row("openai/gpt-6-image", "openrouter", (5.0, 15.0), false),
            ],
            &["spacexsi".into(), "openrouter".into()],
            &[],
        );
    }

    fn key(p: &mut Benchmark, v: &mut View, code: KeyCode) -> Outcome {
        p.key(KeyEvent::new(code, KeyModifiers::NONE), v)
    }

    fn typed(p: &mut Benchmark, v: &mut View, text: &str) {
        for ch in text.chars() {
            key(p, v, KeyCode::Char(ch));
        }
    }

    fn text(p: &Benchmark, v: &View) -> String {
        p.render(v, 84, 24, Theme::truecolor_dark())
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

    /// Enter on the model opens the connection's list with context and
    /// prices, as `/models` shows them; typing filters it; Enter takes
    /// the row under the cursor into the field.
    #[test]
    fn enter_on_the_model_lists_the_connections_models_with_prices() {
        let mut v = two_connections();
        let mut p = Benchmark::new(&v);
        lists(&mut p, &mut v);
        key(&mut p, &mut v, KeyCode::Down);
        key(&mut p, &mut v, KeyCode::Enter);
        assert_eq!(p.input(&v).as_deref(), Some("filter"));
        let t = text(&p, &v);
        assert!(t.contains("model on spacexsi"), "{t}");
        assert!(
            t.contains("grok-4.6") && t.contains("$2") && t.contains("$6"),
            "{t}"
        );
        assert!(t.contains("grok-4.3") && t.contains("$1.25"), "{t}");
        assert!(!t.contains("qwen"), "another connection's rows: {t}");
        assert!(!p.loading);
        typed(&mut p, &mut v, "4.3");
        let t = text(&p, &v);
        assert!(t.contains("grok-4.3") && !t.contains("grok-4.6"), "{t}");
        key(&mut p, &mut v, KeyCode::Enter);
        assert_eq!(p.model(), "grok-4.3");
        assert!(p.picking.is_none());
        assert!(v.composer.text().is_empty());
        let t = text(&p, &v);
        assert!(t.contains("grok-4.3") && t.contains("start the run"), "{t}");
    }

    /// Esc leaves the field as it was; a filter nothing matches lets Enter
    /// take the typed id, for a model the list leaves out.
    #[test]
    fn esc_keeps_the_model_and_an_unlisted_id_can_be_typed() {
        let mut v = two_connections();
        let mut p = Benchmark::new(&v);
        lists(&mut p, &mut v);
        key(&mut p, &mut v, KeyCode::Down);
        key(&mut p, &mut v, KeyCode::Enter);
        key(&mut p, &mut v, KeyCode::Down);
        key(&mut p, &mut v, KeyCode::Esc);
        assert_eq!(p.model(), "grok-4.6");
        key(&mut p, &mut v, KeyCode::Enter);
        typed(&mut p, &mut v, "grok-5-preview");
        let t = text(&p, &v);
        assert!(t.contains("no listed model matches"), "{t}");
        key(&mut p, &mut v, KeyCode::Enter);
        assert_eq!(p.model(), "grok-5-preview");
    }

    /// Changing the connection puts its default model in the field: the
    /// last connection's model is not on this one.
    #[test]
    fn a_new_connection_brings_its_own_model() {
        let mut v = two_connections();
        let mut p = Benchmark::new(&v);
        lists(&mut p, &mut v);
        key(&mut p, &mut v, KeyCode::Right);
        assert_eq!(p.connection(&v), "openrouter");
        assert_eq!(p.model(), "qwen/qwen3.7-max");
        key(&mut p, &mut v, KeyCode::Down);
        key(&mut p, &mut v, KeyCode::Enter);
        let t = text(&p, &v);
        assert!(t.contains("model on openrouter"), "{t}");
        assert!(
            t.contains("minimax/minimax-m2.7") && t.contains("$0.3"),
            "{t}"
        );
        assert!(!t.contains("grok-4.6"), "{t}");
        key(&mut p, &mut v, KeyCode::Esc);
        key(&mut p, &mut v, KeyCode::Up);
        key(&mut p, &mut v, KeyCode::Left);
        assert_eq!(p.model(), "grok-4.6");
    }

    /// A model the catalog says can't call tools is refused at the start.
    #[test]
    fn a_toolless_model_is_refused() {
        let mut v = two_connections();
        let mut p = Benchmark::new(&v);
        lists(&mut p, &mut v);
        key(&mut p, &mut v, KeyCode::Right);
        p.set_model("openai/gpt-6-image");
        while p.form.current().is_some_and(|f| f.id != "start") {
            p.form.step(1);
        }
        assert!(matches!(key(&mut p, &mut v, KeyCode::Enter), Outcome::Stay));
        assert!(
            v.messages
                .iter()
                .any(|m| m.body.contains("can't call tools")),
            "{:?}",
            v.messages
                .iter()
                .map(|m| m.body.clone())
                .collect::<Vec<_>>()
        );
    }

    /// Until the lists come the built-in rows stand and the list says it
    /// is loading; a one-connection list replaces that connection's rows
    /// and keeps the other's.
    #[test]
    fn the_lists_arrive_per_connection() {
        let mut v = two_connections();
        let mut p = Benchmark::new(&v);
        assert!(p.loading);
        key(&mut p, &mut v, KeyCode::Down);
        key(&mut p, &mut v, KeyCode::Enter);
        let t = text(&p, &v);
        assert!(
            t.contains("loading models") && t.contains("grok-4.6"),
            "{t}"
        );
        p.set_models(
            &v,
            &[row("grok-4.3", "spacexsi", (1.25, 2.5), true)],
            &["spacexsi".into()],
            &[],
        );
        assert!(p.loading, "openrouter's list is still to come");
        let t = text(&p, &v);
        assert!(t.contains("grok-4.3") && !t.contains("grok-4.6"), "{t}");
        p.set_models(&v, &[], &[], &["openrouter".into()]);
        assert!(!p.loading);
        key(&mut p, &mut v, KeyCode::Esc);
        key(&mut p, &mut v, KeyCode::Up);
        key(&mut p, &mut v, KeyCode::Right);
        key(&mut p, &mut v, KeyCode::Down);
        key(&mut p, &mut v, KeyCode::Enter);
        let t = text(&p, &v);
        assert!(
            t.contains("qwen/qwen3.7-max"),
            "the built-in row stays: {t}"
        );
    }
}
