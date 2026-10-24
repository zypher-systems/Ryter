use std::borrow::Cow;

use csvlite::Reader;

fn rows(input: &str) -> Vec<Vec<String>> {
    Reader::new(input)
        .map(|r| r.unwrap().into_iter().map(|f| f.into_owned()).collect())
        .collect()
}

#[test]
fn fields_and_records() {
    assert_eq!(rows("a,b\nc,d"), vec![vec!["a", "b"], vec!["c", "d"]]);
}

#[test]
fn a_quoted_field_keeps_its_comma() {
    assert_eq!(rows("\"a,b\",c"), vec![vec!["a,b", "c"]]);
}

#[test]
fn crlf_ends_a_record() {
    assert_eq!(rows("a\r\nb\r\n"), vec![vec!["a"], vec!["b"]]);
}

#[test]
fn fields_are_cows() {
    let first = Reader::new("x,y").next().unwrap().unwrap();
    let field: &Cow<'_, str> = &first[0];
    assert_eq!(field.as_ref(), "x");
}
