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

    /// Session spend cap hit.
    #[error("budget exceeded (${spent:.4} >= ${cap:.2})")]
    Budget {
        /// USD spent so far (known prices only).
        spent: f64,
        /// Configured cap.
        cap: f64,
    },

    /// One crew task hit its dollar or token cap. Stops that task only.
    #[error("task budget: {0}")]
    TaskBudget(String),

    /// In-flight turn was cancelled.
    #[error("cancelled")]
    Cancelled,
}

/// Result alias for [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
