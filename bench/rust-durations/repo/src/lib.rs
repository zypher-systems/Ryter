//! Durations as text: `1h30m` is 5400 seconds.

pub mod error;
pub mod format;
pub mod parse;

pub use error::ParseError;
pub use format::format;
pub use parse::parse;
