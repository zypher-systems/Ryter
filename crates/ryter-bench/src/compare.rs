//! Several models on the same tasks: one run per model, in turn, and the
//! table that says who solved what for how much. The point is choosing a
//! model for a job: the cheapest one with a clean sheet is the one to seat.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::report::{Report, Summary, usd};
use crate::run::{Options, Progress, run};
use crate::task::{Level, Task};

/// A model on a connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seat {
    pub connection: String,
    pub model: String,
}

impl Seat {
    pub fn new(connection: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            connection: connection.into(),
            model: model.into(),
        }
    }

    /// `model`, or `model (connection)` when the connection is not `home`.
    pub fn label(&self, home: &str) -> String {
        if self.connection == home {
            self.model.clone()
        } else {
            format!("{} ({})", self.model, self.connection)
        }
    }
}

/// The runs of several models on the same tasks, with the same budget each.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Comparison {
    pub date: String,
    pub ryter: String,
    /// The level run, or every level.
    pub level: Option<Level>,
    /// Per model.
    pub budget_usd: f64,
    pub timeout_seconds: u64,
    pub sandbox: String,
    /// How many models were to run; `runs` is short of it when the
    /// comparison stopped early.
    #[serde(default)]
    pub seats: usize,
    /// One per model, in the order run; a model not reached has no run.
    pub runs: Vec<Report>,
    /// Why the comparison ended before its last model.
    pub stopped: Option<String>,
    /// Why the comparison could not be written, when a save failed.
    #[serde(skip)]
    pub save_error: Option<String>,
    /// Where the runs are: one folder per model under it, or the one run's
    /// own folder when there was one model.
    pub directory: PathBuf,
}

/// One model's line in the comparison.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelRow {
    pub seat: Seat,
    pub summary: Summary,
    /// The tasks it did not solve, scored or skipped.
    pub failed: Vec<String>,
    pub stopped: Option<String>,
}

impl Comparison {
    /// The one run, when one model was compared with nobody. Several
    /// models of which one ran before a cancel are still a comparison.
    pub fn single(&self) -> Option<&Report> {
        match self.runs.as_slice() {
            [one] if self.seats <= 1 => Some(one),
            _ => None,
        }
    }

    /// Whether anything did not go to its end: the comparison stopped, a
    /// model's run stopped, or a report could not be written.
    pub fn ended_early(&self) -> bool {
        self.stopped.is_some()
            || self.save_error.is_some()
            || self.runs.iter().any(Report::ended_early)
    }

    /// The tasks every run covers, in the first run's order.
    pub fn tasks(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for r in &self.runs {
            for t in &r.results {
                if !names.contains(&t.name) {
                    names.push(t.name.clone());
                }
            }
        }
        names
    }

    pub fn rows(&self) -> Vec<ModelRow> {
        self.runs
            .iter()
            .map(|r| ModelRow {
                seat: Seat::new(&r.connection, &r.model),
                summary: r.summary(),
                failed: r
                    .results
                    .iter()
                    .filter(|t| t.solved != Some(true))
                    .map(|t| t.name.clone())
                    .collect(),
                stopped: r.stopped.clone(),
            })
            .collect()
    }

