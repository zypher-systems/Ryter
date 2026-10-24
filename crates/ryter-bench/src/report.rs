//! What a run produced, and the table it is read from.
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::task::Level;

/// One command of the hidden tests, as it went.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Check {
    pub command: String,
    /// `None`: it did not finish (timed out, or could not start).
    pub exit: Option<i32>,
    /// Its output, the end of it.
    pub output: String,
}

/// One task's run.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TaskResult {
    pub name: String,
    pub title: String,
    pub level: Level,
    /// `None`: the task was not scored (skipped, or could not start).
    pub solved: Option<bool>,
    /// What Ryter's spend events summed to; `None` when a call went unpriced.
    pub usd: Option<f64>,
    /// Whether every spend event was complete.
    pub complete: bool,
    pub calls: u32,
    pub input_tokens: u64,
    pub cached_tokens: u64,
    pub output_tokens: u64,
    pub seconds: f64,
    pub timed_out: bool,
    pub cancelled: bool,
    /// The build hat's exit code.
    pub exit: Option<i32>,
    /// The files the model left changed.
    pub changed: Vec<String>,
    pub acceptance: Vec<Check>,
    /// Why it was not run or not scored.
    pub error: Option<String>,
}

impl TaskResult {
    pub(crate) fn skipped(name: &str, title: &str, level: Level, why: String) -> Self {
        TaskResult {
            name: name.to_string(),
            title: title.to_string(),
            level,
            solved: None,
            usd: None,
            complete: true,
            calls: 0,
            input_tokens: 0,
            cached_tokens: 0,
            output_tokens: 0,
            seconds: 0.0,
            timed_out: false,
            cancelled: false,
            exit: None,
            changed: Vec::new(),
            acceptance: Vec::new(),
            error: Some(why),
        }
    }
}

/// A run: its settings, each task's result, and the totals.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Report {
    pub date: String,
    pub ryter: String,
    pub connection: String,
    pub model: String,
    /// The level run, or every level.
    pub level: Option<Level>,
    pub budget_usd: f64,
    pub timeout_seconds: u64,
    pub sandbox: String,
    pub total_usd: f64,
    pub results: Vec<TaskResult>,
    /// Why the run ended before its last task.
    pub stopped: Option<String>,
    /// Where the run's logs are.
    pub directory: PathBuf,
    /// Why the report could not be written, when a save failed.
    #[serde(skip)]
    pub save_error: Option<String>,
}

/// The numbers a run is read by.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    /// Tasks that were scored.
    pub tasks: usize,
    pub solved: usize,
    /// Everything the run spent, scored tasks and one cut short alike;
    /// `None` when a task that made calls went unpriced.
    pub usd: Option<f64>,
    pub usd_per_solved: Option<f64>,
    pub solved_per_usd: Option<f64>,
    pub median_minutes: Option<f64>,
    pub timed_out: usize,
    pub skipped: usize,
}

impl Report {
    pub fn summary(&self) -> Summary {
        let scored: Vec<&TaskResult> = self.results.iter().filter(|r| r.solved.is_some()).collect();
        let solved = scored.iter().filter(|r| r.solved == Some(true)).count();
        // The cost is what was paid, whether or not the task was scored: a
        // task cut short after calls was paid for too.
        let paid: Vec<&TaskResult> = self.results.iter().filter(|r| r.calls > 0).collect();
        let priced = !paid.is_empty() && paid.iter().all(|r| r.usd.is_some());
        let usd = priced.then(|| paid.iter().map(|r| r.usd.unwrap_or(0.0)).sum::<f64>());
        let mut seconds: Vec<f64> = scored.iter().map(|r| r.seconds).collect();
        seconds.sort_by(f64::total_cmp);
        let median = match seconds.len() {
            0 => None,
            n if n % 2 == 1 => Some(seconds[n / 2]),
            n => Some((seconds[n / 2 - 1] + seconds[n / 2]) / 2.0),
        };
        Summary {
            tasks: scored.len(),
            solved,
            usd,
            usd_per_solved: usd.filter(|_| solved > 0).map(|u| u / solved as f64),
            solved_per_usd: usd.filter(|u| *u > 0.0).map(|u| solved as f64 / u),
            median_minutes: median.map(|s| s / 60.0),
            timed_out: scored.iter().filter(|r| r.timed_out).count(),
            skipped: self.results.len() - scored.len(),
        }
    }

