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
    /// Audit hat: critiquing what changed. Runs tests and linters; edits
    /// nothing. It was the review hat until 0.16, and loads under that name.
    #[serde(rename = "audit", alias = "review")]
    SoloAudit,
    /// Scribe hat: documentation only. Reads everything, writes `.md`,
    /// `.txt` and their kind, runs read-only commands; changes no code.
    #[serde(rename = "scribe")]
    SoloScribe,
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

/// The two rows of the hat rack (`docs/specialists-design.md` §2): the
/// primary hats the work is done in, and the specialists reached for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Row {
    /// Plan and build.
    Primary,
    /// Audit, and the specialists after it.
    Specialist,
}

impl Row {
    /// The hat the other row opens on when none of its hats has been worn:
    /// build for the primary row, audit for the specialists.
    pub fn default_hat(self) -> Role {
        match self {
            Self::Primary => Role::SoloBuild,
            Self::Specialist => Role::SoloAudit,
        }
    }

    /// The other row.
    pub fn other(self) -> Self {
        match self {
            Self::Primary => Self::Specialist,
            Self::Specialist => Self::Primary,
        }
    }
}

impl Role {
    /// The row of the rack this hat is in.
    pub fn row(self) -> Row {
        match self {
            Self::SoloPlan | Self::SoloBuild | Self::Crew => Row::Primary,
            Self::SoloAudit | Self::SoloScribe => Row::Specialist,
        }
    }

    /// The hat `Tab` moves to: the next in this hat's row. Plan and build
    /// trade places; the specialists go round their own row.
    pub fn next_in_row(self) -> Self {
        match self {
            Self::SoloPlan => Self::SoloBuild,
            Self::SoloBuild | Self::Crew => Self::SoloPlan,
            Self::SoloAudit => Self::SoloScribe,
            Self::SoloScribe => Self::SoloAudit,
        }
    }

    /// The hat the other row opens on when nothing there was worn yet.
    pub fn other_row_default(self) -> Self {
        self.row().other().default_hat()
    }

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
                "[hat: plan — nothing may change; read and think. check_package tells you a \
                 dependency's latest release and its known advisories; ask it before you pin a \
                 version. When you have a plan, show it with present_plan: goal, steps, files, \
                 risks, and how to verify]",
            ),
            Self::SoloAudit => Some(
                "[hat: audit — nothing may change; read, run the tests and the product, and \
                 ask check_package what is known against the dependencies. When asked for an \
                 audit, end with findings, blocking ones first, then your verdict]",
            ),
            Self::SoloScribe => Some(
                "[hat: scribe — write documentation only: .md, .txt and their kind. Read \
                 anything; change no code. Say what you read]",
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
            Self::SoloAudit => "audit",
            Self::SoloScribe => "scribe",
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What `[ui] start_hat` may say.
pub const START_HATS: &[&str] = &["plan", "build", "audit", "last"];

/// The hat a new session opens in, from `[ui] start_hat` and the hat this
/// project's latest session ended in. `last` with no earlier session is
/// plan; so is a value that isn't one of [`START_HATS`].
pub fn start_hat(setting: &str, last: Option<Role>) -> Role {
    match setting.trim().to_ascii_lowercase().as_str() {
        "build" => Role::SoloBuild,
        "audit" | "review" => Role::SoloAudit,
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
            "audit" | "review" => Ok(Self::SoloAudit),
            "scribe" => Ok(Self::SoloScribe),
            other => Err(Error::Config(format!(
                "unknown hat {other:?}: build, plan, audit, or scribe"
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
            ("build", Some(Role::SoloAudit), Role::SoloBuild),
            ("audit", None, Role::SoloAudit),
            // The audit hat's old name still names it.
            ("review", None, Role::SoloAudit),
            (" Build ", None, Role::SoloBuild),
            ("last", None, Role::SoloPlan),
            ("last", Some(Role::SoloBuild), Role::SoloBuild),
            ("last", Some(Role::SoloAudit), Role::SoloAudit),
            ("last", Some(Role::SoloScribe), Role::SoloScribe),
            // The scribe is not a hat to start in; `last` may say it.
            ("scribe", None, Role::SoloPlan),
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
        assert!(!Role::SoloPlan.writes_source() && !Role::SoloAudit.writes_source());
        assert!(!Role::SoloScribe.writes_source());
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
        assert_eq!(Role::SoloAudit.hat(), Role::SoloAudit);
        // Nobody can ask for one.
        assert!("builder".parse::<Role>().is_err());
        assert!("crew".parse::<Role>().is_err());
        assert!("test".parse::<Role>().is_err());
    }

    /// `Tab` moves within a row, `Shift+Tab` between rows
    /// (`docs/specialists-design.md` §3).
    #[test]
    fn tab_moves_within_a_row() {
        assert_eq!(Role::SoloPlan.row(), Row::Primary);
        assert_eq!(Role::SoloBuild.row(), Row::Primary);
        assert_eq!(Role::SoloAudit.row(), Row::Specialist);
        assert_eq!(Role::SoloScribe.row(), Row::Specialist);
        assert_eq!(Role::Crew.row(), Row::Primary);
        // Plan and build trade places.
        assert_eq!(Role::SoloPlan.next_in_row(), Role::SoloBuild);
        assert_eq!(Role::SoloBuild.next_in_row(), Role::SoloPlan);
        // The specialists go round their own row.
        assert_eq!(Role::SoloAudit.next_in_row(), Role::SoloScribe);
        assert_eq!(Role::SoloScribe.next_in_row(), Role::SoloAudit);
        assert_eq!(Role::SoloScribe.other_row_default(), Role::SoloBuild);
        assert_eq!("scribe".parse::<Role>().unwrap(), Role::SoloScribe);
        assert_eq!(
            serde_json::to_string(&Role::SoloScribe).unwrap(),
            "\"scribe\""
        );
        // A role from crew mode is build.
        assert_eq!(Role::Crew.next_in_row(), Role::SoloPlan);
        // The other row opens on build, or on audit.
        assert_eq!(Role::SoloPlan.other_row_default(), Role::SoloAudit);
        assert_eq!(Role::SoloBuild.other_row_default(), Role::SoloAudit);
        assert_eq!(Role::SoloAudit.other_row_default(), Role::SoloBuild);
        assert_eq!(Row::Primary.other(), Row::Specialist);
        assert_eq!(Row::Specialist.default_hat(), Role::SoloAudit);
        // Round-trips as the hat name, in logs and sessions; the old name
        // still loads.
        assert_eq!(
            serde_json::to_string(&Role::SoloAudit).unwrap(),
            "\"audit\""
        );
        let old: Role = serde_json::from_str("\"review\"").unwrap();
        assert_eq!(old, Role::SoloAudit);
        assert_eq!("review".parse::<Role>().unwrap(), Role::SoloAudit);
        assert_eq!("audit".parse::<Role>().unwrap(), Role::SoloAudit);
        assert_eq!(Role::SoloAudit.as_str(), "audit");
        assert_eq!("plan".parse::<Role>().unwrap(), Role::SoloPlan);
    }
}
