use std::fmt;

/// Why a duration didn't parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The input is empty.
    Empty,
    /// Digits with no unit after them.
    MissingUnit,
    /// A unit that isn't known, or a part with no digits.
    UnknownUnit(char),
    /// Units not largest first, or repeated.
    OutOfOrder,
    /// The total doesn't fit in a `u64`.
    Overflow,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Empty => write!(f, "empty duration"),
            ParseError::MissingUnit => write!(f, "a number needs a unit"),
            ParseError::UnknownUnit(c) => write!(f, "unknown unit {c:?}"),
            ParseError::OutOfOrder => write!(f, "units go largest first, once each"),
            ParseError::Overflow => write!(f, "the duration is too long"),
        }
    }
}

impl std::error::Error for ParseError {}
