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
    Plan,
    /// Build hat: changing the user's tree directly, behind the permission
    /// gate (edits and non-read-only commands ask).
    #[serde(rename = "build")]
    Build,
    /// Audit hat: critiquing what changed. Runs tests and linters; edits
    /// nothing. It was the review hat until 0.16, and loads under that name.
    #[serde(rename = "audit", alias = "review")]
    Audit,
    /// Scribe hat: documentation only. Reads everything, writes `.md`,
    /// `.txt` and their kind, runs read-only commands; changes no code.
    #[serde(rename = "scribe")]
    Scribe,
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
            Self::Primary => Role::Build,
            Self::Specialist => Role::Audit,
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
            Self::Plan | Self::Build | Self::Crew => Row::Primary,
            Self::Audit | Self::Scribe => Row::Specialist,
        }
    }

    /// The hat `Tab` moves to: the next in this hat's row. Plan and build
    /// trade places; the specialists go round their own row.
    pub fn next_in_row(self) -> Self {
        match self {
            Self::Plan => Self::Build,
            Self::Build | Self::Crew => Self::Plan,
            Self::Audit => Self::Scribe,
            Self::Scribe => Self::Audit,
        }
    }

    /// The hat the other row opens on when nothing there was worn yet.
    pub fn other_row_default(self) -> Self {
        self.row().other().default_hat()
    }

    /// Whether this hat may change the project's files.
    pub fn writes_source(self) -> bool {
        self == Self::Build
    }

    /// A hat a turn can be in: everything but [`Role::Crew`] (a role name
    /// left from crew mode, which only still loads).
    pub fn is_hat(self) -> bool {
        self != Self::Crew
    }

    /// The hat a saved session opens in: its own, or build for a session
    /// from crew mode.
    pub fn hat(self) -> Self {
        if self.is_hat() { self } else { Self::Build }
    }

    /// The line put in front of each message in a hat, so the model knows
    /// what it may do this turn without the system prompt (and the prompt
    /// cache) changing on every switch.
    pub fn hat_note(self) -> Option<&'static str> {
        match self {
            // What the hat allows, not orders: a directive here made the model
            // start working on "are you there?".
            Self::Build => Some(
                "[hat: build — you may change files and run commands when the request calls for it]",
            ),
            Self::Plan => Some(
                "[hat: plan — nothing may change; read and think. check_package tells you a \
                 dependency's latest release and its known advisories; ask it before you pin a \
                 version. web_search and web_fetch are yours for what is current on the web. \
                 A question that takes reading several files goes to scout, which answers with \
                 paths and lines. When you have a plan, show it with present_plan: goal, steps, \
                 files, risks, and how to verify]",
            ),
            Self::Audit => Some(
                "[hat: audit — nothing may change; read, run the tests and the product, and \
                 ask check_package what is known against the dependencies; a question that \
                 takes reading several files goes to scout. When asked for an audit, end with \
                 findings, blocking ones first, then your verdict]",
            ),
            Self::Scribe => Some(
                "[hat: scribe — write documentation only: .md, .txt and their kind. Read \
                 anything; change no code. Say what you read]",
            ),
            Self::Crew => None,
        }
    }

    /// The line a specialist's composer opens on when the hat is chosen:
    /// choosing the hat and pressing Enter is the whole request. The hats
    /// above the line are told what to do in the user's own words.
    pub fn opening(self) -> Option<&'static str> {
        match self {
            Self::Audit => Some("Audit this project"),
            Self::Scribe => Some("Document this project"),
            Self::Plan | Self::Build | Self::Crew => None,
        }
    }

    /// What the hat does, in a few words, for the screen's lists.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Plan => "reads and proposes, changes nothing",
            Self::Build | Self::Crew => "makes the changes",
            Self::Audit => "checks the work, may run it",
            Self::Scribe => "writes the docs, changes no code",
        }
    }

    /// Stable lowercase name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Crew => "crew",
            Self::Plan => "plan",
            Self::Build => "build",
            Self::Audit => "audit",
            Self::Scribe => "scribe",
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
        "build" => Role::Build,
        "audit" | "review" => Role::Audit,
        "last" => last.map_or(Role::Plan, Role::hat),
        _ => Role::Plan,
    }
}

