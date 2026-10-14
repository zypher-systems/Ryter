//! Error types for Ryter core.

use thiserror::Error;

/// Recoverable failures in the harness core.
#[derive(Debug, Error)]
pub enum Error {
    /// A configuration value is invalid.
    #[error("{0}")]
    Config(String),

    /// An identifier could not be parsed.
    #[error("invalid id: {0}")]
    InvalidId(String),

    /// The inference provider failed.
    #[error("provider: {0}")]
    Provider(String),

    /// Filesystem failure.
    #[error("io: {0}")]
    Io(String),

    /// Session spend cap hit, or a call it could not price.
    #[error("{}", budget_message(*spent, *cap, unpriced.as_deref(), *incomplete))]
    Budget {
        /// USD spent so far (known prices only).
        spent: f64,
        /// Configured cap.
        cap: f64,
        /// A model whose call had no price, so the cap could not see it.
        unpriced: Option<String>,
        /// An interrupted request left this session's actual spend unknown.
        incomplete: bool,
    },

    /// An outbound MCP server failed, timed out, or went away.
    #[error("{0}")]
    Mcp(String),

    /// In-flight turn was cancelled.
    #[error("cancelled")]
    Cancelled,
}

fn budget_message(spent: f64, cap: f64, unpriced: Option<&str>, incomplete: bool) -> String {
    // Same voice as the round-cap stop: say what stopped, how to raise or
    // lift it, and that "continue" carries on.
    if incomplete {
        return format!(
            "stopped: the ${cap:.2} budget cannot account for an earlier request — \
             its usage record is incomplete (known spend is at least ${spent:.4}). \
             Start a new session, or `/budget off` to go on with unknown spend."
        );
    }
    match unpriced {
        Some(model) => format!(
            "stopped: the ${cap:.2} budget can't see what {model} costs — it has no \
             price, so the budget would never stop it. Give it one under [pricing] \
             in config.toml, or `/budget off` to go on without one. Say \"continue\" \
             once it has a price or the budget is off."
        ),
        None => format!(
            "stopped: session spend ${spent:.4} reached the ${cap:.2} budget \
             (/budget +N raises it, /budget N sets a new cap, /budget off lifts \
             it). Say \"continue\" to carry on."
        ),
    }
}

/// Result alias for [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_stop_matches_round_cap_voice() {
        let err = Error::Budget {
            spent: 5.0,
            cap: 5.0,
            unpriced: None,
            incomplete: false,
        };
        let msg = err.to_string();
        assert!(msg.starts_with("stopped:"), "{msg}");
        assert!(msg.contains("/budget +N"), "{msg}");
        assert!(msg.contains("Say \"continue\""), "{msg}");
    }
}