    /// The table, as a file the user can paste anywhere.
    pub fn markdown(&self) -> String {
        let s = self.summary();
        let which = match self.level {
            Some(l) => format!("the {l} tasks"),
            None => "every task".to_string(),
        };
        let mut lines = vec![
            format!("# Ryter bench — {}", self.date),
            String::new(),
            format!(
                "Ryter {} on `{}` (connection `{}`), {which}; budget ${:.2}, spent {}; {} s per task; one attempt per task; sandbox {}.",
                self.ryter,
                self.model,
                self.connection,
                self.budget_usd,
                usd(Some(self.total_usd)),
                self.timeout_seconds,
                self.sandbox
            ),
            String::new(),
            "| Solved | Cost | Cost per solved | Solved per $ | Median minutes | Timed out |"
                .into(),
            "|---|---|---|---|---|---|".into(),
            format!(
                "| {}/{} | {} | {} | {} | {} | {} |",
                s.solved,
                s.tasks,
                usd(s.usd),
                usd(s.usd_per_solved),
                one(s.solved_per_usd),
                one(s.median_minutes),
                s.timed_out
            ),
            String::new(),
            "| Task | Level | Result | Cost | Minutes |".into(),
            "|---|---|---|---|---|".into(),
        ];
        for r in &self.results {
            let result = match (r.solved, &r.error) {
                (Some(true), _) => "✓".to_string(),
                (Some(false), _) if r.timed_out => "✗ (timed out)".to_string(),
                (Some(false), _) if r.cancelled => "✗ (cancelled)".to_string(),
                (Some(false), _) => "✗".to_string(),
                (None, Some(e)) => format!("skipped: {e}"),
                (None, None) => "not run".to_string(),
            };
            lines.push(format!(
                "| {} | {} | {} | {} | {} |",
                r.name,
                r.level,
                result,
                usd(r.usd),
                if r.solved.is_some() {
                    one(Some(r.seconds / 60.0))
                } else {
                    "—".into()
                }
            ));
        }
        if let Some(why) = &self.stopped {
            lines.push(String::new());
            lines.push(format!("Stopped: {why}."));
        }
        lines.join("\n") + "\n"
    }

    /// Write `report.json` and `RESULTS.md` into `dir`; returns the table's path.
    pub fn save(&self, dir: &Path) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(dir.join("report.json"), json + "\n")?;
        let md = dir.join("RESULTS.md");
        std::fs::write(&md, self.markdown())?;
        Ok(md)
    }

    /// One line for the chat or the terminal.
    pub fn headline(&self) -> String {
        let s = self.summary();
        let mut line = format!("{}/{} solved for {}", s.solved, s.tasks, usd(s.usd));
        if let Some(per) = s.solved_per_usd {
            line.push_str(&format!(", {per:.1} solved per dollar"));
        }
        if s.skipped > 0 {
            line.push_str(&format!(", {} skipped", s.skipped));
        }
        if let Some(why) = &self.stopped {
            line.push_str(&format!("; stopped: {why}"));
        }
        if let Some(e) = &self.save_error {
            line.push_str(&format!("; the report could not be written: {e}"));
        }
        line
    }

    /// Whether the run did not go to its end: stopped, or not saved.
    pub fn ended_early(&self) -> bool {
        self.stopped.is_some() || self.save_error.is_some()
    }
}

/// `$0.123`, or a dash.
pub fn usd(v: Option<f64>) -> String {
    v.map_or_else(|| "—".to_string(), |v| format!("${v:.3}"))
}

