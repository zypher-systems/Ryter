//! `/benchmark` — the bench's tasks on a model of your choice, from inside
//! the TUI: pick the connection, the model, the level and a budget, and
//! watch tasks solved per dollar come in. The run is `ryter bench`'s, in a
//! thread; its table lands under `~/.ryter/bench/` and its headline in the
//! chat.

use crossterm::event::{KeyCode, KeyEvent};
use ryter_bench::{Level, Progress, Report, TaskResult};

use super::widgets::{self, Al, Field, Form, Kind};
use super::{Body, Notice, Outcome, Panel};
use crate::action::Action;
use crate::chat::wrap;
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
    /// Editing the model via the composer.
    editing: bool,
    phase: Phase,
    /// The keyed connections, in the select's order.
    connections: Vec<String>,
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
        }
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
        let tasks = ryter_bench::select(&[], self.level()).len();
        if tasks == 0 {
            view.error("no task at that level");
            return Outcome::Stay;
        }
        Outcome::Act(Action::StartBench {
            connection: self
                .connections
                .get(self.select_idx("connection"))
                .cloned()
                .unwrap_or_else(|| view.connection.clone()),
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
            Phase::Setup if self.editing => "type · enter apply · esc cancel".into(),
            Phase::Setup => "↑↓ move · ←→ change · enter edit/start · esc close".into(),
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
        self.editing.then(|| "model".to_string())
    }

    fn prefill(&self, _view: &View) -> Option<String> {
        self.editing
            .then(|| self.form.current().map(Field::value_text))
            .flatten()
    }

    fn render(&self, view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let mut lines = match &self.phase {
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
                self.form.adjust(-1);
                Outcome::Stay
            }
            KeyCode::Right | KeyCode::Char(' ') => {
                self.form.adjust(1);
                Outcome::Stay
            }
            KeyCode::Enter => match self.form.current().map(|f| (f.id, &f.kind)) {
                Some(("start", _)) => self.start(view),
                Some((_, Kind::Select { .. })) => {
                    self.form.adjust(1);
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

    fn on_notice(&mut self, n: &Notice, _view: &mut View) {
        match n {
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
