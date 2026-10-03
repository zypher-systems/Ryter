//! The hats: what the model may do in a turn.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::error::Error;

/// The hat a turn is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Plan hat: reading and proposing in the user's tree. Writes notes and
    /// memory only.
    #[serde(rename = "plan")]
    SoloPlan,
    /// Build hat: changing the user's tree directly, behind the permission
    /// gate (edits and non-read-only commands ask).
    #[serde(rename = "build")]
    SoloBuild,
    /// Review hat: critiquing what changed. Runs tests and linters; edits
    /// nothing.
    #[serde(rename = "review")]
    SoloReview,
    /// A role from crew mode, which was removed, or the test hat, which
    /// was removed after it. Nothing runs as it: it is what a session saved
    /// before then names in its spend and its mode, so those still load.
    #[serde(
        alias = "orchestrator",
        alias = "architect",
        alias = "planner",
        alias = "builder",
        alias = "auditor",
        alias = "test"
    )]
    Crew,
}

impl Role {
    /// Whether this hat may change the project's files.
    pub fn writes_source(self) -> bool {
        self == Self::SoloBuild
    }

    /// A hat a turn can be in: everything but [`Role::Crew`].
    pub fn is_solo(self) -> bool {
        self != Self::Crew
    }

    /// The hat a saved session opens in: its own, or build for a session
    /// from crew mode.
    pub fn hat(self) -> Self {
        if self.is_solo() {
            self
        } else {
            Self::SoloBuild
        }
    }

    /// The hat `Tab` moves to, in the order the work goes: plan → build →
    /// review, and round to plan.
    pub fn next_hat(self) -> Self {
        match self {
            Self::SoloPlan => Self::SoloBuild,
            Self::SoloBuild => Self::SoloReview,
            Self::SoloReview => Self::SoloPlan,
            Self::Crew => Self::SoloBuild,
        }
    }

    /// The hat `Shift+Tab` moves to: the same round, backwards.
    pub fn prev_hat(self) -> Self {
        match self {
            Self::SoloPlan => Self::SoloReview,
            Self::SoloReview => Self::SoloBuild,
            Self::SoloBuild => Self::SoloPlan,
            Self::Crew => Self::SoloBuild,
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
                "[hat: plan — nothing may change; read and think. When you have a plan, show it \
                 with present_plan: goal, steps, files, risks, and how to verify]",
            ),
            Self::SoloReview => Some(
                "[hat: review — nothing may change; read and run tests. When asked for a review, \
                 end with findings, blocking ones first, then your verdict]",
            ),
            Self::Crew => None,
        }
    }

    /// Stable lowercase name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Crew => "crew",
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

/// What `[ui] start_hat` may say.
pub const START_HATS: &[&str] = &["plan", "build", "review", "last"];

/// The hat a new session opens in, from `[ui] start_hat` and the hat this
/// project's latest session ended in. `last` with no earlier session is
/// plan; so is a value that isn't one of [`START_HATS`].
pub fn start_hat(setting: &str, last: Option<Role>) -> Role {
    match setting.trim().to_ascii_lowercase().as_str() {
        "build" => Role::SoloBuild,
        "review" => Role::SoloReview,
        "last" => last.map_or(Role::SoloPlan, Role::hat),
        _ => Role::SoloPlan,
    }
}

impl FromStr for Role {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "plan" => Ok(Self::SoloPlan),
            "build" => Ok(Self::SoloBuild),
            "review" => Ok(Self::SoloReview),
            other => Err(Error::Config(format!(
                "unknown hat {other:?}: build, plan, or review"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_session_opens_in_the_hat_the_setting_names() {
        for (setting, last, want) in [
            ("plan", None, Role::SoloPlan),
            ("build", Some(Role::SoloReview), Role::SoloBuild),
            ("review", None, Role::SoloReview),
            (" Build ", None, Role::SoloBuild),
            ("last", None, Role::SoloPlan),
            ("last", Some(Role::SoloBuild), Role::SoloBuild),
            ("last", Some(Role::SoloReview), Role::SoloReview),
            // A session from crew mode, or one left in the test hat that
            // was, opens in build.
            ("last", Some(Role::Crew), Role::SoloBuild),
            // Test is no hat, and neither is a typo.
            ("test", Some(Role::SoloBuild), Role::SoloPlan),
            ("bulid", None, Role::SoloPlan),
            ("", None, Role::SoloPlan),
        ] {
            assert_eq!(start_hat(setting, last), want, "{setting:?} {last:?}");
        }
    }

    #[test]
    fn only_the_build_hat_writes_source() {
        assert!(Role::SoloBuild.writes_source());
        assert!(!Role::SoloPlan.writes_source() && !Role::SoloReview.writes_source());
        assert!(!Role::Crew.writes_source());
    }

    /// A session saved in crew mode, or in the test hat, names roles that
    /// are gone. Its spend and its mode still load, and it opens in the
    /// build hat.
    #[test]
    fn roles_from_crew_mode_still_load() {
        for old in [
            "orchestrator",
            "architect",
            "planner",
            "builder",
            "auditor",
            "crew",
            "test",
        ] {
            let role: Role = serde_json::from_str(&format!("\"{old}\"")).unwrap();
            assert_eq!(role, Role::Crew, "{old}");
            assert_eq!(role.hat(), Role::SoloBuild);
            assert!(!role.is_solo());
        }
        assert_eq!(Role::SoloReview.hat(), Role::SoloReview);
        // Nobody can ask for one.
        assert!("builder".parse::<Role>().is_err());
        assert!("crew".parse::<Role>().is_err());
        assert!("test".parse::<Role>().is_err());
    }

    #[test]
    fn tab_goes_round_in_the_order_the_work_does() {
        let mut h = Role::SoloPlan;
        let mut seen = Vec::new();
        for _ in 0..3 {
            seen.push(h.as_str());
            h = h.next_hat();
        }
        assert_eq!(seen, ["plan", "build", "review"]);
        assert_eq!(h, Role::SoloPlan);
        // And back the other way.
        for back in ["review", "build", "plan"] {
            h = h.prev_hat();
            assert_eq!(h.as_str(), back);
        }
        // From build, where a session opens: on to review, back to plan.
        assert_eq!(Role::SoloBuild.next_hat(), Role::SoloReview);
        assert_eq!(Role::SoloBuild.prev_hat(), Role::SoloPlan);
        // A role from crew mode has no place in the round.
        assert_eq!(Role::Crew.next_hat(), Role::SoloBuild);
        // Round-trips as the hat name, in logs and sessions.
        assert_eq!(
            serde_json::to_string(&Role::SoloReview).unwrap(),
            "\"review\""
        );
        assert_eq!("plan".parse::<Role>().unwrap(), Role::SoloPlan);
    }
}
