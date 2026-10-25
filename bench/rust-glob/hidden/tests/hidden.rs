use globlite::{Error, Options, Pattern};

fn matches(pattern: &str, path: &str) -> bool {
    Pattern::new(pattern).unwrap().matches(path)
}

fn with(pattern: &str, options: Options, path: &str) -> bool {
    Pattern::with_options(pattern, options).unwrap().matches(path)
}

#[test]
fn stars_and_marks_never_cross_a_separator() {
    assert!(matches("*", "file"));
    assert!(matches("*", ""));
    assert!(!matches("*", "a/b"));
    assert!(matches("a/*/c", "a/b/c"));
    assert!(!matches("a/*/c", "a/b/x/c"));
    assert!(matches("a/*/c", "a//c"));
    assert!(!matches("a?b", "a/b"));
    assert!(matches("*a*", "banana"));
    assert!(!matches("*a*", "x/anna"));
}

#[test]
fn double_star_is_zero_or_more_whole_segments() {
    assert!(matches("a/**/b", "a/b"));
    assert!(matches("a/**/b", "a/x/b"));
    assert!(matches("a/**/b", "a/x/y/b"));
    assert!(!matches("a/**/b", "a/xb"));
    assert!(!matches("a/**/b", "ab"));
    assert!(matches("**/b", "b"));
    assert!(matches("**/b", "x/y/b"));
    assert!(!matches("**/b", "x/y/bb"));
    assert!(matches("a/**", "a"));
    assert!(matches("a/**", "a/x"));
    assert!(matches("a/**", "a/x/y"));
    assert!(!matches("a/**", "ab"));
    assert!(matches("**", "anything/at/all"));
    assert!(matches("**", ""));
    assert!(matches("**/*.rs", "src/a/b.rs"));
    assert!(matches("**/*.rs", "b.rs"));
}

#[test]
fn a_double_star_inside_a_segment_is_one_star() {
    assert!(matches("a**b", "axyzb"));
    assert!(matches("a**b", "ab"));
    assert!(!matches("a**b", "a/b"));
    assert!(matches("**.rs", "main.rs"));
    assert!(!matches("**.rs", "src/main.rs"));
}

#[test]
fn classes_ranges_negation_and_literals() {
    assert!(matches("[a-cx]", "b"));
    assert!(matches("[a-cx]", "x"));
    assert!(!matches("[a-cx]", "d"));
    assert!(matches("[!abc]", "d"));
    assert!(!matches("[!abc]", "a"));
    assert!(matches("[^0-9]", "x"));
    assert!(!matches("[^0-9]", "5"));
    // `]` first is a literal; `-` first or last is a literal.
    assert!(matches("[]a]", "]"));
    assert!(matches("[]a]", "a"));
    assert!(!matches("[]a]", "b"));
    assert!(matches("[!]a]", "b"));
    assert!(!matches("[!]a]", "]"));
    assert!(matches("[-a]", "-"));
    assert!(matches("[a-]", "-"));
    assert!(!matches("[a-]", "b"));
    // A class never matches a separator, negated or not.
    assert!(!matches("a[!x]b", "a/b"));
    assert!(!matches("a[/]b", "a/b"));
}

#[test]
fn escapes_make_literals() {
    assert!(matches("\\*.rs", "*.rs"));
    assert!(!matches("\\*.rs", "main.rs"));
    assert!(matches("a\\?b", "a?b"));
    assert!(!matches("a\\?b", "axb"));
    assert!(matches("\\[x\\]", "[x]"));
    assert!(matches("a\\\\b", "a\\b"));
    assert!(matches("[\\]", "\\"));
}

#[test]
fn errors_say_where() {
    assert_eq!(
        Pattern::new("src/[abc").unwrap_err(),
        Error::UnterminatedClass { position: 4 }
    );
    assert_eq!(
        Pattern::new("[]").unwrap_err(),
        Error::UnterminatedClass { position: 0 }
    );
    assert_eq!(
        Pattern::new("[!]").unwrap_err(),
        Error::UnterminatedClass { position: 0 }
    );
    assert_eq!(
        Pattern::new("ab\\").unwrap_err(),
        Error::TrailingEscape { position: 2 }
    );
    assert_eq!(Pattern::new("ok").unwrap().as_str(), "ok");
}

#[test]
fn case_insensitive_covers_literals_and_classes() {
    let ci = Options {
        case_insensitive: true,
        ..Options::default()
    };
    assert!(with("README.md", ci, "readme.MD"));
    assert!(!matches("README.md", "readme.MD"));
    assert!(with("[a-c]x", ci, "Bx"));
    assert!(with("[A-C]x", ci, "bx"));
    assert!(!with("[a-c]x", ci, "dx"));
    assert!(!with("[!a-c]x", ci, "Bx"));
    assert!(with("*.RS", ci, "main.rs"));
}

#[test]
fn a_literal_leading_dot_can_be_required() {
    let dot = Options {
        require_literal_leading_dot: true,
        ..Options::default()
    };
    assert!(matches("*", ".git"));
    assert!(!with("*", dot, ".git"));
    assert!(with(".*", dot, ".git"));
    assert!(with("*", dot, "git"));
    assert!(!with("?git", dot, ".git"));
    assert!(!with("[.a]git", dot, ".git"));
    assert!(with("[.a]git", dot, "agit"));
    assert!(!with("src/*", dot, "src/.env"));
    assert!(with("src/.*", dot, "src/.env"));
    assert!(!with("src/*.env", dot, "src/.env"));
    assert!(with("a.*", dot, "a.b"));
    // `**` does not step through a dot segment either.
    assert!(!with("**/x", dot, ".hidden/x"));
    assert!(with("**/x", dot, "shown/x"));
    assert!(!with("a/**", dot, "a/.b"));
    assert!(!with("a/**", dot, "a/b/.c"));
    assert!(with("a/**", dot, "a/b/c"));
    assert!(with("**/.x", dot, "d/.x"));
}