impl FromStr for Role {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "plan" => Ok(Self::Plan),
            "build" => Ok(Self::Build),
            "audit" | "review" => Ok(Self::Audit),
            "scribe" => Ok(Self::Scribe),
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
            ("plan", None, Role::Plan),
            ("build", Some(Role::Audit), Role::Build),
            ("audit", None, Role::Audit),
            // The audit hat's old name still names it.
            ("review", None, Role::Audit),
            (" Build ", None, Role::Build),
            ("last", None, Role::Plan),
            ("last", Some(Role::Build), Role::Build),
            ("last", Some(Role::Audit), Role::Audit),
            ("last", Some(Role::Scribe), Role::Scribe),
            // The scribe is not a hat to start in; `last` may say it.
            ("scribe", None, Role::Plan),
            // A session from crew mode, or one left in the test hat that
            // was, opens in build.
            ("last", Some(Role::Crew), Role::Build),
            // Test is no hat, and neither is a typo.
            ("test", Some(Role::Build), Role::Plan),
            ("bulid", None, Role::Plan),
            ("", None, Role::Plan),
        ] {
            assert_eq!(start_hat(setting, last), want, "{setting:?} {last:?}");
        }
    }

    #[test]
    fn only_the_build_hat_writes_source() {
        assert!(Role::Build.writes_source());
        assert!(!Role::Plan.writes_source() && !Role::Audit.writes_source());
        assert!(!Role::Scribe.writes_source());
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
            assert_eq!(role.hat(), Role::Build);
            assert!(!role.is_hat());
        }
        assert_eq!(Role::Audit.hat(), Role::Audit);
        // Nobody can ask for one.
        assert!("builder".parse::<Role>().is_err());
        assert!("crew".parse::<Role>().is_err());
        assert!("test".parse::<Role>().is_err());
    }

    /// `Tab` moves within a row, `Shift+Tab` between rows
    /// (`docs/specialists-design.md` §3).
    #[test]
    fn tab_moves_within_a_row() {
        assert_eq!(Role::Plan.row(), Row::Primary);
        assert_eq!(Role::Build.row(), Row::Primary);
        assert_eq!(Role::Audit.row(), Row::Specialist);
        assert_eq!(Role::Scribe.row(), Row::Specialist);
        assert_eq!(Role::Crew.row(), Row::Primary);
        // Plan and build trade places.
        assert_eq!(Role::Plan.next_in_row(), Role::Build);
        assert_eq!(Role::Build.next_in_row(), Role::Plan);
        // The specialists go round their own row.
        assert_eq!(Role::Audit.next_in_row(), Role::Scribe);
        assert_eq!(Role::Scribe.next_in_row(), Role::Audit);
        assert_eq!(Role::Scribe.other_row_default(), Role::Build);
        assert_eq!("scribe".parse::<Role>().unwrap(), Role::Scribe);
        assert_eq!(serde_json::to_string(&Role::Scribe).unwrap(), "\"scribe\"");
        // A role from crew mode is build.
        assert_eq!(Role::Crew.next_in_row(), Role::Plan);
        // The other row opens on build, or on audit.
        assert_eq!(Role::Plan.other_row_default(), Role::Audit);
        assert_eq!(Role::Build.other_row_default(), Role::Audit);
        assert_eq!(Role::Audit.other_row_default(), Role::Build);
        assert_eq!(Row::Primary.other(), Row::Specialist);
        assert_eq!(Row::Specialist.default_hat(), Role::Audit);
        // Round-trips as the hat name, in logs and sessions; the old name
        // still loads.
        assert_eq!(serde_json::to_string(&Role::Audit).unwrap(), "\"audit\"");
        let old: Role = serde_json::from_str("\"review\"").unwrap();
        assert_eq!(old, Role::Audit);
        assert_eq!("review".parse::<Role>().unwrap(), Role::Audit);
        assert_eq!("audit".parse::<Role>().unwrap(), Role::Audit);
        assert_eq!(Role::Audit.as_str(), "audit");
        assert_eq!("plan".parse::<Role>().unwrap(), Role::Plan);
    }
}