    /// The models ranked for the job: most solved first, then the cheapest.
    pub fn ranked(&self) -> Vec<ModelRow> {
        let mut rows = self.rows();
        rows.sort_by(|a, b| {
            b.summary.solved.cmp(&a.summary.solved).then_with(|| {
                match (a.summary.usd, b.summary.usd) {
                    (Some(x), Some(y)) => x.total_cmp(&y),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
            })
        });
        rows
    }

    /// The table: a line per model, then the tasks against the models.
    /// One model: its own report's table.
    pub fn markdown(&self) -> String {
        if let Some(one) = self.single() {
            return one.markdown();
        }
        let which = match self.level {
            Some(l) => format!("the {l} tasks"),
            None => "every task".to_string(),
        };
        let home = self.home();
        let mut lines = vec![
            format!("# Ryter bench — {} models compared — {}", self.runs.len(), self.date),
            String::new(),
            format!(
                "Ryter {} on {which}; budget ${:.2} per model; {} s per task; one attempt per task; sandbox {}.",
                self.ryter, self.budget_usd, self.timeout_seconds, self.sandbox
            ),
            String::new(),
            "| Model | Connection | Solved | Cost | Cost per solved | Solved per $ | Median minutes | Stopped |"
                .into(),
            "|---|---|---|---|---|---|---|---|".into(),
        ];
        for row in self.ranked() {
            let s = &row.summary;
            lines.push(format!(
                "| `{}` | {} | {}/{} | {} | {} | {} | {} | {} |",
                row.seat.model,
                row.seat.connection,
                s.solved,
                s.tasks,
                usd(s.usd),
                usd(s.usd_per_solved),
                one(s.solved_per_usd),
                one(s.median_minutes),
                row.stopped.as_deref().unwrap_or("")
            ));
        }
        let tasks = self.tasks();
        if !tasks.is_empty() {
            lines.push(String::new());
            let mut head = "| Task |".to_string();
            let mut rule = "|---|".to_string();
            for r in &self.runs {
                head.push_str(&format!(
                    " `{}` |",
                    Seat::new(&r.connection, &r.model).label(&home)
                ));
                rule.push_str("---|");
            }
            lines.push(head);
            lines.push(rule);
            for name in &tasks {
                let mut line = format!("| {name} |");
                for r in &self.runs {
                    let cell = match r.results.iter().find(|t| &t.name == name) {
                        Some(t) => match t.solved {
                            Some(true) => format!("✓ {}", usd(t.usd)),
                            Some(false) if t.timed_out => format!("✗ timed out {}", usd(t.usd)),
                            Some(false) => format!("✗ {}", usd(t.usd)),
                            None => "skipped".to_string(),
                        },
                        None => "not run".to_string(),
                    };
                    line.push_str(&format!(" {cell} |"));
                }
                lines.push(line);
            }
        }
        if let Some(why) = &self.stopped {
            lines.push(String::new());
            lines.push(format!("Stopped: {why}."));
        }
        lines.join("\n") + "\n"
    }

    /// The connection most runs are on: the labels leave it out.
    fn home(&self) -> String {
        let mut counts: Vec<(String, usize)> = Vec::new();
        for r in &self.runs {
            match counts.iter_mut().find(|(c, _)| c == &r.connection) {
                Some((_, n)) => *n += 1,
                None => counts.push((r.connection.clone(), 1)),
            }
        }
        counts
            .into_iter()
            .max_by_key(|(_, n)| *n)
            .map(|(c, _)| c)
            .unwrap_or_default()
    }

    /// One line for the chat or the terminal: each model's score and cost,
    /// best first. One model: its report's line.
    pub fn headline(&self) -> String {
        if let Some(one) = self.single() {
            return one.headline();
        }
        let home = self.home();
        let tasks = self.tasks().len();
        let mut parts: Vec<String> = self
            .ranked()
            .iter()
            .map(|row| {
                let s = &row.summary;
                let mut p = format!(
                    "{} {}/{} for {}",
                    row.seat.label(&home),
                    s.solved,
                    s.tasks,
                    usd(s.usd)
                );
                if row.stopped.is_some() {
                    p.push_str(" (stopped)");
                }
                p
            })
            .collect();
        if let Some(why) = &self.stopped {
            parts.push(format!("stopped: {why}"));
        }
        if let Some(e) = &self.save_error {
            parts.push(format!("the comparison could not be written: {e}"));
        }
        let models = if self.runs.len() == self.seats.max(1) {
            format!("{} models", self.runs.len())
        } else {
            format!("{} of {} models", self.runs.len(), self.seats)
        };
        format!(
            "{models} on {tasks} task{}: {}",
            if tasks == 1 { "" } else { "s" },
            parts.join(" · ")
        )
    }

    /// Write `compare.json` and `RESULTS.md` into `dir`; returns the
    /// table's path. One model's comparison is its report, saved in its
    /// own folder.
    pub fn save(&self, dir: &Path) -> std::io::Result<PathBuf> {
        if let Some(one) = self.single() {
            return one.save(&one.directory);
        }
        std::fs::create_dir_all(dir)?;
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(dir.join("compare.json"), json + "\n")?;
        let md = dir.join("RESULTS.md");
        std::fs::write(&md, self.markdown())?;
        Ok(md)
    }
}

fn one(v: Option<f64>) -> String {
    v.map_or_else(|| "—".to_string(), |v| format!("{v:.1}"))
}

/// A model id as a folder name.
pub fn slug(model: &str) -> String {
    let mut out = String::new();
    for c in model.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.truncate(out.trim_end_matches('-').len());
    if out.is_empty() {
        "model".to_string()
    } else {
        out
    }
}

/// The run's folder name under the bench directory: the stamp and the
/// model, or `compare` for several.
pub fn folder(stamp: &str, seats: &[Seat]) -> String {
    match seats {
        [one] => format!("{stamp}-{}", slug(&one.model)),
        _ => format!("{stamp}-compare"),
    }
}

/// A folder name per seat, all different: the model's slug; the
/// connection's in front where two slugs meet (`a/b` and `a-b`, or one
/// model on two connections); a number after where they still do.
pub fn model_folders(seats: &[Seat]) -> Vec<String> {
    let slugs: Vec<String> = seats.iter().map(|s| slug(&s.model)).collect();
    let mut names: Vec<String> = (0..seats.len())
        .map(|i| {
            if slugs.iter().filter(|n| **n == slugs[i]).count() > 1 {
                format!("{}-{}", slug(&seats[i].connection), slugs[i])
            } else {
                slugs[i].clone()
            }
        })
        .collect();
    for i in 0..names.len() {
        let base = names[i].clone();
        let mut n = 2;
        while names[..i].contains(&names[i]) {
            names[i] = format!("{base}-{n}");
            n += 1;
        }
    }
    names
}

/// Every seat on the same tasks, in turn, each under `opts.budget_usd` of
/// its own; `opts.connection` and `opts.model` are the seat's, `opts.dir`
/// the comparison's, with a folder per model under it when there are
/// several. A cancel ends the model in progress after its task and skips
/// the rest. A model whose run stops (it did not start, or went unpriced)
/// does not stop the next: that may be the model's own trouble.
pub fn run_models(
    tasks: &[&Task],
    seats: &[Seat],
    opts: &Options,
    mut on: impl FnMut(Progress),
) -> Comparison {
    let level = match tasks.iter().map(|t| t.level).collect::<Vec<_>>().as_slice() {
        [] => None,
        [first, rest @ ..] if rest.iter().all(|l| l == first) => Some(*first),
        _ => None,
    };
    let mut comparison = Comparison {
        date: ryter_core::clock::today(),
        ryter: ryter_core::VERSION.to_string(),
        level,
        budget_usd: opts.budget_usd,
        timeout_seconds: opts.timeout.as_secs(),
        sandbox: opts.sandbox.clone(),
        seats: seats.len(),
        runs: Vec::new(),
        stopped: None,
        directory: opts.dir.clone(),
        save_error: None,
    };
    let total = seats.len();
    let folders = model_folders(seats);
    for (index, seat) in seats.iter().enumerate() {
        if opts.cancel.is_cancelled() {
            comparison.stopped = Some(format!("cancelled before {}", seat.model));
            break;
        }
        let mut own = opts.clone();
        own.connection = seat.connection.clone();
        own.model = seat.model.clone();
        if total > 1 {
            own.dir = opts.dir.join(&folders[index]);
        }
        on(Progress::Model {
            connection: seat.connection.clone(),
            model: seat.model.clone(),
            index,
            total,
        });
        let report = run(tasks, &own, &mut on);
        comparison.runs.push(report);
        if let Err(e) = comparison.save(&opts.dir) {
            comparison.save_error = Some(e.to_string());
        }
    }
    if let Some(why) = &comparison.stopped {
        on(Progress::Stopped { why: why.clone() });
    }
    match comparison.save(&opts.dir) {
        Ok(_) => comparison.save_error = None,
        Err(e) => comparison.save_error = Some(e.to_string()),
    }
    comparison
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::TaskResult;

    fn result(name: &str, solved: bool, usd: f64) -> TaskResult {
        TaskResult {
            name: name.into(),
            title: name.into(),
            level: Level::Hard,
            solved: Some(solved),
            usd: Some(usd),
            complete: true,
            calls: 3,
            input_tokens: 1,
            cached_tokens: 0,
            output_tokens: 1,
            seconds: 60.0,
            timed_out: false,
            cancelled: false,
            exit: Some(0),
            changed: vec![],
            acceptance: vec![],
            error: None,
        }
    }

    fn report(connection: &str, model: &str, results: Vec<TaskResult>) -> Report {
        Report {
            date: "2026-10-23".into(),
            ryter: "0.38.0".into(),
            connection: connection.into(),
            model: model.into(),
            level: Some(Level::Hard),
            budget_usd: 5.0,
            timeout_seconds: 600,
            sandbox: "workspace".into(),
            total_usd: results.iter().filter_map(|r| r.usd).sum(),
            results,
            stopped: None,
            directory: PathBuf::from("/tmp/x"),
            save_error: None,
        }
    }

    fn two() -> Comparison {
        Comparison {
            date: "2026-10-23".into(),
            ryter: "0.38.0".into(),
            level: Some(Level::Hard),
            budget_usd: 5.0,
            timeout_seconds: 600,
            sandbox: "workspace".into(),
            seats: 2,
            runs: vec![
                report(
                    "openrouter",
                    "deepseek/deepseek-pro-latest",
                    vec![
                        result("expr-precedence", true, 0.14),
                        result("ts-debounce", false, 0.135),
                    ],
                ),
                report(
                    "anthropic",
                    "claude-sonnet-5-5",
                    vec![
                        result("expr-precedence", true, 0.20),
                        result("ts-debounce", true, 0.25),
                    ],
                ),
            ],
            stopped: None,
            directory: PathBuf::from("/tmp/cmp"),
            save_error: None,
        }
    }

    /// Most solved first, then the cheapest; the matrix has a column per
    /// model and a row per task with the mark and the cost.
    #[test]
    fn the_table_ranks_by_solved_then_cost_and_crosses_tasks_with_models() {
        let c = two();
        let md = c.markdown();
        let sonnet = md
            .find("| `claude-sonnet-5-5` | anthropic | 2/2 | $0.450 |")
            .unwrap();
        let deepseek = md
            .find("| `deepseek/deepseek-pro-latest` | openrouter | 1/2 | $0.275 |")
            .unwrap();
        assert!(sonnet < deepseek, "{md}");
        assert!(md.contains("| ts-debounce | ✗ $0.135 | ✓ $0.250 |"), "{md}");
        assert!(md.contains("budget $5.00 per model"), "{md}");
        assert!(
            md.contains(
                "| Task | `deepseek/deepseek-pro-latest` | `claude-sonnet-5-5 (anthropic)` |"
            ) || md.contains(
                "| Task | `deepseek/deepseek-pro-latest (openrouter)` | `claude-sonnet-5-5` |"
            ),
            "{md}"
        );
    }

    #[test]
    fn the_headline_names_every_model_best_first() {
        let c = two();
        let h = c.headline();
        assert!(h.starts_with("2 models on 2 tasks: "), "{h}");
        let sonnet = h.find("claude-sonnet-5-5").unwrap();
        let deepseek = h.find("deepseek/deepseek-pro-latest").unwrap();
        assert!(sonnet < deepseek, "{h}");
        assert!(
            h.contains("2/2 for $0.450") && h.contains("1/2 for $0.275"),
            "{h}"
        );
    }

    /// One model compared with nobody is its own report: the same table and
    /// line `/benchmark` always showed.
    #[test]
    fn one_model_is_its_report() {
        let mut c = two();
        c.seats = 1;
        c.runs.truncate(1);
        assert_eq!(c.markdown(), c.runs[0].markdown());
        assert_eq!(c.headline(), c.runs[0].headline());
    }

    #[test]
    fn folders_are_the_model_or_compare() {
        let a = Seat::new("openrouter", "deepseek/deepseek-pro-latest");
        let b = Seat::new("anthropic", "claude-sonnet-5-5");
        assert_eq!(
            folder("2026-10-23-13-21", std::slice::from_ref(&a)),
            "2026-10-23-13-21-deepseek-deepseek-pro-latest"
        );
        assert_eq!(
            folder("2026-10-23-13-21", &[a, b]),
            "2026-10-23-13-21-compare"
        );
    }

    #[test]
    fn one_model_of_two_after_a_cancel_is_still_a_comparison() {
        let mut c = two();
        c.runs.truncate(1);
        c.stopped = Some("cancelled before claude-sonnet-5-5".into());
        assert!(c.single().is_none());
        assert!(c.ended_early());
        let line = c.headline();
        assert!(
            line.starts_with("1 of 2 models on")
                && line.ends_with("stopped: cancelled before claude-sonnet-5-5"),
            "{line}"
        );
        assert!(c.markdown().contains("| Model |"), "the table is written");
        let dir = tempfile::tempdir().unwrap();
        c.save(dir.path()).unwrap();
        assert!(
            dir.path().join("compare.json").is_file() && dir.path().join("RESULTS.md").is_file()
        );
        let mut one = two();
        one.seats = 1;
        one.runs.truncate(1);
        assert!(one.single().is_some());
        assert_eq!(one.headline(), one.runs[0].headline());
    }

    #[test]
    fn model_folders_never_meet() {
        let seats = [
            Seat::new("openrouter", "a/b"),
            Seat::new("openrouter", "a-b"),
            Seat::new("anthropic", "claude"),
            Seat::new("openrouter", "claude"),
            Seat::new("openrouter-", "a-b"),
        ];
        let names = model_folders(&seats);
        assert_eq!(
            names,
            [
                "openrouter-a-b",
                "openrouter-a-b-2",
                "anthropic-claude",
                "openrouter-claude",
                "openrouter-a-b-3"
            ]
        );
        assert_eq!(model_folders(&seats[2..3]), ["claude"]);
    }

    #[test]
    fn a_slug_never_starts_with_a_dash() {
        assert_eq!(slug("~openai/gpt-luna-latest"), "openai-gpt-luna-latest");
        assert_eq!(
            slug("deepseek/deepseek-v4.1-flash-1017"),
            "deepseek-deepseek-v4-1-flash-1017"
        );
        assert_eq!(slug("a//b--"), "a-b");
        assert_eq!(slug("~"), "model");
    }
}