fn one(v: Option<f64>) -> String {
    v.map_or_else(|| "—".to_string(), |v| format!("{v:.1}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(name: &str, solved: Option<bool>, usd: Option<f64>, seconds: f64) -> TaskResult {
        TaskResult {
            name: name.into(),
            title: name.into(),
            level: Level::Hard,
            solved,
            usd,
            complete: true,
            calls: if solved.is_some() { 3 } else { 0 },
            input_tokens: 10,
            cached_tokens: 5,
            output_tokens: 2,
            seconds,
            timed_out: false,
            cancelled: false,
            exit: Some(0),
            changed: vec![],
            acceptance: vec![],
            error: solved
                .is_none()
                .then(|| "needs: cargo --version".to_string()),
        }
    }

    fn cut_short(name: &str, usd: f64) -> TaskResult {
        let mut r = result(name, None, Some(usd), 30.0);
        r.calls = 3;
        r.cancelled = true;
        r.error = Some("cancelled".into());
        r
    }

    fn report() -> Report {
        Report {
            date: "2026-10-23".into(),
            ryter: "0.38.0".into(),
            connection: "anthropic".into(),
            model: "claude-sonnet-4-5".into(),
            level: Some(Level::Hard),
            budget_usd: 5.0,
            timeout_seconds: 600,
            sandbox: "workspace".into(),
            total_usd: 0.3,
            results: vec![
                result("a", Some(true), Some(0.1), 60.0),
                result("b", Some(false), Some(0.2), 180.0),
                result("c", Some(true), Some(0.0), 120.0),
                result("d", None, None, 0.0),
            ],
            stopped: None,
            directory: PathBuf::from("/tmp/x"),
            save_error: None,
        }
    }

    #[test]
    fn the_summary_is_solved_over_cost_and_skips_what_was_not_scored() {
        let s = report().summary();
        assert_eq!((s.tasks, s.solved, s.skipped, s.timed_out), (3, 2, 1, 0));
        assert!((s.usd.unwrap() - 0.3).abs() < 1e-9);
        assert!((s.usd_per_solved.unwrap() - 0.15).abs() < 1e-9);
        assert!((s.solved_per_usd.unwrap() - 2.0 / 0.3).abs() < 1e-9);
        assert_eq!(s.median_minutes, Some(2.0));
        let mut r = report();
        r.results[0].usd = None;
        let s = r.summary();
        assert_eq!(
            (s.usd, s.usd_per_solved, s.solved_per_usd),
            (None, None, None)
        );
        assert_eq!(s.solved, 2);
        // A task cut short after calls is paid for, and the cost says so.
        let mut r = report();
        r.results.push(cut_short("e", 0.2));
        let s = r.summary();
        assert!((s.usd.unwrap() - 0.5).abs() < 1e-9, "{:?}", s.usd);
        assert!((s.usd_per_solved.unwrap() - 0.25).abs() < 1e-9);
        assert_eq!((s.tasks, s.solved, s.skipped), (3, 2, 2));
        let mut only = report();
        only.results = vec![cut_short("a", 0.3)];
        assert_eq!(only.summary().usd, Some(0.3));
        assert!(
            only.headline().starts_with("0/0 solved for $0.300"),
            "{}",
            only.headline()
        );
    }

    #[test]
    fn the_table_and_the_headline() {
        let md = report().markdown();
        assert!(md.contains("# Ryter bench — 2026-10-23"));
        assert!(md.contains("the hard tasks; budget $5.00, spent $0.300"));
        assert!(
            md.contains("| 2/3 | $0.300 | $0.150 | 6.7 | 2.0 | 0 |"),
            "{md}"
        );
        assert!(md.contains("| a | hard | ✓ | $0.100 | 1.0 |"), "{md}");
        assert!(md.contains("| b | hard | ✗ | $0.200 | 3.0 |"), "{md}");
        assert!(
            md.contains("| d | hard | skipped: needs: cargo --version | — | — |"),
            "{md}"
        );
        assert_eq!(
            report().headline(),
            "2/3 solved for $0.300, 6.7 solved per dollar, 1 skipped"
        );
        let mut r = report();
        r.stopped = Some("budget reserve reached before e".into());
        assert!(
            r.markdown()
                .ends_with("Stopped: budget reserve reached before e.\n")
        );
        assert!(
            r.headline()
                .ends_with("; stopped: budget reserve reached before e")
        );
    }

    #[test]
    fn save_writes_both_files() {
        let dir = tempfile::tempdir().unwrap();
        let md = report().save(dir.path()).unwrap();
        assert_eq!(md, dir.path().join("RESULTS.md"));
        let back: Report =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("report.json")).unwrap())
                .unwrap();
        assert_eq!(back.results, report().results);
    }
}
