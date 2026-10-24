//! A small CSV reader over a string slice.
pub mod error;
pub mod reader;

pub use error::Error;
pub use reader::Reader;
