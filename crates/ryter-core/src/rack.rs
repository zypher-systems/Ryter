//! The hat rack: what each hat has done in a session, folded from the
//! session's events. The screen feeds it live; a resumed session reads it
//! back from the event log, so both show the same figures.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader};
use std::path::Path;

use crate::event::AgentEvent;
use crate::role::Role;

/// One hat's totals for the session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HatTotals {
    /// Turns that started in this hat.
    pub turns: u32,
    /// Plans the user approved.
    pub plans_approved: u32,
    /// Plans the user rejected.
    pub plans_rejected: u32,
    /// Files this hat changed, each counted once.
    pub files: BTreeSet<String>,
    /// Lines it added, gross.
    pub added: u64,
    /// Lines it removed, gross.
    pub removed: u64,
    /// Reviews that ended `VERDICT: PASS`.
    pub verdicts_passed: u32,
    /// Reviews that ended `VERDICT: FAIL`.
    pub verdicts_failed: u32,
    /// Test scenarios that passed, across reports.
    pub checks_passed: u32,
    /// Test scenarios not reached, because an earlier one failed.
    pub checks_skipped: u32,
    /// Test scenarios that failed, across reports.
    pub checks_failed: u32,
}

/// The four hats' totals.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rack {
    plan: HatTotals,
    build: HatTotals,
    review: HatTotals,
    test: HatTotals,
    /// The hat the turn in progress is in.
    hat: Option<Role>,
    /// A hat put on in the middle of a turn that has not done anything
    /// yet. Its first step counts as a turn of its own: a plan approved in
    /// the plan hat is built in the build hat, in the same turn.
    switched: Option<Role>,
}

/// `✗ 2 of 5 failed` or `✓ 5 of 5 passed` as `(passed, failed)`.
fn checks(headline: &str, passed: bool) -> (u32, u32) {
    let mut nums = headline
        .split_whitespace()
        .filter_map(|w| w.parse::<u32>().ok());
    match (nums.next(), nums.next()) {
        (Some(n), Some(total)) if passed => (total.max(n), 0),
        (Some(n), Some(total)) => (total.saturating_sub(n), n),
        // A headline in another shape: the report still passed or failed.
        _ if passed => (1, 0),
        _ => (0, 1),
    }
}

impl Rack {
    /// A hat's totals. A role from crew mode has none of its own: build's.
    pub fn of(&self, role: Role) -> &HatTotals {
        match role {
            Role::SoloPlan => &self.plan,
            Role::SoloReview => &self.review,
            Role::SoloTest => &self.test,
            Role::SoloBuild | Role::Crew => &self.build,
        }
    }

    fn of_mut(&mut self, role: Role) -> &mut HatTotals {
        match role {
            Role::SoloPlan => &mut self.plan,
            Role::SoloReview => &mut self.review,
            Role::SoloTest => &mut self.test,
            Role::SoloBuild | Role::Crew => &mut self.build,
        }
    }

    /// Whether any turn has run in `role`.
    pub fn worn(&self, role: Role) -> bool {
        self.of(role).turns > 0
    }

    /// A hat put on mid-turn has started to work: that is a turn of its.
    fn step(&mut self) {
        if let Some(role) = self.switched.take() {
            self.of_mut(role).turns += 1;
        }
    }

