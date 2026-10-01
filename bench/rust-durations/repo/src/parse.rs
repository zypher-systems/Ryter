use crate::error::ParseError;

/// Seconds in `input`. Only whole seconds (`45s`) are understood so far.
pub fn parse(input: &str) -> Result<u64, ParseError> {
    if input.is_empty() {
        return Err(ParseError::Empty);
    }
    match input.strip_suffix('s') {
        Some(digits) => digits.parse().map_err(|_| ParseError::MissingUnit),
        None => Err(ParseError::MissingUnit),
    }
}
