//! `/benchmark` — the bench's tasks on a model of your choice, from inside
//! the TUI: pick the connection, the model, the level and a budget, and
//! watch tasks solved per dollar come in. The run is `ryter bench`'s, in a
//! thread; its table lands under `~/.ryter/bench/` and its headline in the
//! chat. The models are picked from each connection's list, with context
//! and prices, as `/models` shows it; several run in turn and are compared,
//! so the cheapest one with a clean sheet can be seated for the job.

use std::collections::{BTreeMap, BTreeSet};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::text::{Line, Span};
use ryter_bench::{Comparison, Level, Progress, Report, Seat, TaskResult};
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

/// A model whose run is over, while the next runs.
#[derive(Debug, Clone, PartialEq)]
struct ModelLine {
    seat: Seat,
    solved: usize,
    tasks: usize,
    usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
enum Phase {
    /// The form.
    Setup,
    /// A run is going.
    Running {
        /// The model on now, of `models` compared.
        model: Option<Seat>,
        models: usize,
        /// The models done so far, in order.
        finished: Vec<ModelLine>,
        rows: Vec<Row>,
        spent: f64,
        activity: String,
        /// Esc asked to stop; waiting for the answer.
        confirm_stop: bool,
    },
    /// The run finished.
    Done(Box<Comparison>),
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
    /// The models to run, each on its connection, in the order picked.
    seats: Vec<Seat>,
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
        let seats = if model.is_empty() {
            Vec::new()
        } else {
            vec![Seat::new(&connection, &model)]
        };
        let fields = vec![
            Field::new(
                "connection",
                "connection",
                Kind::Select {
                    options: connections.clone(),
                    idx,
                },
            ),
            Field::new("models", "models", Kind::Text(String::new())),
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
        let mut p = Self {
            form: Form::new(fields),
            editing: false,
            phase: Phase::Setup,
            connections,
            seats,
            catalog,
            loading,
            settled: BTreeSet::new(),
            picking: None,
        };
        p.show_seats(view);
        p
    }

    /// The seats as the form shows them: the model, with its connection
    /// when it is not the form's.
    fn seats_text(&self, view: &View) -> String {
        let home = self.connection(view);
        self.seats
            .iter()
            .map(|s| s.label(&home))
            .collect::<Vec<_>>()
            .join(" · ")
    }

    fn show_seats(&mut self, view: &View) {
        let text = self.seats_text(view);
        if let Some(Kind::Text(t)) = self.form.get_mut("models").map(|f| &mut f.kind) {
            *t = text;
        }
    }

