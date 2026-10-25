//! What a check command's output comes to: the errors in it, as blocks a
//! person or a model can act on, capped so the cost of reading them is
//! bounded. Written for rustc, tsc, pyright, go and the C compilers, and
//! for anything else that prints the word `error` at the head of a line.

/// The most error blocks a report carries.
pub const MAX_BLOCKS: usize = 20;
/// The most characters a report carries, the count line aside.
pub const MAX_CHARS: usize = 3_000;
/// Lines a block may run to after its first: a compiler's context
/// (`-->`, `|`, `=`), not its whole explanation.
const BLOCK_LINES: usize = 8;

/// Whether a line opens an error block.
fn opens(line: &str) -> bool {
    let t = line.trim_start();
    if t.is_empty() {
        return false;
    }
    let low = t.to_ascii_lowercase();
    // rustc's closing lines are about the errors, not one of them.
    if low.starts_with("error: aborting")
        || low.starts_with("error: could not compile")
        || low.contains("previous error")
        || low.starts_with("warning")
        || low.contains(" - warning:")
        || low.contains(": warning:")
    {
        return false;
    }
    // `error[E0308]: …`, `error: …`, `src/x.ts(1,2): error TS…`,
    // `x.py:1:2 - error: …`, `x.c:1:2: error: …`, `ERROR: …`.
    if low.starts_with("error") && low[5..].starts_with([':', '[', ' ']) {
        return true;
    }
    if low.contains(": error") || low.contains(" - error:") || low.contains(" error ts") {
        return true;
    }
    // Go prints `./x.go:12:5: message` with no word for it.
    location(t)
}

/// `path:line:col` or `path:line` at the head of a line, with a path that
/// has a file extension: Go's and `make`'s way of saying where.
fn location(t: &str) -> bool {
    let Some((path, rest)) = t.split_once(':') else {
        return false;
    };
    if path.is_empty() || !path.contains('.') || path.contains(' ') {
        return false;
    }
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    !digits.is_empty() && rest[digits.len()..].starts_with(':')
}

/// Whether a line continues the block before it: a compiler's context,
/// indented or led by its own marks.
fn continues(line: &str) -> bool {
    !line.trim().is_empty()
        && (line.starts_with(' ')
            || line.starts_with('\t')
            || line.starts_with("-->")
            || line.starts_with('|')
            || line.starts_with('='))
}

/// The error blocks in `output`, first to last, each trimmed at the end.
pub fn blocks(output: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut open: Option<(String, usize)> = None;
    for line in output.lines() {
        if opens(line) {
            if let Some((b, _)) = open.take() {
                out.push(b);
            }
            open = Some((line.trim_end().to_string(), 0));
        } else if let Some((b, n)) = open.as_mut() {
            if continues(line) && *n < BLOCK_LINES {
                b.push('\n');
                b.push_str(line.trim_end());
                *n += 1;
            } else if !continues(line) {
                out.push(open.take().unwrap().0);
            }
        }
    }
    if let Some((b, _)) = open {
        out.push(b);
    }
    out
}