    /// Count what `ev` says happened.
    pub fn apply(&mut self, ev: &AgentEvent) {
        match ev {
            AgentEvent::TurnStarted { role, .. } => {
                self.hat = Some(*role);
                self.switched = None;
                self.of_mut(*role).turns += 1;
            }
            AgentEvent::ModeChanged { role } => {
                if self.hat != Some(*role) {
                    self.switched = Some(*role);
                }
                self.hat = Some(*role);
            }
            AgentEvent::ToolCall { .. } => self.step(),
            AgentEvent::ToolResult { is_error, diff, .. } => {
                self.step();
                // Only the build hat changes the project's files; a plan's
                // notes are not the work.
                if let (false, Some(d)) = (*is_error, diff) {
                    if self.hat.is_none_or(Role::writes_source) {
                        let t = &mut self.build;
                        t.files.insert(d.path.clone());
                        t.added += d.added as u64;
                        t.removed += d.removed as u64;
                    }
                }
            }
            AgentEvent::Planned { approved } => {
                if *approved {
                    self.plan.plans_approved += 1;
                } else {
                    self.plan.plans_rejected += 1;
                }
            }
            AgentEvent::Reviewed { verdict, .. } => match verdict {
                Some(true) => self.review.verdicts_passed += 1,
                Some(false) => self.review.verdicts_failed += 1,
                None => {}
            },
            AgentEvent::Tested {
                headline,
                passed,
                rows,
                ..
            } => {
                // A scenario not reached, because an earlier one failed, is
                // counted among the failures in the headline; here it is a
                // warning, not a failure of its own.
                let (ok, bad) = checks(headline, *passed);
                let unreached = rows
                    .iter()
                    .filter(|r| r.starts_with('✗') && r.contains("· not reached"))
                    .count();
                let unreached = u32::try_from(unreached).unwrap_or(u32::MAX).min(bad);
                self.test.checks_passed += ok;
                self.test.checks_skipped += unreached;
                self.test.checks_failed += bad - unreached;
            }
            _ => {}
        }
    }

    /// The totals a session's event log adds up to. A log that can't be
    /// read, or a line that isn't an event, counts for nothing.
    pub fn from_log(path: &Path) -> Self {
        let mut rack = Self::default();
        let Ok(file) = std::fs::File::open(path) else {
            return rack;
        };
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            // Most of a log is streamed text, which counts for nothing here.
            if line.starts_with("{\"kind\":\"token\"")
                || line.starts_with("{\"kind\":\"reasoning\"")
            {
                continue;
            }
            if let Ok(ev) = serde_json::from_str::<AgentEvent>(&line) {
                rack.apply(&ev);
            }
        }
        rack.hat = None;
        rack.switched = None;
        rack
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::FileDiff;

    fn started(role: Role) -> AgentEvent {
        AgentEvent::TurnStarted { turn: 1, role }
    }

    fn edit(path: &str, added: usize, removed: usize) -> AgentEvent {
        AgentEvent::ToolResult {
            id: "t".into(),
            output: String::new(),
            is_error: false,
            duration_ms: None,
            diff: Some(Box::new(FileDiff {
                path: path.into(),
                created: false,
                added,
                removed,
                hunks: Vec::new(),
                elided: 0,
            })),
        }
    }

    fn tested(headline: &str, passed: bool) -> AgentEvent {
        tested_rows(headline, passed, Vec::new())
    }

    fn tested_rows(headline: &str, passed: bool, rows: Vec<String>) -> AgentEvent {
        AgentEvent::Tested {
            model: "m".into(),
            headline: headline.into(),
            passed,
            rows,
            file: String::new(),
            first_failed: None,
            tree: None,
            total_usd: None,
            duration_ms: 0,
        }
    }

    fn reviewed(verdict: Option<bool>) -> AgentEvent {
        AgentEvent::Reviewed {
            model: "m".into(),
            connection: "c".into(),
            verdict,
            tree: None,
            total_usd: None,
        }
    }

