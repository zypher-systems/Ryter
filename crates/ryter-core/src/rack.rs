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
    /// What the latest audit left in the tree: `Some(0)` changed nothing,
    /// `Some(n)` had n files put back; `None` before any audit.
    pub last_restored: Option<u32>,
}

/// The four hats' totals.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rack {
    plan: HatTotals,
    build: HatTotals,
    review: HatTotals,
    scribe: HatTotals,
    /// The hat the turn in progress is in.
    hat: Option<Role>,
    /// A hat put on in the middle of a turn that has not done anything
    /// yet. Its first step counts as a turn of its own: a plan approved in
    /// the plan hat is built in the build hat, in the same turn.
    switched: Option<Role>,
    /// This turn's audit was counted from its `Audited` event; the
    /// `Reviewed` that `/audit` adds after it is the same verdict.
    audited: bool,
}

impl Rack {
    /// A hat's totals. A role from crew mode has none of its own: build's.
    pub fn of(&self, role: Role) -> &HatTotals {
        match role {
            Role::SoloPlan => &self.plan,
            Role::SoloAudit => &self.review,
            Role::SoloScribe => &self.scribe,
            Role::SoloBuild | Role::Crew => &self.build,
        }
    }

    fn of_mut(&mut self, role: Role) -> &mut HatTotals {
        match role {
            Role::SoloPlan => &mut self.plan,
            Role::SoloAudit => &mut self.review,
            Role::SoloScribe => &mut self.scribe,
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
                self.audited = false;
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
                // The build hat changes the project's files and the scribe
                // its documentation; a plan's notes are not the work.
                if let (false, Some(d)) = (*is_error, diff) {
                    let t = if self.hat.is_none_or(Role::writes_source) {
                        Some(&mut self.build)
                    } else if self.hat == Some(Role::SoloScribe) {
                        Some(&mut self.scribe)
                    } else {
                        None
                    };
                    if let Some(t) = t {
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
            // An old log's reviews; a new audit is counted from `Audited`.
            AgentEvent::Reviewed { verdict, .. } if !self.audited => match verdict {
                Some(true) => self.review.verdicts_passed += 1,
                Some(false) => self.review.verdicts_failed += 1,
                None => {}
            },
            AgentEvent::Audited {
                verdict, restored, ..
            } => {
                self.audited = true;
                match verdict {
                    Some(true) => self.review.verdicts_passed += 1,
                    Some(false) => self.review.verdicts_failed += 1,
                    None => {}
                }
                self.review.last_restored = Some(restored.len() as u32);
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
        AgentEvent::TurnStarted {
            turn: 1,
            role,
            at: 0,
        }
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
            started(Role::SoloAudit),
            reviewed(Some(false)),
            started(Role::SoloBuild),
            edit("a.rs", 5, 1),
            edit("b.rs", 2, 0),
            started(Role::SoloAudit),
            reviewed(Some(true)),
            reviewed(None),
        ] {
            r.apply(&ev);
        }
        assert_eq!(r.of(Role::SoloPlan).turns, 1);
        assert_eq!(r.of(Role::SoloPlan).plans_approved, 1);
        let b = r.of(Role::SoloBuild);
        assert_eq!((b.turns, b.files.len(), b.added, b.removed), (2, 2, 15, 2));
        let v = r.of(Role::SoloAudit);
        assert_eq!((v.turns, v.verdicts_passed, v.verdicts_failed), (2, 1, 1));
        assert!(r.worn(Role::SoloAudit));
        // A session from before: a turn in the test hat counts as build's.
        r.apply(&AgentEvent::TurnStarted {
            turn: 9,
            role: Role::Crew,
            at: 0,
        });
        assert_eq!(r.of(Role::SoloBuild).turns, 3);
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
            role: Role::SoloAudit,
        });
        r.apply(&started(Role::SoloAudit));
        r.apply(&edit("c.rs", 1, 0));
        assert_eq!(r.of(Role::SoloAudit).turns, 1);
    }

    #[test]
    fn a_log_adds_up_to_what_the_events_did() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let events = [
            started(Role::SoloBuild),
            AgentEvent::Token { text: "hi".into() },
            edit("a.rs", 2, 1),
            started(Role::SoloAudit),
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
        assert_eq!(read.of(Role::SoloAudit), live.of(Role::SoloAudit));
        assert_eq!(Rack::from_log(&dir.path().join("none")), Rack::default());
        // A log from before the rename names the audit hat `review`.
        let old = dir.path().join("old.jsonl");
        std::fs::write(
            &old,
            "{\"kind\":\"turn_started\",\"turn\":1,\"role\":\"review\"}\n",
        )
        .unwrap();
        assert_eq!(Rack::from_log(&old).of(Role::SoloAudit).turns, 1);
    }
}
