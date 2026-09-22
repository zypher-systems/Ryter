//! Crew spend: every specialist round is priced, attributed to a task, and
//! checked against the session budget and per-task caps.
//!
//! Before this, `run_specialist` discarded usage entirely. Builders, auditors,
//! and the architect — most of a crew's tokens — never reached the spend total,
//! the spend card, or the budget stop.

use std::collections::{BTreeMap, HashSet};
use std::sync::Mutex;

use crate::error::{Error, Result};
use crate::role::Role;
use crate::spend::{PriceBook, Usage};

/// One priced model call.
#[derive(Debug, Clone, PartialEq)]
pub struct SpendLine {
    /// Queue task id (or `patch` for patch-level work).
    pub task: String,
    /// Who spent it.
    pub role: Role,
    /// Connection name.
    pub connection: String,
    /// Model id.
    pub model: String,
    /// Tokens.
    pub usage: Usage,
    /// USD, when the model has a known price.
    pub usd: Option<f64>,
}

/// Caps a crew run enforces.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Caps {
    /// Session budget in USD (0 = none).
    pub session_usd: f64,
    /// Session spend before this crew run started.
    pub session_before: f64,
    /// USD per task (0 = none).
    pub task_usd: f64,
    /// Billable tokens per task: uncached input plus output (0 = none). Works
    /// for unpriced models, where a dollar cap cannot.
    pub task_tokens: u64,
}

/// Shared by every job in a crew run.
#[derive(Debug, Default)]
pub struct Meter {
    book: PriceBook,
    caps: Caps,
    /// Connections with no API cost (local model servers).
    free: HashSet<String>,
    /// Session spend log; each call is appended the moment it is charged, so
    /// a crash or kill mid-batch still leaves a record of what was spent.
    log: Option<std::path::PathBuf>,
    /// Where each charge is shown as it happens (TUI, `--json`). Without it
    /// the spend card sat still until a whole batch finished, while the
    /// provider's dashboard moved.
    sink: Option<std::sync::mpsc::Sender<crate::event::AgentEvent>>,
    /// `[reasoning_effort]` overrides, for every specialist this run bills.
    efforts: BTreeMap<String, String>,
    lines: Mutex<Vec<SpendLine>>,
    /// How many lines the session has already recorded.
    recorded: Mutex<usize>,
}

/// Totals for one task or a whole run.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Tally {
    /// Uncached input + output tokens.
    pub billable_tokens: u64,
    /// Cached input tokens.
    pub cached_tokens: u64,
    /// Known USD.
    pub usd: f64,
    /// Some call had no price, so `usd` is a lower bound.
    pub unpriced: bool,
}

impl Tally {
    fn add(&mut self, l: &SpendLine) {
        self.billable_tokens +=
            l.usage.input_tokens.saturating_sub(l.usage.cached_tokens) + l.usage.output_tokens;
        self.cached_tokens += l.usage.cached_tokens;
        match l.usd {
            Some(u) => self.usd += u,
            None => self.unpriced = true,
        }
    }

    /// `$0.42`, `≥$0.42` when some calls were unpriced, `$?.??` when none were.
    pub fn label(&self) -> String {
        if self.unpriced && self.usd == 0.0 {
            "$?.??".into()
        } else if self.unpriced {
            format!("≥${:.2}", self.usd)
        } else {
            format!("${:.2}", self.usd)
        }
    }
}

impl Meter {
    /// A meter pricing with `book` and enforcing `caps`.
    pub fn new(book: PriceBook, caps: Caps) -> Self {
        Self {
            book,
            caps,
            free: HashSet::new(),
            log: None,
            sink: None,
            efforts: BTreeMap::new(),
            lines: Mutex::new(Vec::new()),
            recorded: Mutex::new(0),
        }
    }

    /// Treat these connections as free: a local model has no API cost. Its
    /// tokens still count, so the token cap still stops a runaway loop.
    pub fn with_free(mut self, connections: HashSet<String>) -> Self {
        self.free = connections;
        self
    }

    /// Append every charge to this spend log as it happens.
    pub fn with_log(mut self, path: std::path::PathBuf) -> Self {
        self.log = Some(path);
        self
    }

