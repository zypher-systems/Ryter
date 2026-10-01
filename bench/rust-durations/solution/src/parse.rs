use crate::error::ParseError;

/// Seconds in each unit, largest first.
pub(crate) const UNITS: [(char, u64); 4] = [('d', 86_400), ('h', 3_600), ('m', 60), ('s', 1)];

/// Seconds in `input`: `<digits><unit>` parts, largest unit first, once each.
pub fn parse(input: &str) -> Result<u64, ParseError> {
    if input.is_empty() {
        return Err(ParseError::Empty);
    }
    let mut total: u64 = 0;
    // The next unit must come after this place in `UNITS`.
    let mut next = 0;
    let mut digits = String::new();
    for c in input.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        let place = UNITS
            .iter()
            .position(|(unit, _)| *unit == c)
            .ok_or(ParseError::UnknownUnit(c))?;
        if digits.is_empty() {
            return Err(ParseError::UnknownUnit(c));
        }
        if place < next {
            return Err(ParseError::OutOfOrder);
        }
        next = place + 1;
        let n: u64 = digits.parse().map_err(|_| ParseError::Overflow)?;
        digits.clear();
        total = n
            .checked_mul(UNITS[place].1)
            .and_then(|secs| total.checked_add(secs))
            .ok_or(ParseError::Overflow)?;
    }
    if digits.is_empty() {
        Ok(total)
    } else {
        Err(ParseError::MissingUnit)
    }
}
