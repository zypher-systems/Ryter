use globlite::Pattern;

fn matches(pattern: &str, path: &str) -> bool {
    Pattern::new(pattern).unwrap().matches(path)
}

#[test]
fn a_star_stays_in_its_segment() {
    assert!(matches("*.rs", "main.rs"));
    assert!(!matches("*.rs", "src/main.rs"));
    assert!(matches("src/*.rs", "src/main.rs"));
}

#[test]
fn a_question_mark_is_one_character() {
    assert!(matches("a?c", "abc"));
    assert!(!matches("a?c", "ac"));
    assert!(!matches("a?c", "a/c"));
}

#[test]
fn a_class_is_one_of_its_characters() {
    assert!(matches("[abc].txt", "b.txt"));
    assert!(!matches("[abc].txt", "d.txt"));
    assert!(matches("[0-9][0-9]", "42"));
}

#[test]
fn a_double_star_crosses_segments() {
    assert!(matches("src/**/*.rs", "src/a/b/c.rs"));
    assert!(matches("src/**/*.rs", "src/c.rs"));
    assert!(!matches("src/**/*.rs", "lib/c.rs"));
}
