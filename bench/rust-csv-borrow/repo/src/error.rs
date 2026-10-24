use std::fmt;

/// What stops the reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A quoted field that never closed; `line` is where its record began.
    UnterminatedQuote { line: usize },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnterminatedQuote { line } => write!(f, "line {line}: quoted field never closed"),
        }
    }
}

impl std::error::Error for Error {}