    /// Use the user's `[reasoning_effort]` overrides for this run.
    pub fn with_efforts(mut self, efforts: BTreeMap<String, String>) -> Self {
        self.efforts = efforts;
        self
    }

    /// How hard `role` should reason on this run.
    pub fn effort(&self, role: Role) -> Option<String> {
        crate::config::effort_for(Some(&self.efforts), role)
    }

    /// Report every charge to `sink` the moment it is made.
    pub fn with_sink(mut self, sink: std::sync::mpsc::Sender<crate::event::AgentEvent>) -> Self {
        self.sink = Some(sink);
        self
    }

    /// Whether charges are already reported live, so a batch summary must
    /// not report them again.
    pub fn is_live(&self) -> bool {
        self.sink.is_some()
    }

    /// Record one call and enforce the caps. `Error::Budget` stops the whole
    /// crew; `Error::TaskBudget` stops only this task.
    pub fn charge(
        &self,
        task: &str,
        role: Role,
        connection: &str,
        model: &str,
        usage: Usage,
        reported_usd: Option<f64>,
    ) -> Result<()> {
        let line = SpendLine {
            task: task.to_string(),
            role,
            connection: connection.to_string(),
            model: model.to_string(),
            usage,
            usd: reported_usd.or_else(|| {
                if self.free.contains(connection) {
                    Some(0.0)
                } else {
                    self.book.cost(model, usage)
                }
            }),
        };
        if let Some(path) = &self.log {
            crate::session::append_jsonl(
                path,
                &crate::session::spend_record(
                    line.connection.clone(),
                    line.model.clone(),
                    line.role,
                    line.usage,
                    line.usd,
                ),
            )?;
        }
        if let Some(sink) = &self.sink {
            // Before the caps: the call that trips one was still paid for.
            let _ = sink.send(crate::event::AgentEvent::Spend {
                connection: line.connection.clone(),
                model: line.model.clone(),
                role: line.role,
                subagent_id: None,
                input_tokens: line.usage.input_tokens,
                output_tokens: line.usage.output_tokens,
                cached_tokens: line.usage.cached_tokens,
                total_usd: line.usd,
            });
        }
        let mut lines = self
            .lines
            .lock()
            .map_err(|e| Error::Config(e.to_string()))?;
        lines.push(line);
        let mut run = Tally::default();
        let mut this = Tally::default();
        for l in lines.iter() {
            run.add(l);
            if l.task == task {
                this.add(l);
            }
        }
        drop(lines);
        let spent = self.caps.session_before + run.usd;
        if self.caps.session_usd > 0.0 && spent >= self.caps.session_usd {
            return Err(Error::Budget {
                spent,
                cap: self.caps.session_usd,
            });
        }
        if self.caps.task_usd > 0.0 && this.usd >= self.caps.task_usd {
            return Err(Error::TaskBudget(format!(
                "task {task} reached its ${:.2} cap ({})",
                self.caps.task_usd,
                this.label()
            )));
        }
        if self.caps.task_tokens > 0 && this.billable_tokens >= self.caps.task_tokens {
            return Err(Error::TaskBudget(format!(
                "task {task} reached its {} token cap",
                self.caps.task_tokens
            )));
        }
        Ok(())
    }

    /// Totals for one task.
    pub fn task(&self, task: &str) -> Tally {
        let mut t = Tally::default();
        if let Ok(lines) = self.lines.lock() {
            for l in lines.iter().filter(|l| l.task == task) {
                t.add(l);
            }
        }
        t
    }

    /// Totals per role across the run.
    pub fn by_role(&self) -> BTreeMap<String, Tally> {
        let mut out: BTreeMap<String, Tally> = BTreeMap::new();
        if let Ok(lines) = self.lines.lock() {
            for l in lines.iter() {
                out.entry(l.role.to_string()).or_default().add(l);
            }
        }
        out
    }

    /// Totals for the whole run.
    pub fn total(&self) -> Tally {
        let mut t = Tally::default();
        if let Ok(lines) = self.lines.lock() {
            for l in lines.iter() {
                t.add(l);
            }
        }
        t
    }