    /// Add the seat, or remove it when it is picked already.
    fn toggle_seat(&mut self, view: &View, seat: Seat) {
        match self.seats.iter().position(|s| *s == seat) {
            Some(i) => {
                self.seats.remove(i);
            }
            None => self.seats.push(seat),
        }
        self.show_seats(view);
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

    /// Open the model list, the cursor on the first model picked from this
    /// connection, or at the top.
    fn open_list(&mut self, view: &mut View) {
        view.composer.clear();
        let connection = self.connection(view);
        let first = self
            .seats
            .iter()
            .find(|s| s.connection == connection)
            .map(|s| s.model.clone());
        self.picking = Some(
            first
                .and_then(|m| self.listed(view).iter().position(|r| r.id == m))
                .unwrap_or(0),
        );
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
        let connection = self.connection(view);
        let picked = |id: &str| {
            self.seats
                .iter()
                .any(|s| s.connection == connection && s.model == id)
        };
        let rows: Vec<Vec<String>> = list
            .iter()
            .skip(first)
            .take(rows_h)
            .map(|m| {
                vec![
                    if picked(&m.id) { "✓" } else { "" }.to_string(),
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
            &format!("models on {connection} · enter adds or removes · type to filter"),
            theme,
        )];
        let mut table = widgets::table(
            &["", "model", "context", "in/M", "out/M"],
            &rows,
            &[Al::L, Al::L, Al::R, Al::R, Al::R],
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
        if self.seats.is_empty() {
            view.error("pick a model to run the bench on");
            return Outcome::Stay;
        }
        if let Some(seat) = self.seats.iter().find(|s| {
            self.catalog.iter().any(|m| {
                m.id == s.model
                    && m.connection.as_deref() == Some(s.connection.as_str())
                    && m.tools == Some(false)
            })
        }) {
            view.error(format!(
                "{} can't call tools, so it can't read a file or run a test",
                seat.model
            ));
            return Outcome::Stay;
        }
        let tasks = ryter_bench::select(&[], self.level()).len();
        if tasks == 0 {
            view.error("no task at that level");
            return Outcome::Stay;
        }
        Outcome::Act(Action::StartBench {
            seats: self.seats.clone(),
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

    /// `←`/`→` on the focused field; a new connection relabels the seats.
    fn adjust(&mut self, view: &View, delta: i32) {
        if self.form.adjust(delta) {
            self.show_seats(view);
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

    #[allow(clippy::too_many_arguments)]
    fn render_running(
        model: Option<&Seat>,
        models: usize,
        finished: &[ModelLine],
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
        let mut lines = Vec::new();
        if models > 1 {
            for f in finished {
                lines.push(widgets::note(
                    &format!(
                        "{} {} {}/{} for {}",
                        if f.solved == f.tasks && f.tasks > 0 {
                            "✓"
                        } else {
                            "✗"
                        },
                        f.seat.model,
                        f.solved,
                        f.tasks,
                        ryter_bench::report::usd(f.usd)
                    ),
                    theme,
                ));
            }
            if let Some(m) = model {
                lines.push(widgets::text(
                    &format!(
                        "model {} of {models}: {} ({})",
                        finished.len() + 1,
                        m.model,
                        m.connection
                    ),
                    theme,
                ));
            }
        }
        lines.push(widgets::text(
            &format!(
                "{done} of {} done · {solved} solved · spent {}",
                rows.len(),
                ryter_bench::report::usd(Some(spent))
            ),
            theme,
        ));
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

    /// Several models: a line per model, best first, with the tasks each
    /// did not solve; the task-by-task table is in the file.
    fn render_compared(
        c: &Comparison,
        w: usize,
        theme: Theme,
    ) -> Vec<ratatui::text::Line<'static>> {
        let mut lines: Vec<ratatui::text::Line<'static>> =
            wrap::wrap_plain(&c.headline(), w.saturating_sub(2))
                .iter()
                .map(|row| widgets::text(row, theme))
                .collect();
        lines.push(widgets::note(
            &format!(
                "{} · budget ${:.2} per model · {} s per task",
                c.ryter, c.budget_usd, c.timeout_seconds
            ),
            theme,
        ));
        lines.push(widgets::blank(theme));
        let table: Vec<Vec<String>> = c
            .ranked()
            .iter()
            .map(|row| {
                let s = &row.summary;
                let clean = s.tasks > 0 && s.solved == s.tasks && row.stopped.is_none();
                vec![
                    format!("{} {}", if clean { "✓" } else { "✗" }, row.seat.model),
                    row.seat.connection.clone(),
                    format!("{}/{}", s.solved, s.tasks),
                    ryter_bench::report::usd(s.usd),
                    ryter_bench::report::usd(s.usd_per_solved),
                    // The reason for a stop is in the headline above.
                    format!(
                        "{}{}",
                        row.failed.join(", "),
                        if row.stopped.is_some() {
                            " · stopped"
                        } else {
                            ""
                        }
                    ),
                ]
            })
            .collect();
        lines.extend(widgets::table(
            &[
                "model",
                "connection",
                "solved",
                "cost",
                "per solved",
                "not solved",
            ],
            &table,
            &[Al::L, Al::L, Al::R, Al::R, Al::R, Al::L],
            None,
            w,
            theme,
        ));
        lines.push(widgets::blank(theme));
        for row in wrap::wrap_plain(
            &format!(
                "the table, task by task, is in {}",
                c.directory.join("RESULTS.md").display()
            ),
            w.saturating_sub(2),
        ) {
            lines.push(widgets::note(&row, theme));
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
            Phase::Done(c) => match c.single() {
                Some(r) => format!("{}/{}", r.summary().solved, r.summary().tasks),
                None => format!("{} models", c.runs.len()),
            },
        }
    }

    fn legend(&self, _view: &View) -> String {
        match &self.phase {
            Phase::Setup if self.picking.is_some() => {
                "↑↓ move · type to filter · enter add/remove · esc done".into()
            }
            Phase::Setup if self.editing => "type · enter apply · esc cancel".into(),
            Phase::Setup => "↑↓ move · ←→ change · enter pick/start · ⌫ unpick · esc close".into(),
            Phase::Running {
                confirm_stop: true, ..
            } => "⏎ stop · n keep running".into(),
            Phase::Running { .. } => "esc stop · c close (the run goes on)".into(),
            Phase::Done(_) => "r run again · esc close".into(),
        }
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        match &self.phase {
            // Room for the models' lines and what each did not solve.
            Phase::Done(c) if c.single().is_none() => (104, 24),
            _ => (84, 24),
        }
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
                model,
                models,
                finished,
                rows,
                spent,
                activity,
                confirm_stop,
            } => Self::render_running(
                model.as_ref(),
                *models,
                finished,
                rows,
                *spent,
                activity,
                *confirm_stop,
                w,
                theme,
            ),
            Phase::Done(c) => match c.single() {
                Some(report) => Self::render_done(report, w, theme),
                None => Self::render_compared(c, w, theme),
            },
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
                    let listed = self.listed(view).get(sel).map(|m| m.id.clone());
                    match listed {
                        Some(id) => self.toggle_seat(view, Seat::new(self.connection(view), id)),
                        // A model the list leaves out: the typed id, and the
                        // filter goes so the list is whole again.
                        None if !typed.is_empty() => {
                            self.toggle_seat(view, Seat::new(self.connection(view), typed));
                            view.composer.clear();
                            self.picking = Some(0);
                        }
                        None => {}
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
            KeyCode::Backspace | KeyCode::Delete
                if self.form.current().is_some_and(|f| f.id == "models") =>
            {
                self.seats.pop();
                self.show_seats(view);
                Outcome::Stay
            }
            KeyCode::Enter => match self.form.current().map(|f| (f.id, &f.kind)) {
                Some(("start", _)) => self.start(view),
                Some(("models", _)) => {
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
                    if !matches!(
                        progress,
                        Progress::Model { .. } | Progress::Started { .. } | Progress::Finished(_)
                    ) {
                        return;
                    }
                    // A run started from this or an earlier panel: follow it
                    // from here on with the tasks it will cover.
                    self.phase = Phase::Running {
                        model: None,
                        models: 1,
                        finished: Vec::new(),
                        rows: Self::rows_for(&ryter_bench::select(&[], self.level())),
                        spent: 0.0,
                        activity: String::new(),
                        confirm_stop: false,
                    };
                }
                let level = self.level();
                let Phase::Running {
                    model,
                    models,
                    finished,
                    rows,
                    spent,
                    activity,
                    ..
                } = &mut self.phase
                else {
                    return;
                };
                if let Progress::Model {
                    connection,
                    model: id,
                    total,
                    ..
                } = progress
                {
                    // The last model's rows fold into its line; the next
                    // starts on fresh rows.
                    if let Some(last) = model.take() {
                        let scored: Vec<&TaskResult> = rows
                            .iter()
                            .filter_map(|r| match &r.state {
                                State::Done(t) if t.solved.is_some() => Some(t),
                                _ => None,
                            })
                            .collect();
                        finished.push(ModelLine {
                            seat: last,
                            solved: scored.iter().filter(|t| t.solved == Some(true)).count(),
                            tasks: scored.len(),
                            usd: scored
                                .iter()
                                .map(|t| t.usd)
                                .sum::<Option<f64>>()
                                .filter(|_| !scored.is_empty()),
                        });
                    }
                    *model = Some(Seat::new(connection, id));
                    *models = *total;
                    *rows = Self::rows_for(&ryter_bench::select(&[], level));
                    *spent = 0.0;
                    activity.clear();
                    return;
                }
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
                    Progress::Model { .. } => {}
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

    fn seats(p: &Benchmark) -> Vec<String> {
        p.seats
            .iter()
            .map(|s| format!("{}:{}", s.connection, s.model))
            .collect()
    }

    /// The form opens on the build hat's model; Enter on the models row
    /// lists the connection's models with context and prices, as `/models`
    /// shows them, the picked one marked; typing filters; Enter adds the
    /// row under the cursor, and the list stays open for the next.
    #[test]
    fn enter_on_the_models_lists_the_connections_models_with_prices() {
        let mut v = two_connections();
        let mut p = Benchmark::new(&v);
        lists(&mut p, &mut v);
        assert_eq!(seats(&p), vec!["spacexsi:grok-4.6"]);
        key(&mut p, &mut v, KeyCode::Down);
        key(&mut p, &mut v, KeyCode::Enter);
        assert_eq!(p.input(&v).as_deref(), Some("filter"));
        let t = text(&p, &v);
        assert!(t.contains("models on spacexsi"), "{t}");
        assert!(
            t.contains("✓  grok-4.6") && t.contains("$2") && t.contains("$6"),
            "{t}"
        );
        assert!(t.contains("grok-4.3") && t.contains("$1.25"), "{t}");
        assert!(!t.contains("qwen"), "another connection's rows: {t}");
        typed(&mut p, &mut v, "4.3");
        let t = text(&p, &v);
        assert!(t.contains("grok-4.3") && !t.contains("grok-4.6"), "{t}");
        key(&mut p, &mut v, KeyCode::Enter);
        assert_eq!(seats(&p), vec!["spacexsi:grok-4.6", "spacexsi:grok-4.3"]);
        assert!(p.picking.is_some(), "the list stays open for the next pick");
        assert!(text(&p, &v).contains("✓  grok-4.3"));
        // Enter again takes it out.
        key(&mut p, &mut v, KeyCode::Enter);
        assert_eq!(seats(&p), vec!["spacexsi:grok-4.6"]);
        key(&mut p, &mut v, KeyCode::Esc);
        assert!(p.picking.is_none());
        assert!(v.composer.text().is_empty());
        let t = text(&p, &v);
        assert!(t.contains("grok-4.6") && t.contains("start the run"), "{t}");
    }

    /// A filter nothing matches lets Enter add the typed id, for a model
    /// the list leaves out; Backspace on the models row removes the last.
    #[test]
    fn an_unlisted_id_can_be_typed_and_backspace_removes_the_last() {
        let mut v = two_connections();
        let mut p = Benchmark::new(&v);
        lists(&mut p, &mut v);
        key(&mut p, &mut v, KeyCode::Down);
        key(&mut p, &mut v, KeyCode::Enter);
        typed(&mut p, &mut v, "grok-5-preview");
        let t = text(&p, &v);
        assert!(t.contains("no listed model matches"), "{t}");
        key(&mut p, &mut v, KeyCode::Enter);
        assert_eq!(
            seats(&p),
            vec!["spacexsi:grok-4.6", "spacexsi:grok-5-preview"]
        );
        assert!(
            v.composer.text().is_empty(),
            "the filter goes with the pick"
        );
        key(&mut p, &mut v, KeyCode::Esc);
        key(&mut p, &mut v, KeyCode::Backspace);
        assert_eq!(seats(&p), vec!["spacexsi:grok-4.6"]);
        key(&mut p, &mut v, KeyCode::Backspace);
        assert!(seats(&p).is_empty());
        while p.form.current().is_some_and(|f| f.id != "start") {
            p.form.step(1);
        }
        assert!(matches!(key(&mut p, &mut v, KeyCode::Enter), Outcome::Stay));
        assert!(
            v.messages.iter().any(|m| m.body.contains("pick a model")),
            "no seat, no run"
        );
    }

    /// Models from two connections sit side by side: the form names the
    /// connection of a seat that is not on the form's, and the start
    /// carries every seat with its own connection.
    #[test]
    fn seats_from_two_connections_are_compared() {
        let mut v = two_connections();
        let mut p = Benchmark::new(&v);
        lists(&mut p, &mut v);
        key(&mut p, &mut v, KeyCode::Right);
        assert_eq!(p.connection(&v), "openrouter");
        let t = text(&p, &v);
        assert!(t.contains("grok-4.6 (spacexsi)"), "{t}");
        key(&mut p, &mut v, KeyCode::Down);
        key(&mut p, &mut v, KeyCode::Enter);
        let t = text(&p, &v);
        assert!(t.contains("models on openrouter"), "{t}");
        assert!(
            t.contains("minimax/minimax-m2.7") && t.contains("$0.3"),
            "{t}"
        );
        assert!(!t.contains("grok-4.6"), "{t}");
        typed(&mut p, &mut v, "minimax");
        key(&mut p, &mut v, KeyCode::Enter);
        key(&mut p, &mut v, KeyCode::Esc);
        let t = text(&p, &v);
        assert!(
            t.contains("grok-4.6 (spacexsi) · minimax/minimax-m2.7"),
            "{t}"
        );
        while p.form.current().is_some_and(|f| f.id != "start") {
            p.form.step(1);
        }
        match key(&mut p, &mut v, KeyCode::Enter) {
            Outcome::Act(Action::StartBench { seats, .. }) => assert_eq!(
                seats,
                vec![
                    Seat::new("spacexsi", "grok-4.6"),
                    Seat::new("openrouter", "minimax/minimax-m2.7")
                ]
            ),
            _ => panic!("the start carries the seats"),
        }
    }

    /// A model the catalog says can't call tools is refused at the start,
    /// by name.
    #[test]
    fn a_toolless_model_is_refused() {
        let mut v = two_connections();
        let mut p = Benchmark::new(&v);
        lists(&mut p, &mut v);
        p.toggle_seat(&v, Seat::new("openrouter", "openai/gpt-6-image"));
        while p.form.current().is_some_and(|f| f.id != "start") {
            p.form.step(1);
        }
        assert!(matches!(key(&mut p, &mut v, KeyCode::Enter), Outcome::Stay));
        assert!(
            v.messages
                .iter()
                .any(|m| m.body.contains("openai/gpt-6-image can't call tools")),
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
