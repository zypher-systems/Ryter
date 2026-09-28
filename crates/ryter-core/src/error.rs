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
    #[error("{}", budget_message(*spent, *cap, unpriced.as_deref()))]
    Budget {
        /// USD spent so far (known prices only).
        spent: f64,
        /// Configured cap.
        cap: f64,
        /// A model whose call had no price, so the cap could not see it.
        unpriced: Option<String>,
    },

    /// One crew task hit its dollar or token cap. Stops that task only.
    #[error("task budget: {0}")]
    TaskBudget(String),

    /// In-flight turn was cancelled.
    #[error("cancelled")]
    Cancelled,
}

fn budget_message(spent: f64, cap: f64, unpriced: Option<&str>) -> String {
    match unpriced {
        Some(model) => format!(
            "the ${cap:.2} budget can't see what {model} costs: it has no price, so \
             the budget would never stop it. Give it one under [pricing] in \
             config.toml, or turn the budget off (`/budget off`) to go on without one"
        ),
        None => format!("budget exceeded (${spent:.4} >= ${cap:.2})"),
    }
}

/// Result alias for [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