    #[test]
    fn each_hat_counts_its_own() {
        let mut r = Rack::default();
        for ev in [
            started(Role::SoloPlan),
            AgentEvent::Planned { approved: true },
            started(Role::SoloBuild),
            edit("a.rs", 8, 1),
            started(Role::SoloReview),
            reviewed(Some(false)),
            started(Role::SoloBuild),
            edit("a.rs", 5, 1),
            edit("b.rs", 2, 0),
            started(Role::SoloReview),
            reviewed(Some(true)),
            reviewed(None),
            started(Role::SoloTest),
            tested("✗ 2 of 5 failed", false),
            tested("✓ 3 of 3 passed", true),
            tested_rows(
                "✗ 2 of 3 failed",
                false,
                vec![
                    "✓ 1  starts".into(),
                    "✗ 2  login".into(),
                    "✗ 3  publish · not reached (needs 2)".into(),
                ],
            ),
        ] {
            r.apply(&ev);
        }
        assert_eq!(r.of(Role::SoloPlan).turns, 1);
        assert_eq!(r.of(Role::SoloPlan).plans_approved, 1);
        let b = r.of(Role::SoloBuild);
        assert_eq!((b.turns, b.files.len(), b.added, b.removed), (2, 2, 15, 2));
        let v = r.of(Role::SoloReview);
        assert_eq!((v.turns, v.verdicts_passed, v.verdicts_failed), (2, 1, 1));
        let t = r.of(Role::SoloTest);
        assert_eq!(
            (t.turns, t.checks_passed, t.checks_skipped, t.checks_failed),
            (1, 7, 1, 3)
        );
        assert!(r.worn(Role::SoloTest));
    }

    #[test]
    fn an_edit_outside_the_build_hat_is_not_the_builds() {
        let mut r = Rack::default();
        r.apply(&started(Role::SoloPlan));
        r.apply(&edit("notes/plan.md", 4, 0));
        assert!(r.of(Role::SoloBuild).files.is_empty());
        // A failed edit changed nothing.
        r.apply(&started(Role::SoloBuild));
        let AgentEvent::ToolResult { diff, .. } = edit("a.rs", 3, 0) else {
            unreachable!()
        };
        r.apply(&AgentEvent::ToolResult {
            id: "t".into(),
            output: "error".into(),
            is_error: true,
            duration_ms: None,
            diff,
        });
        assert_eq!(r.of(Role::SoloBuild).added, 0);
    }

    #[test]
    fn a_plan_built_in_the_turn_it_was_approved_in_is_a_build_turn() {
        let mut r = Rack::default();
        r.apply(&started(Role::SoloPlan));
        r.apply(&AgentEvent::Planned { approved: true });
        r.apply(&AgentEvent::ModeChanged {
            role: Role::SoloBuild,
        });
        assert_eq!(r.of(Role::SoloBuild).turns, 0, "nothing built yet");
        r.apply(&edit("a.rs", 1, 0));
        r.apply(&edit("b.rs", 1, 0));
        assert_eq!(r.of(Role::SoloBuild).turns, 1);
        // A hat change that a turn of its own follows is counted once.
        r.apply(&AgentEvent::ModeChanged {
            role: Role::SoloReview,
        });
        r.apply(&started(Role::SoloReview));
        r.apply(&edit("c.rs", 1, 0));
        assert_eq!(r.of(Role::SoloReview).turns, 1);
    }

    #[test]
    fn a_log_adds_up_to_what_the_events_did() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let events = [
            started(Role::SoloBuild),
            AgentEvent::Token { text: "hi".into() },
            edit("a.rs", 2, 1),
            started(Role::SoloReview),
            reviewed(Some(true)),
        ];
        let mut live = Rack::default();
        let mut text = String::new();
        for ev in &events {
            live.apply(ev);
            text.push_str(&serde_json::to_string(ev).unwrap());
            text.push('\n');
        }
        text.push_str("not an event\n");
        std::fs::write(&path, text).unwrap();
        let read = Rack::from_log(&path);
        assert_eq!(read.of(Role::SoloBuild), live.of(Role::SoloBuild));
        assert_eq!(read.of(Role::SoloReview), live.of(Role::SoloReview));
        assert_eq!(Rack::from_log(&dir.path().join("none")), Rack::default());
    }

    #[test]
    fn a_headline_in_another_shape_still_counts_once() {
        assert_eq!(checks("✓ 5 of 5 passed", true), (5, 0));
        assert_eq!(checks("✗ 2 of 5 failed", false), (3, 2));
        assert_eq!(checks("done", true), (1, 0));
        assert_eq!(checks("broken", false), (0, 1));
    }
}
