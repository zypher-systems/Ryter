use std::fmt;

/// What stops the reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A quoted field that never closed; `line` is where its record began.
    UnterminatedQuote { line: usize },
    /// Something other than the delimiter or a line end after a closing
    /// quote, at this 1-based line and character column.
    UnexpectedAfterQuote { line: usize, column: usize },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnterminatedQuote { line } => write!(f, "line {line}: quoted field never closed"),
            Error::UnexpectedAfterQuote { line, column } => {
                write!(f, "line {line}, column {column}: unexpected character after a closing quote")
            }
        }
    }
}

impl std::error::Error for Error {}