/// A check's report: how many errors, and the text that says them. A
/// check that is a test run (`cargo test`, `pytest`) reports its failed
/// tests (`failures::digest`), counted the same. With neither in a failed
/// run, the output's last lines stand in; a run that passed says so in a
/// word.
pub fn report(output: &str, ok: bool) -> (usize, String) {
    // A test runner's failures first: its `path:line: message` lines
    // would otherwise read as a compiler's.
    if !ok {
        if let Some(d) = crate::failures::digest(output).filter(|d| !d.failures.is_empty()) {
            return (d.failures.len(), d.text());
        }
    }
    let blocks = blocks(output);
    if blocks.is_empty() {
        return if ok {
            (0, "passed".into())
        } else {
            (
                0,
                format!("failed\n{}", crate::run::last_lines(output.trim_end(), 12)),
            )
        };
    }
    let n = blocks.len();
    let mut text = format!("{n} error{}\n", if n == 1 { "" } else { "s" });
    let mut shown = 0;
    for b in blocks.iter().take(MAX_BLOCKS) {
        if text.len() + b.len() > MAX_CHARS && shown > 0 {
            break;
        }
        text.push('\n');
        text.push_str(b);
        text.push('\n');
        shown += 1;
    }
    if shown < n {
        text.push_str(&format!("\n… and {} more\n", n - shown));
    }
    (n, text.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rustc_errors_are_blocks_with_their_location_and_the_closing_lines_are_not() {
        let out = "   Compiling app v0.1.0\n\
            error[E0308]: mismatched types\n \
            --> src/main.rs:4:9\n  |\n4 |     let x: u32 = \"a\";\n  |            ---   ^^^ expected `u32`, found `&str`\n\n\
            warning: unused variable: `y`\n \
            --> src/main.rs:5:9\n\n\
            error: aborting due to 1 previous error; 1 warning emitted\n\n\
            error: could not compile `app` (bin \"app\") due to 1 previous error\n[exit 101]";
        let b = blocks(out);
        assert_eq!(b.len(), 1, "{b:?}");
        assert!(b[0].starts_with("error[E0308]: mismatched types\n --> src/main.rs:4:9"));
        let (n, text) = report(out, false);
        assert_eq!(n, 1);
        assert!(text.starts_with("1 error\n\nerror[E0308]"), "{text}");
    }

    #[test]
    fn tsc_pyright_go_and_cc_errors_are_counted() {
        let tsc = "src/app.ts(12,5): error TS2322: Type 'string' is not assignable to type 'number'.\n\nFound 1 error in src/app.ts:12\n";
        assert_eq!(blocks(tsc).len(), 1);
        let py = "/w/app.py:3:5 - error: \"x\" is not defined (reportUndefinedVariable)\n  /w/app.py:9:1 - warning: unused\n1 error, 1 warning, 0 informations\n";
        assert_eq!(blocks(py).len(), 1);
        let go =
            "# example.com/app\n./main.go:7:2: undefined: fmt\n./main.go:9:1: missing return\n";
        assert_eq!(blocks(go).len(), 2);
        let cc = "main.c:4:3: error: expected ';' before 'return'\n    4 |   int x = 1\n      |            ^\n      |            ;\n";
        let b = blocks(cc);
        assert_eq!(b.len(), 1);
        assert!(b[0].lines().count() == 4, "{b:?}");
    }

    #[test]
    fn a_clean_run_passes_and_a_silent_failure_shows_its_tail() {
        assert_eq!(
            report("   Checking app\n    Finished\n", true),
            (0, "passed".into())
        );
        let (n, text) = report(
            "make: *** No rule to make target 'check'.  Stop.\n[exit 2]",
            false,
        );
        assert_eq!(n, 0);
        assert!(text.starts_with("failed\nmake: ***"), "{text}");
    }

    /// A check command that runs tests reports the tests that failed,
    /// counted as the errors are, so the model is told what failed and not
    /// the output's last lines.
    #[test]
    fn a_check_that_is_a_test_run_reports_its_failed_tests() {
        let out = "=== FAILURES ===\n___ test_add ___\n>       assert add(1, 2) == 4\nE       assert 3 == 4\n\ntests/test_x.py:5: AssertionError\n=== 1 failed, 2 passed in 0.03s ===\n[exit 1]";
        let (n, text) = report(out, false);
        assert_eq!(n, 1);
        assert!(
            text.starts_with(
                "1 failed\n\n✗ test_add\n>       assert add(1, 2) == 4\nE       assert 3 == 4"
            ),
            "{text}"
        );
        assert!(text.ends_with("1 failed, 2 passed in 0.03s"), "{text}");
    }

    #[test]
    fn the_report_is_capped() {
        let many: String = (0..30)
            .map(|i| format!("error: number {i}\n --> src/f{i}.rs:1:1\n\n"))
            .collect();
        let (n, text) = report(&many, false);
        assert_eq!(n, 30);
        assert!(text.starts_with("30 errors\n"));
        assert!(text.ends_with("… and 10 more"), "{text}");
        let long: String = (0..6)
            .map(|i| format!("error: {}\n", "x".repeat(900 + i)))
            .collect();
        let (_, text) = report(&long, false);
        assert!(text.len() < MAX_CHARS + 1_000, "{}", text.len());
        assert!(text.contains("… and"), "{text}");
    }
}
