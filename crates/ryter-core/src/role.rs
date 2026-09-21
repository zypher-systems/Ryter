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
}

impl Role {
    /// Whether this role may mutate product source (not pass notes).
    pub fn writes_source(self) -> bool {
        matches!(self, Self::Builder)
    }

    /// Stable lowercase name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Orchestrator => "orchestrator",
            Self::Architect => "architect",
            Self::Builder => "builder",
            Self::Auditor => "auditor",
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
            "orchestrator" => Ok(Self::Orchestrator),
            "architect" | "planner" | "plan" => Ok(Self::Architect),
            "builder" | "build" => Ok(Self::Builder),
            "auditor" | "audit" => Ok(Self::Auditor),
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
    }
}
