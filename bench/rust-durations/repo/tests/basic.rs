use durfmt::{format, parse, ParseError};

#[test]
fn seconds_still_parse() {
    assert_eq!(parse("45s"), Ok(45));
}

#[test]
fn hours_and_minutes_parse() {
    assert_eq!(parse("1h30m"), Ok(5400));
}

#[test]
fn hours_and_minutes_format() {
    assert_eq!(format(5400), "1h30m");
}

#[test]
fn empty_is_an_error() {
    assert_eq!(parse(""), Err(ParseError::Empty));
}
