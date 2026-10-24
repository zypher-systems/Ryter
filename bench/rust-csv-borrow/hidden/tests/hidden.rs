use std::borrow::Cow;

use csvlite::{Error, Reader};

fn rows(input: &str) -> Vec<Vec<String>> {
    Reader::new(input)
        .map(|r| r.unwrap().into_iter().map(|f| f.into_owned()).collect())
        .collect()
}

fn results(input: &str) -> Vec<Result<Vec<String>, Error>> {
    Reader::new(input)
        .map(|r| r.map(|fs| fs.into_iter().map(|f| f.into_owned()).collect()))
        .collect()
}

#[test]
fn doubled_quotes_and_line_ends_inside_quotes() {
    assert_eq!(rows("\"say \"\"hi\"\"\",x"), vec![vec!["say \"hi\"", "x"]]);
    assert_eq!(rows("\"two\nlines\",\"a\r\nb\"\nnext"), vec![vec!["two\nlines", "a\r\nb"], vec!["next"]]);
    assert_eq!(rows("\"\"\"\""), vec![vec!["\""]]);
}

#[test]
fn empty_fields_records_and_inputs() {
    assert!(rows("").is_empty());
    assert_eq!(rows("\n"), vec![vec![""]]);
    assert_eq!(rows("a\n\nb\n"), vec![vec!["a"], vec![""], vec!["b"]]);
    assert_eq!(rows("a,"), vec![vec!["a", ""]]);
    assert_eq!(rows(","), vec![vec!["", ""]]);
    assert_eq!(rows("\"\",\"\""), vec![vec!["", ""]]);
    assert_eq!(rows("a\r\n"), vec![vec!["a"]]);
}

#[test]
fn a_quote_inside_an_unquoted_field_is_a_character() {
    assert_eq!(rows("a\"b,c"), vec![vec!["a\"b", "c"]]);
    assert_eq!(rows("5' 11\",tall"), vec![vec!["5' 11\"", "tall"]]);
}

#[test]
fn the_byte_order_mark_is_skipped() {
    assert_eq!(rows("\u{feff}a,b"), vec![vec!["a", "b"]]);
    let first = Reader::new("\u{feff}a,b").next().unwrap().unwrap();
    assert!(matches!(first[0], Cow::Borrowed("a")));
}

#[test]
fn another_delimiter() {
    assert_eq!(
        Reader::new("a;b,c;\"d;e\"")
            .with_delimiter(';')
            .map(|r| r.unwrap().into_iter().map(|f| f.into_owned()).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        vec![vec!["a", "b,c", "d;e"]]
    );
}

#[test]
fn fields_borrow_unless_a_quote_was_doubled() {
    let input = "plain,\"quoted, with comma\",\"has \"\"one\"\"\",\"\"";
    let fields = Reader::new(input).next().unwrap().unwrap();
    assert!(matches!(fields[0], Cow::Borrowed(_)), "{:?}", fields[0]);
    assert!(matches!(fields[1], Cow::Borrowed(_)), "{:?}", fields[1]);
    assert!(matches!(fields[2], Cow::Owned(_)), "{:?}", fields[2]);
    assert!(matches!(fields[3], Cow::Borrowed(_)), "{:?}", fields[3]);
    assert_eq!(fields[1], "quoted, with comma");
    assert_eq!(fields[2], "has \"one\"");
    // Borrowed means a slice of the input itself.
    let Cow::Borrowed(slice) = fields[1] else { panic!() };
    let start = input.as_ptr() as usize;
    let at = slice.as_ptr() as usize;
    assert!(at >= start && at < start + input.len(), "borrowed from elsewhere");
}

#[test]
fn the_borrow_outlives_the_reader() {
    let input = String::from("keep,me\n");
    let fields = {
        let mut reader = Reader::new(&input);
        reader.next().unwrap().unwrap()
    };
    assert_eq!(fields, vec![Cow::Borrowed("keep"), Cow::Borrowed("me")]);
}

#[test]
fn an_unterminated_quote_names_the_line_its_record_began_on() {
    let got = results("a,b\nc,\"open\nstill open");
    assert_eq!(got.len(), 2);
    assert_eq!(got[0], Ok(vec!["a".to_string(), "b".to_string()]));
    assert_eq!(got[1], Err(Error::UnterminatedQuote { line: 2 }));
}

#[test]
fn a_character_after_a_closing_quote_is_an_error_with_its_place() {
    assert_eq!(results("\"a\"b"), vec![Err(Error::UnexpectedAfterQuote { line: 1, column: 4 })]);
    assert_eq!(
        results("x,\"y\" z,w"),
        vec![Err(Error::UnexpectedAfterQuote { line: 1, column: 6 })]
    );
    let got = results("a\nb\n\"c\"d\ne");
    assert_eq!(got.len(), 3);
    assert_eq!(got[2], Err(Error::UnexpectedAfterQuote { line: 3, column: 4 }));
    // A line end inside quotes counts as a line.
    assert_eq!(
        results("\"one\ntwo\"x"),
        vec![Err(Error::UnexpectedAfterQuote { line: 2, column: 5 })]
    );
    // Open, one doubled quote, close, then a stray character.
    assert_eq!(
        results("\"\"\"\"z"),
        vec![Err(Error::UnexpectedAfterQuote { line: 1, column: 5 })]
    );
    // Five quotes never close: open, two doubled, and nothing after.
    assert_eq!(results("\"\"\"\"\""), vec![Err(Error::UnterminatedQuote { line: 1 })]);
}

#[test]
fn nothing_follows_an_error() {
    let mut reader = Reader::new("\"a\"b\nc,d\ne");
    assert!(reader.next().unwrap().is_err());
    assert!(reader.next().is_none());
    assert!(reader.next().is_none());
}

#[test]
fn errors_display() {
    assert_eq!(Error::UnterminatedQuote { line: 3 }.to_string(), "line 3: quoted field never closed");
    assert!(Error::UnexpectedAfterQuote { line: 1, column: 4 }.to_string().contains("1"));
}
