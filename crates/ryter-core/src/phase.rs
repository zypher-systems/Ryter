//! Orchestrator phase: which specialist kinds may run.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::error::Error;
use crate::role::Role;

/// Campaign phase. Constrains which specialists the orchestrator may spawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    /// Spawn planners only.
    Plan,
    /// Spawn architects only.
    Architect,
    /// Spawn builders (and auditors as the merge gate).
    Build,
    /// Spawn extra review specialists.
    Audit,
}

impl Phase {
    /// Default entry: Build.
    pub const DEFAULT: Self = Self::Build;

    /// Specialist roles this phase is allowed to run.
    pub fn allowed_roles(self) -> &'static [Role] {
        match self {
            Self::Plan => &[Role::Planner],
            Self::Architect => &[Role::Architect],
            Self::Build => &[Role::Builder, Role::Auditor],
            Self::Audit => &[Role::Auditor],
        }
    }

    /// Whether `role` may be spawned in this phase.
    pub fn allows(self, role: Role) -> bool {
        self.allowed_roles().contains(&role)
    }

    /// Stable lowercase name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Architect => "architect",
            Self::Build => "build",
            Self::Audit => "audit",
        }
    }
}

impl Default for Phase {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Phase {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "plan" => Ok(Self::Plan),
            "architect" => Ok(Self::Architect),
            "build" => Ok(Self::Build),
            "audit" => Ok(Self::Audit),
            other => Err(Error::Config(format!("unknown phase {other:?}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_build() {
        assert_eq!(Phase::default(), Phase::Build);
    }

    #[test]
    fn build_allows_builders_and_auditors() {
        assert!(Phase::Build.allows(Role::Builder));
        assert!(Phase::Build.allows(Role::Auditor));
        assert!(!Phase::Build.allows(Role::Planner));
        assert!(!Phase::Plan.allows(Role::Builder));
    }

    #[test]
    fn parse_round_trip() {
        for phase in [Phase::Plan, Phase::Architect, Phase::Build, Phase::Audit] {
            assert_eq!(phase.as_str().parse::<Phase>().unwrap(), phase);
        }
    }
}