    /// Lines not yet written to the session's spend log. Totals keep counting
    /// them, so the budget check still sees the whole run.
    pub fn unrecorded(&self) -> Vec<SpendLine> {
        let (Ok(lines), Ok(mut done)) = (self.lines.lock(), self.recorded.lock()) else {
            return Vec::new();
        };
        let out = lines[(*done).min(lines.len())..].to_vec();
        *done = lines.len();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u64, output: u64) -> Usage {
        Usage {
            input_tokens: input,
            output_tokens: output,
            cached_tokens: 0,
        }
    }

    #[test]
    fn crew_spend_counts_against_the_session_budget() {
        let m = Meter::new(
            PriceBook::new(),
            Caps {
                session_usd: 1.0,
                session_before: 0.95,
                ..Caps::default()
            },
        );
        let err = m
            .charge("t1", Role::Builder, "c", "m", usage(1, 1), Some(0.10))
            .unwrap_err();
        assert!(matches!(err, Error::Budget { .. }), "{err}");
    }

    #[test]
    fn a_task_cap_stops_only_that_task() {
        let m = Meter::new(
            PriceBook::new(),
            Caps {
                task_usd: 0.50,
                ..Caps::default()
            },
        );
        m.charge("t1", Role::Builder, "c", "m", usage(1, 1), Some(0.30))
            .unwrap();
        m.charge("t2", Role::Builder, "c", "m", usage(1, 1), Some(0.30))
            .unwrap();
        let err = m
            .charge("t1", Role::Auditor, "c", "m", usage(1, 1), Some(0.25))
            .unwrap_err();
        assert!(matches!(err, Error::TaskBudget(_)), "{err}");
        assert!((m.task("t1").usd - 0.55).abs() < 1e-9);
    }

    /// Unknown prices cannot trip a dollar cap; the token cap still works.
    #[test]
    fn the_token_cap_covers_unpriced_models() {
        let m = Meter::new(
            PriceBook::new(),
            Caps {
                task_usd: 0.01,
                task_tokens: 1_000,
                ..Caps::default()
            },
        );
        m.charge("t1", Role::Builder, "c", "mystery", usage(600, 100), None)
            .unwrap();
        let err = m
            .charge("t1", Role::Builder, "c", "mystery", usage(300, 100), None)
            .unwrap_err();
        assert!(matches!(err, Error::TaskBudget(_)), "{err}");
        assert_eq!(m.task("t1").label(), "$?.??");
    }

    /// A local model costs nothing in fees, and that is a real $0.00, not an
    /// unknown one. Its tokens still count toward the cap.
    #[test]
    fn local_connections_are_free_but_still_capped() {
        let m = Meter::new(
            PriceBook::new(),
            Caps {
                task_tokens: 1_000,
                ..Caps::default()
            },
        )
        .with_free(["ollama".to_string()].into());
        m.charge(
            "t1",
            Role::Builder,
            "ollama",
            "qwen3-coder",
            usage(500, 100),
            None,
        )
        .unwrap();
        assert_eq!(m.task("t1").label(), "$0.00");
        let err = m
            .charge(
                "t1",
                Role::Builder,
                "ollama",
                "qwen3-coder",
                usage(500, 100),
                None,
            )
            .unwrap_err();
        assert!(matches!(err, Error::TaskBudget(_)));
    }

    /// A run killed mid-batch must still leave a record of what it spent.
    #[test]
    fn every_charge_reaches_the_log_immediately() {
        let d = tempfile::TempDir::new().unwrap();
        let path = d.path().join("spend.jsonl");
        let m = Meter::new(PriceBook::new(), Caps::default()).with_log(path.clone());
        m.charge("t1", Role::Builder, "c", "m", usage(10, 1), Some(0.02))
            .unwrap();
        m.charge("t1", Role::Auditor, "c", "m", usage(10, 1), Some(0.01))
            .unwrap();
        // No batch finished and nothing was recorded by the agent, yet:
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains("\"builder\"") && text.contains("\"auditor\""));
    }

    #[test]
    fn cached_tokens_are_not_billable() {
        let m = Meter::new(PriceBook::new(), Caps::default());
        m.charge(
            "t1",
            Role::Builder,
            "c",
            "m",
            Usage {
                input_tokens: 10_000,
                output_tokens: 100,
                cached_tokens: 9_000,
            },
            Some(0.01),
        )
        .unwrap();
        let t = m.task("t1");
        assert_eq!(t.billable_tokens, 1_100);
        assert_eq!(t.cached_tokens, 9_000);
    }
}
