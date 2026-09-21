//! Agent roles: orchestrator vs specialists.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::error::Error;

/// Who is acting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// User-facing conversation. Reads, asks, delegates. Never writes `src/`.
    Orchestrator,
    /// Plans and designs: scope, shape, risks, and the task list builders
    /// run. Writes project memory, never product source. Absorbed the old
    /// planner role; `planner` still parses, and old logs still load.
    #[serde(alias = "planner")]
    Architect,
    /// Writes product code in its own git worktree.
    Builder,
    /// Reviews a builder diff; cannot write product code.
    Auditor,
    /// Normal mode, plan hat: one model reading and proposing in the user's
    /// tree. Writes notes and memory only.
    #[serde(rename = "plan")]
    SoloPlan,
    /// Normal mode, build hat: one model changing the user's tree directly,
    /// behind the permission gate (edits and non-read-only commands ask).
    #[serde(rename = "build")]
    SoloBuild,
    /// Normal mode, review hat: one model critiquing what changed. Runs tests
    /// and linters; edits nothing.
    #[serde(rename = "review")]
    SoloReview,
}

impl Role {
    /// Whether this role may mutate product source (not pass notes).
    pub fn writes_source(self) -> bool {
        matches!(self, Self::Builder | Self::SoloBuild)
    }

    /// One of normal mode's hats.
    pub fn is_solo(self) -> bool {
        matches!(self, Self::SoloPlan | Self::SoloBuild | Self::SoloReview)
    }

    /// The hat `Tab` moves to: build → plan → review → build.
    pub fn next_hat(self) -> Self {
        match self {
            Self::SoloBuild => Self::SoloPlan,
            Self::SoloPlan => Self::SoloReview,
            _ => Self::SoloBuild,
        }
    }

    /// The hat `Shift+Tab` moves to.
    pub fn prev_hat(self) -> Self {
        match self {
            Self::SoloBuild => Self::SoloReview,
            Self::SoloReview => Self::SoloPlan,
            _ => Self::SoloBuild,
        }
    }

    /// The line put in front of each message in a hat, so the model knows
    /// what it may do this turn without the system prompt (and the prompt
    /// cache) changing on every switch.
    pub fn hat_note(self) -> Option<&'static str> {
        match self {
            // What the hat allows, not orders: a directive here made the model
            // start working on "are you there?".
            Self::SoloBuild => Some(
                "[hat: build — you may change files and run commands when the request calls for it]",
            ),
            Self::SoloPlan => Some(
                "[hat: plan — nothing may change; read and think. When asked for a plan, end with \
                 files, steps, risks, and how to verify]",
            ),
            Self::SoloReview => Some(
                "[hat: review — nothing may change; read and run tests. When asked for a review, \
                 end with findings, blocking ones first]",
            ),
            _ => None,
        }
    }

    /// Stable lowercase name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Orchestrator => "orchestrator",
            Self::Architect => "architect",
            Self::Builder => "builder",
            Self::Auditor => "auditor",
            Self::SoloPlan => "plan",
            Self::SoloBuild => "build",
            Self::SoloReview => "review",
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Role {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "orchestrator" | "lead" | "crew" => Ok(Self::Orchestrator),
            "architect" | "planner" => Ok(Self::Architect),
            "builder" => Ok(Self::Builder),
            "auditor" | "audit" => Ok(Self::Auditor),
            "plan" => Ok(Self::SoloPlan),
            "build" => Ok(Self::SoloBuild),
            "review" => Ok(Self::SoloReview),
            other => Err(Error::Config(format!("unknown role {other:?}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_builder_writes_source() {
        assert!(!Role::Orchestrator.writes_source());
        assert!(!Role::Architect.writes_source());
        assert!(!Role::Architect.writes_source());
        assert!(Role::Builder.writes_source());
        assert!(!Role::Auditor.writes_source());
        assert!(Role::SoloBuild.writes_source());
        assert!(!Role::SoloPlan.writes_source() && !Role::SoloReview.writes_source());
    }

    #[test]
    fn tab_cycles_build_plan_review() {
        let mut h = Role::SoloBuild;
        let mut seen = Vec::new();
        for _ in 0..3 {
            seen.push(h.as_str());
            h = h.next_hat();
        }
        assert_eq!(seen, ["build", "plan", "review"]);
        assert_eq!(h, Role::SoloBuild);
        assert_eq!(Role::SoloBuild.prev_hat(), Role::SoloReview);
        // Round-trips as the hat name, in logs and sessions.
        assert_eq!(
            serde_json::to_string(&Role::SoloReview).unwrap(),
            "\"review\""
        );
        assert_eq!("plan".parse::<Role>().unwrap(), Role::SoloPlan);
    }
}
