use durfmt::{format, parse, ParseError};

#[test]
fn every_unit_parses() {
    assert_eq!(parse("1d"), Ok(86_400));
    assert_eq!(parse("2h"), Ok(7_200));
    assert_eq!(parse("90m"), Ok(5_400));
    assert_eq!(parse("0s"), Ok(0));
    assert_eq!(parse("1d1h1m1s"), Ok(90_061));
    assert_eq!(parse("2d30s"), Ok(172_830));
}

#[test]
fn a_number_needs_a_unit() {
    assert_eq!(parse("15"), Err(ParseError::MissingUnit));
    assert_eq!(parse("1h30"), Err(ParseError::MissingUnit));
}

#[test]
fn unknown_units_are_named() {
    assert_eq!(parse("5x"), Err(ParseError::UnknownUnit('x')));
    assert_eq!(parse("1h5w"), Err(ParseError::UnknownUnit('w')));
    assert_eq!(parse("h"), Err(ParseError::UnknownUnit('h')));
    assert_eq!(parse("1h 30m"), Err(ParseError::UnknownUnit(' ')));
}

#[test]
fn units_go_largest_first_and_once() {
    assert_eq!(parse("30m1h"), Err(ParseError::OutOfOrder));
    assert_eq!(parse("1h1h"), Err(ParseError::OutOfOrder));
    assert_eq!(parse("1s1d"), Err(ParseError::OutOfOrder));
}

#[test]
fn too_much_is_an_overflow_not_a_panic() {
    assert_eq!(parse("99999999999999999999999s"), Err(ParseError::Overflow));
    assert_eq!(parse("213503982334602d"), Err(ParseError::Overflow));
    assert_eq!(parse("18446744073709551615s"), Ok(u64::MAX));
    // The last day that fits has room for seven more hours, not eight.
    assert_eq!(parse("213503982334601d7h"), Ok(18_446_744_073_709_551_600));
    assert_eq!(parse("213503982334601d8h"), Err(ParseError::Overflow));
}

#[test]
fn formatting_leaves_out_zero_units() {
    assert_eq!(format(0), "0s");
    assert_eq!(format(59), "59s");
    assert_eq!(format(60), "1m");
    assert_eq!(format(86_400), "1d");
    assert_eq!(format(90_061), "1d1h1m1s");
    assert_eq!(format(172_830), "2d30s");
}

#[test]
fn format_and_parse_round_trip() {
    for n in [0, 1, 59, 60, 61, 3_599, 3_600, 86_399, 86_400, 1_234_567, u64::MAX] {
        assert_eq!(parse(&format(n)), Ok(n), "{n} as {}", format(n));
    }
}

#[test]
fn every_error_says_something() {
    for e in [
        ParseError::Empty,
        ParseError::MissingUnit,
        ParseError::UnknownUnit('x'),
        ParseError::OutOfOrder,
        ParseError::Overflow,
    ] {
        assert!(!e.to_string().is_empty());
    }
}
