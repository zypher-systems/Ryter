//! The mark on what Ryter keeps, so what 1.0 writes, 1.x can read.
//!
//! Sessions, `config.toml`, `settings.toml`, `routing.toml`, `.ryter/run.toml`,
//! `.ryter/plan.md` and the spend records carry a schema number. A change to
//! one of them is an addition: a new field has a default, and a renamed field
//! reads both names. `tests/persisted.rs` holds every kept release to that.
//! The number moves only for a change that cannot be an addition, and an older
//! Ryter then refuses the file rather than reading it wrong, or saving it back
//! with what it did not understand left out.

use std::fmt::Display;

use crate::error::{Error, Result};

/// The schema this Ryter reads and writes.
pub const SCHEMA: u32 = 1;

/// The schema of a file with no mark: everything written before there was
/// one, and every file a person wrote and left unmarked.
pub const fn unmarked() -> u32 {
    1
}

/// Stop at a file written by a Ryter newer than this one. `what` names it.
pub fn refuse_newer(found: u32, what: impl Display) -> Result<()> {
    if found <= SCHEMA {
        return Ok(());
    }
    Err(Error::Config(format!(
        "{what} was written by a newer Ryter (schema {found}; this one reads up to {SCHEMA}). \
         Nothing was changed. `ryter update` installs the latest release."
    )))
}

/// The mark in a TOML file's text. One that is not a number is no mark, and
/// text that does not parse is for the file's own reader to report.
pub fn of_toml(text: &str) -> u32 {
    toml::from_str::<toml::Table>(text)
        .ok()
        .and_then(|table| table.get("schema")?.as_integer())
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(unmarked())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_with_no_mark_is_the_first_schema() {
        assert_eq!(of_toml("start = \"x\"\n"), 1);
        assert_eq!(of_toml("schema = \"two\"\n"), 1);
        assert_eq!(of_toml("[schema]\norder = []\n"), 1, "a connection's table");
        assert_eq!(of_toml("not toml ["), 1);
        assert_eq!(of_toml("schema = 3\n"), 3);
    }

    #[test]
    fn only_a_newer_number_is_refused_and_it_says_what_to_do() {
        assert!(refuse_newer(SCHEMA, "x").is_ok());
        let why = refuse_newer(SCHEMA + 1, "x.toml").unwrap_err().to_string();
        assert!(why.contains("x.toml"), "{why}");
        assert!(why.contains("`ryter update`"), "{why}");
    }
}
