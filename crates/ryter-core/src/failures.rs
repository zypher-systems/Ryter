//! What a test run's output comes to: the tests that failed, each with
//! what it said and where, and the runner's own totals, capped so the cost
//! of reading them is bounded. Written for cargo test, go test, pytest,
//! unittest, jest, vitest and mocha; an output no runner is recognised in
//! keeps its last lines.
//!
//! A runner prints its failures in the middle of its output and its totals
//! at the end. The last lines, which is what the model used to be handed,
//! carry the totals and the names, and the assertion that failed was above
//! them: the model spent a round running one test again to read it, or
//! guessed at the fix.

use crate::run::last_lines;

/// The most failures a report carries.
pub const MAX_FAILURES: usize = 10;
/// The most characters a report carries, the count line aside.
pub const MAX_CHARS: usize = 4_000;
/// Lines of detail one failure may run to.
const DETAIL_LINES: usize = 12;
/// Lines of totals a report carries: one per test binary or package.
const MAX_TOTALS: usize = 8;

/// One failed test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The test as the runner names it.
    pub name: String,
    /// What it said: the assertion, the message, where. Dedented, capped.
    pub detail: Vec<String>,
}

/// A test run's failures and totals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digest {
    /// The runner recognised.
    pub runner: &'static str,
    /// The failures, in the runner's order.
    pub failures: Vec<Failure>,
    /// The runner's own totals, as it printed them, trimmed.
    pub totals: Vec<String>,
}

impl Digest {
    /// The report: the count, each failure under a `✗` line, how many were
    /// left out, and the totals last, where a reader of the tail finds them.
    pub fn text(&self) -> String {
        let n = self.failures.len();
        let mut s = format!("{n} failed\n");
        let mut shown = 0;
        for f in self.failures.iter().take(MAX_FAILURES) {
            let block = f.block();
            if s.len() + block.len() > MAX_CHARS && shown > 0 {
                break;
            }
            s.push('\n');
            s.push_str(&block);
            s.push('\n');
            shown += 1;
        }
        if shown < n {
            s.push_str(&format!("\n… and {} more\n", n - shown));
        }
        if !self.totals.is_empty() {
            s.push('\n');
            s.push_str(&self.totals.join("\n"));
        }
        s.trim_end().to_string()
    }
}

impl Failure {
    fn block(&self) -> String {
        let mut s = format!("✗ {}", self.name);
        for l in &self.detail {
            s.push('\n');
            s.push_str(l);
        }
        s
    }
}

/// The failures and totals in `output`, by the first runner that recognises
/// it. `None` when no runner does.
pub fn digest(output: &str) -> Option<Digest> {
    // The `[exit N]` line the shell tool appends is not the runner's.
    let lines: Vec<&str> = output
        .lines()
        .filter(|l| !l.trim_start().starts_with("[exit "))
        .collect();
    [cargo, go, pytest, unittest, jest, vitest, mocha]
        .iter()
        .find_map(|parse| parse(&lines))
}

/// What a test command's output comes to, for the model. A run that
/// passed: the runner's totals, or its last `tail` lines when no runner is
/// recognised. A run that failed: its failures, each with what it said;
/// failing that, the compiler's errors when that is what it printed;
/// failing that, its last `tail` lines.
pub fn report(output: &str, ok: bool, tail: usize) -> String {
    let out = output.trim_end();
    let digest = digest(out);
    if ok {
        return match digest {
            Some(d) if !d.totals.is_empty() => d.totals.join("\n"),
            _ => last_lines(out, tail),
        };
    }
    if let Some(d) = digest.filter(|d| !d.failures.is_empty()) {
        return d.text();
    }
    if !crate::check::blocks(out).is_empty() {
        return crate::check::report(out, false).1;
    }
    last_lines(out, tail)
}

/// `detail` with the indentation they share removed, blank lines dropped,
/// and no more than a failure may run to.
fn tidy(detail: Vec<String>) -> Vec<String> {
    let kept: Vec<String> = detail
        .into_iter()
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.trim().is_empty())
        .collect();
    let indent = kept
        .iter()
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    let mut out: Vec<String> = kept
        .iter()
        .map(|l| l.get(indent..).unwrap_or(l.trim_start()).to_string())
        .collect();
    if out.len() > DETAIL_LINES {
        let more = out.len() - (DETAIL_LINES - 1);
        out.truncate(DETAIL_LINES - 1);
        out.push(format!("… {more} more lines"));
    }
    out
}

fn found(runner: &'static str, failures: Vec<Failure>, totals: Vec<String>) -> Option<Digest> {
    let totals = kept_totals(totals);
    (!failures.is_empty() || !totals.is_empty()).then_some(Digest {
        runner,
        failures,
        totals,
    })
}

/// The totals a report carries when a run printed more than fit: the ones
/// that say a failure first, then the last ones, in the runner's order. The
/// row reads the last line, so a `FAILED` past the cap must not be dropped
/// for an earlier `ok`.
fn kept_totals(totals: Vec<String>) -> Vec<String> {
    if totals.len() <= MAX_TOTALS {
        return totals;
    }
    // `test result: FAILED`, go's `FAIL\tpkg`, `2 failed`, `1 error`; not
    // cargo's `0 failed` on a passing binary.
    let says_failed = |t: &str| {
        let low = t.to_ascii_lowercase();
        let count_before = |word: &str| {
            low.find(word).is_some_and(|i| {
                let digits: String = low[..i]
                    .chars()
                    .rev()
                    .take_while(char::is_ascii_digit)
                    .collect();
                !digits.is_empty() && digits.chars().any(|d| d != '0')
            })
        };
        t.contains("FAILED")
            || t.starts_with("FAIL")
            || count_before(" failed")
            || count_before(" error")
            || count_before(" failing")
    };
    let mut keep: Vec<usize> = totals
        .iter()
        .enumerate()
        .filter(|(_, t)| says_failed(t))
        .map(|(i, _)| i)
        .take(MAX_TOTALS)
        .collect();
    for i in (0..totals.len()).rev() {
        if keep.len() >= MAX_TOTALS {
            break;
        }
        if !keep.contains(&i) {
            keep.push(i);
        }
    }
    keep.sort_unstable();
    keep.into_iter().map(|i| totals[i].clone()).collect()
}

/// Whether a line is a code frame's (`  3 |   foo();`, `    |      ^`,
/// `  4| foo();`), which the assertion lines say better.
fn code_frame(t: &str) -> bool {
    let t = t.trim_start();
    if t.starts_with('|') {
        return true;
    }
    let digits: String = t.chars().take_while(char::is_ascii_digit).collect();
    !digits.is_empty() && t[digits.len()..].trim_start().starts_with('|')
}

/// cargo test: `---- name stdout ----` blocks, each from its `panicked at`
/// line to the blank line before the next, and `test result:` totals.
fn cargo(lines: &[&str]) -> Option<Digest> {
    let header = |t: &str| {
        t.strip_prefix("---- ")
            .and_then(|r| r.strip_suffix(" stdout ----"))
            .map(str::to_string)
    };
    // The closing `failures:` list names the tests that failed. Under
    // `--show-output` a passing test gets a `---- name stdout ----` block
    // too, so a block is a failure only when the list names it; with no
    // list (the output cut short), a block with a panic in it.
    let listed: Vec<String> = lines
        .iter()
        .rposition(|l| l.trim() == "failures:")
        .map(|at| {
            lines[at + 1..]
                .iter()
                .take_while(|l| l.starts_with(' ') || l.starts_with('\t'))
                .map(|l| l.trim().to_string())
                .collect()
        })
        .unwrap_or_default();
    let mut failures = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some(name) = header(lines[i].trim()) else {
            i += 1;
            continue;
        };
        let mut detail = Vec::new();
        i += 1;
        while i < lines.len() {
            let t = lines[i].trim();
            // The next block, the closing list, the totals, or a test's
            // own line (`test x ... ok`, printed between blocks under
            // `--nocapture`) ends this one.
            if header(t).is_some()
                || t == "failures:"
                || t.starts_with("test result:")
                || (t.starts_with("test ") && t.contains(" ... "))
            {
                break;
            }
            if !t.starts_with("note: run with `RUST_BACKTRACE") {
                detail.push(lines[i].to_string());
            }
            i += 1;
        }
        let panicked = detail.iter().position(|l| l.contains("panicked at"));
        let failed = if listed.is_empty() {
            panicked.is_some()
        } else {
            listed.contains(&name)
        };
        if !failed {
            continue;
        }
        // What the test printed before it panicked is its own; the panic
        // and the assertion under it are the failure.
        if let Some(p) = panicked {
            detail.drain(..p);
        }
        failures.push(Failure {
            name,
            detail: tidy(detail),
        });
    }
    let totals = lines
        .iter()
        .map(|l| l.trim())
        .filter(|t| t.starts_with("test result:"))
        .map(str::to_string)
        .collect();
    found("cargo test", failures, totals)
}

/// go test: `--- FAIL: Name (0.00s)` with its indented lines, subtests
/// nested one indent in, and the `FAIL`/`ok` package lines.
fn go(lines: &[&str]) -> Option<Digest> {
    let mut failures: Vec<Failure> = Vec::new();
    let mut totals = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        let t = l.trim_start();
        if let Some(rest) = t.strip_prefix("--- FAIL: ") {
            let name = rest.rsplit_once(" (").map_or(rest, |(n, _)| n).to_string();
            let indent = l.len() - t.len();
            let mut detail = Vec::new();
            i += 1;
            while i < lines.len() {
                let d = lines[i];
                let dt = d.trim_start();
                let di = d.len() - dt.len();
                if dt.trim().is_empty()
                    || di <= indent
                    || dt.starts_with("--- ")
                    || dt.starts_with("=== ")
                {
                    break;
                }
                detail.push(d.to_string());
                i += 1;
            }
            failures.push(Failure {
                name,
                detail: tidy(detail),
            });
            continue;
        }
        let tt = t.trim_end();
        if tt == "FAIL"
            || tt.starts_with("FAIL\t")
            || tt.starts_with("ok\t")
            || tt.starts_with("ok  \t")
        {
            totals.push(tt.to_string());
        }
        i += 1;
    }
    // A parent whose subtests failed says nothing of its own.
    let names: Vec<String> = failures.iter().map(|f| f.name.clone()).collect();
    failures.retain(|f| {
        !f.detail.is_empty() || !names.iter().any(|n| n.starts_with(&format!("{}/", f.name)))
    });
    found("go test", failures, totals)
}

/// pytest: `____ name ____` blocks in the FAILURES section, each read for
/// its `>` statement, `E` assertion lines and `path:line: Error` location,
/// and the `=== 1 failed, 2 passed in 0.1s ===` totals.
fn pytest(lines: &[&str]) -> Option<Digest> {
    let header = |t: &str| {
        (t.len() > 6 && t.starts_with("___") && t.ends_with("___"))
            .then(|| t.trim_matches('_').trim().to_string())
            .filter(|n| !n.is_empty())
    };
    let totals_line = |t: &str| {
        let c = t.trim_matches(|c: char| c == '=' || c.is_whitespace());
        let counted = c
            .split(' ')
            .next()
            .is_some_and(|n| !n.is_empty() && n.chars().all(|ch| ch.is_ascii_digit()));
        (counted
            && c.contains(" in ")
            && c.ends_with('s')
            && ["passed", "failed", "error", "skipped", "xfailed", "xpassed"]
                .iter()
                .any(|w| c.contains(w)))
        .then(|| c.to_string())
    };
    let mut failures = Vec::new();
    let mut totals = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        if let Some(total) = totals_line(t) {
            totals.push(total);
            i += 1;
            continue;
        }
        let Some(name) = header(t) else {
            i += 1;
            continue;
        };
        let mut block = Vec::new();
        i += 1;
        while i < lines.len() {
            let t = lines[i].trim();
            if header(t).is_some() || t.starts_with("===") || totals_line(t).is_some() {
                break;
            }
            block.push(lines[i]);
            i += 1;
        }
        let telling: Vec<String> = block
            .iter()
            .filter(|l| {
                let t = l.trim_start();
                t.starts_with('>')
                    || t == "E"
                    || t.starts_with("E ")
                    || t.starts_with("E\t")
                    || py_location(t)
            })
            .map(|l| l.to_string())
            .collect();
        let detail = if telling.is_empty() {
            let kept: Vec<&&str> = block.iter().filter(|l| !l.trim().is_empty()).collect();
            kept[kept.len().saturating_sub(DETAIL_LINES)..]
                .iter()
                .map(|l| l.to_string())
                .collect()
        } else {
            telling
        };
        failures.push(Failure {
            name,
            detail: tidy(detail),
        });
    }
    found("pytest", failures, totals)
}

/// `path.py:12: AssertionError`, pytest's way of saying where.
fn py_location(t: &str) -> bool {
    let Some((path, rest)) = t.split_once(".py:") else {
        return false;
    };
    if path.contains(' ') {
        return false;
    }
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    !digits.is_empty() && rest[digits.len()..].starts_with(": ")
}

/// unittest: `FAIL: name` or `ERROR: name` under a `====` rule, its
/// traceback read from its last `File` line, and `Ran N tests` with the
/// `FAILED (…)` or `OK` after it.
fn unittest(lines: &[&str]) -> Option<Digest> {
    let rule = |t: &str| t.len() >= 10 && (t.starts_with("=====") || t.starts_with("-----"));
    let mut failures = Vec::new();
    let mut totals = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        if t.starts_with("Ran ") && t.contains(" test") {
            totals.push(t.to_string());
            if let Some(next) = lines[i + 1..]
                .iter()
                .map(|l| l.trim())
                .find(|l| !l.is_empty())
            {
                if next.starts_with("FAILED") || next == "OK" || next.starts_with("OK (") {
                    totals.push(next.to_string());
                }
            }
            i += 1;
            continue;
        }
        let under_rule = i > 0 && lines[i - 1].trim().starts_with("=====");
        let name = t
            .strip_prefix("FAIL: ")
            .or_else(|| t.strip_prefix("ERROR: "))
            .filter(|_| under_rule);
        let Some(name) = name else {
            i += 1;
            continue;
        };
        let mut block = Vec::new();
        i += 1;
        while i < lines.len() {
            let t = lines[i].trim();
            if rule(t) && !block.is_empty() {
                break;
            }
            if !rule(t) {
                block.push(lines[i]);
            }
            i += 1;
        }
        let from = block
            .iter()
            .rposition(|l| l.trim_start().starts_with("File \""))
            .unwrap_or_else(|| block.len().saturating_sub(DETAIL_LINES));
        failures.push(Failure {
            name: name.to_string(),
            detail: tidy(block[from..].iter().map(|l| l.to_string()).collect()),
        });
    }
    found("unittest", failures, totals)
}

/// jest: `● suite › test` blocks, read without their code frames and with
/// their first `at` line, repeats in the closing summary dropped, and the
/// `Tests:` totals.
fn jest(lines: &[&str]) -> Option<Digest> {
    let mut failures: Vec<Failure> = Vec::new();
    let mut totals = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        if t.starts_with("Tests:") {
            totals.push(t.to_string());
            i += 1;
            continue;
        }
        let Some(name) = t.strip_prefix("● ") else {
            i += 1;
            continue;
        };
        let name = name.trim().to_string();
        let mut detail = Vec::new();
        let mut located = false;
        i += 1;
        while i < lines.len() {
            let t = lines[i].trim();
            if t.starts_with("● ")
                || t.starts_with("Tests:")
                || t.starts_with("Test Suites:")
                || t.starts_with("PASS ")
                || t.starts_with("FAIL ")
                || t == "Summary of all failing tests"
            {
                break;
            }
            if t.starts_with("at ") {
                if !located {
                    detail.push(lines[i].to_string());
                    located = true;
                }
            } else if !code_frame(t) || t.starts_with('>') {
                detail.push(lines[i].to_string());
            }
            i += 1;
        }
        if !failures.iter().any(|f| f.name == name) {
            failures.push(Failure {
                name,
                detail: tidy(detail),
            });
        }
    }
    found("jest", failures, totals)
}

/// vitest: ` FAIL  file > suite > test` blocks up to the next `⎯` rule,
/// read without their code frames, and the `Tests  1 failed | 2 passed`
/// totals.
fn vitest(lines: &[&str]) -> Option<Digest> {
    let mut failures = Vec::new();
    let mut totals = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        if t.starts_with("Tests ") && (t.contains("passed") || t.contains("failed")) {
            totals.push(t.to_string());
            i += 1;
            continue;
        }
        let name = t
            .strip_prefix("FAIL ")
            .map(str::trim)
            .filter(|n| n.contains(" > "));
        let Some(name) = name else {
            i += 1;
            continue;
        };
        let mut detail = Vec::new();
        i += 1;
        while i < lines.len() {
            let t = lines[i].trim();
            if t.starts_with('⎯') || (t.starts_with("FAIL ") && t.contains(" > ")) {
                break;
            }
            if !code_frame(t) {
                detail.push(lines[i].to_string());
            }
            i += 1;
        }
        failures.push(Failure {
            name: name.to_string(),
            detail: tidy(detail),
        });
    }
    found("vitest", failures, totals)
}

/// mocha: after its `N failing` line, `1) suite` blocks whose next line
/// names the test, read with their first `at` line, and the `passing` and
/// `failing` totals.
fn mocha(lines: &[&str]) -> Option<Digest> {
    fn numbered(t: &str) -> Option<&str> {
        let digits: String = t.chars().take_while(char::is_ascii_digit).collect();
        (!digits.is_empty())
            .then(|| t[digits.len()..].strip_prefix(") "))
            .flatten()
    }
    let total = |t: &str| {
        let (n, word) = t.split_once(' ').unwrap_or(("", ""));
        !n.is_empty()
            && n.chars().all(|c| c.is_ascii_digit())
            && (word.starts_with("passing")
                || word.starts_with("failing")
                || word.starts_with("pending"))
    };
    let mut failures = Vec::new();
    let mut totals = Vec::new();
    let mut failing_seen = false;
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        if total(t) {
            totals.push(t.to_string());
            failing_seen |= t.contains("failing");
            i += 1;
            continue;
        }
        let Some(suite) = numbered(t).filter(|_| failing_seen) else {
            i += 1;
            continue;
        };
        let mut name = suite.to_string();
        i += 1;
        if let Some(test) = lines.get(i).map(|l| l.trim()).filter(|l| !l.is_empty()) {
            name = format!("{suite} › {}", test.trim_end_matches(':'));
            i += 1;
        }
        let mut detail = Vec::new();
        let mut located = false;
        while i < lines.len() {
            let t = lines[i].trim();
            if numbered(t).is_some() {
                break;
            }
            if t.starts_with("at ") {
                if !located {
                    detail.push(lines[i].to_string());
                    located = true;
                }
            } else {
                detail.push(lines[i].to_string());
            }
            i += 1;
        }
        failures.push(Failure {
            name,
            detail: tidy(detail),
        });
    }
    found("mocha", failures, totals)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARGO: &str = "   Compiling app v0.1.0\n    Finished `test` profile\n     Running unittests src/lib.rs\n\n\
running 3 tests\ntest tests::b ... ok\ntest tests::a ... FAILED\ntest tests::c ... FAILED\n\n\
failures:\n\n\
---- tests::a stdout ----\nsetting up\nthread 'tests::a' panicked at src/lib.rs:10:9:\nassertion `left == right` failed\n  left: 1\n right: 2\nnote: run with `RUST_BACKTRACE=1` environment variable to display a backtrace\n\n\
---- tests::c stdout ----\nthread 'tests::c' panicked at src/lib.rs:20:5:\nnot yet\n\n\n\
failures:\n    tests::a\n    tests::c\n\n\
test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n\n[exit 101]";

    #[test]
    fn cargo_failures_carry_the_panic_and_the_assertion_not_the_tests_prints() {
        let d = digest(CARGO).unwrap();
        assert_eq!(d.runner, "cargo test");
        assert_eq!(d.failures.len(), 2);
        assert_eq!(d.failures[0].name, "tests::a");
        assert_eq!(
            d.failures[0].detail,
            vec![
                "thread 'tests::a' panicked at src/lib.rs:10:9:",
                "assertion `left == right` failed",
                "  left: 1",
                " right: 2",
            ]
        );
        assert_eq!(
            d.failures[1].detail,
            vec!["thread 'tests::c' panicked at src/lib.rs:20:5:", "not yet"]
        );
        assert_eq!(
            d.totals,
            vec![
                "test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s"
            ]
        );
        let text = d.text();
        assert!(
            text.starts_with("2 failed\n\n✗ tests::a\nthread 'tests::a' panicked"),
            "{text}"
        );
        assert!(text.ends_with("finished in 0.00s"), "{text}");
        assert!(
            !text.contains("setting up") && !text.contains("RUST_BACKTRACE"),
            "{text}"
        );
    }

    #[test]
    fn a_passing_cargo_run_is_its_totals_and_an_unrecognised_one_its_tail() {
        let ok = "running 2 tests\ntest a ... ok\ntest b ... ok\n\ntest result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n\n     Running tests/cli.rs\n\nrunning 1 test\ntest c ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n";
        assert_eq!(
            report(ok, true, 60),
            "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n\
             test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s"
        );
        let plain: String = (1..=70).map(|i| format!("line {i}\n")).collect();
        let tail = report(&plain, true, 60);
        assert!(
            tail.starts_with("line 11\n") && tail.ends_with("line 70"),
            "{tail}"
        );
        assert_eq!(
            report(
                "make: *** No rule to make target 'test'.  Stop.\n[exit 2]",
                false,
                60
            ),
            "make: *** No rule to make target 'test'.  Stop.\n[exit 2]"
        );
    }

    #[test]
    fn a_test_run_that_did_not_compile_reports_the_compiler() {
        let out = "   Compiling app v0.1.0\nerror[E0308]: mismatched types\n --> src/lib.rs:4:9\n  |\n4 |     let x: u32 = \"a\";\n\nerror: could not compile `app` (lib test) due to 1 previous error\n[exit 101]";
        let text = report(out, false, 60);
        assert!(text.starts_with("1 error\n\nerror[E0308]"), "{text}");
    }

    const GO: &str = "=== RUN   TestAdd\n--- FAIL: TestAdd (0.00s)\n    add_test.go:12: add(1, 2) = 3, want 4\n    add_test.go:13: and more\n=== RUN   TestTable\n=== RUN   TestTable/zero\n=== RUN   TestTable/neg\n--- FAIL: TestTable (0.00s)\n    --- PASS: TestTable/zero (0.00s)\n    --- FAIL: TestTable/neg (0.00s)\n        add_test.go:30: neg: got 1\nFAIL\nFAIL\texample.com/m/add\t0.004s\nok  \texample.com/m/util\t0.002s\n[exit 1]";

    #[test]
    fn go_failures_are_the_tests_with_their_lines_and_a_parent_of_subtests_is_not_one() {
        let d = digest(GO).unwrap();
        assert_eq!(d.runner, "go test");
        let names: Vec<&str> = d.failures.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["TestAdd", "TestTable/neg"]);
        assert_eq!(
            d.failures[0].detail,
            vec![
                "add_test.go:12: add(1, 2) = 3, want 4",
                "add_test.go:13: and more"
            ]
        );
        assert_eq!(d.failures[1].detail, vec!["add_test.go:30: neg: got 1"]);
        assert_eq!(
            d.totals,
            vec![
                "FAIL",
                "FAIL\texample.com/m/add\t0.004s",
                "ok  \texample.com/m/util\t0.002s"
            ]
        );
    }

    const PYTEST: &str = "============================= test session starts ==============================\ncollected 3 items\n\ntests/test_x.py F.F                                                      [100%]\n\n=================================== FAILURES ===================================\n__________________________________ test_add ___________________________________\n\n    def test_add():\n>       assert add(1, 2) == 4\nE       assert 3 == 4\nE        +  where 3 = add(1, 2)\n\ntests/test_x.py:5: AssertionError\n_______________________________ test_raises ________________________________\n\n    def test_raises():\n>       boom()\n\ntests/test_x.py:9: \n_ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _\n\n    def boom():\n>       raise ValueError(\"no\")\nE       ValueError: no\n\nsrc/x.py:3: ValueError\n=========================== short test summary info ============================\nFAILED tests/test_x.py::test_add - assert 3 == 4\nFAILED tests/test_x.py::test_raises - ValueError: no\n========================= 2 failed, 1 passed in 0.03s ==========================\n[exit 1]";

    #[test]
    fn pytest_failures_are_the_statement_the_assertion_and_where() {
        let d = digest(PYTEST).unwrap();
        assert_eq!(d.runner, "pytest");
        assert_eq!(d.failures.len(), 2, "{:?}", d.failures);
        assert_eq!(d.failures[0].name, "test_add");
        assert_eq!(
            d.failures[0].detail,
            vec![
                ">       assert add(1, 2) == 4",
                "E       assert 3 == 4",
                "E        +  where 3 = add(1, 2)",
                "tests/test_x.py:5: AssertionError"
            ]
        );
        assert_eq!(d.failures[1].name, "test_raises");
        assert!(
            d.failures[1]
                .detail
                .contains(&"E       ValueError: no".to_string()),
            "{:?}",
            d.failures[1].detail
        );
        assert!(
            d.failures[1]
                .detail
                .contains(&"src/x.py:3: ValueError".to_string())
        );
        assert_eq!(d.totals, vec!["2 failed, 1 passed in 0.03s"]);
        // `pytest -q` prints the totals bare.
        let quiet =
            "F.\n=== FAILURES ===\n___ test_a ___\nE   assert 1 == 2\n1 failed, 1 passed in 0.01s";
        assert_eq!(
            digest(quiet).unwrap().totals,
            vec!["1 failed, 1 passed in 0.01s"]
        );
        assert_eq!(
            report("..\n2 passed in 0.01s\n", true, 60),
            "2 passed in 0.01s"
        );
    }

    const UNITTEST: &str = "F.\n======================================================================\nFAIL: test_add (test_x.TestX.test_add)\n----------------------------------------------------------------------\nTraceback (most recent call last):\n  File \"/p/test_x.py\", line 5, in test_add\n    self.assertEqual(add(1, 2), 4)\nAssertionError: 3 != 4\n\n----------------------------------------------------------------------\nRan 2 tests in 0.001s\n\nFAILED (failures=1)\n[exit 1]";

    #[test]
    fn unittest_failures_are_read_from_their_last_file_line() {
        let d = digest(UNITTEST).unwrap();
        assert_eq!(d.runner, "unittest");
        assert_eq!(d.failures.len(), 1);
        assert_eq!(d.failures[0].name, "test_add (test_x.TestX.test_add)");
        assert_eq!(
            d.failures[0].detail,
            vec![
                "  File \"/p/test_x.py\", line 5, in test_add",
                "    self.assertEqual(add(1, 2), 4)",
                "AssertionError: 3 != 4"
            ]
        );
        assert_eq!(
            d.totals,
            vec!["Ran 2 tests in 0.001s", "FAILED (failures=1)"]
        );
        assert_eq!(
            report("..\n----\nRan 2 tests in 0.001s\n\nOK\n", true, 60),
            "Ran 2 tests in 0.001s\nOK"
        );
    }

    const JEST: &str = " FAIL  src/x.test.ts\n  adds\n    ✕ returns the sum (3 ms)\n    ✓ handles zero (1 ms)\n\n  ● adds › returns the sum\n\n    expect(received).toBe(expected) // Object.is equality\n\n    Expected: 4\n    Received: 3\n\n      3 |   it('returns the sum', () => {\n    > 4 |     expect(add(1, 2)).toBe(4);\n        |                        ^\n      5 |   });\n\n      at Object.<anonymous> (src/x.test.ts:4:24)\n      at Promise.then.completed (node_modules/jest-circus/build/utils.js:298:28)\n\nSummary of all failing tests\n FAIL  src/x.test.ts\n  ● adds › returns the sum\n\n    expect(received).toBe(expected) // Object.is equality\n\nTest Suites: 1 failed, 1 total\nTests:       1 failed, 1 passed, 2 total\nSnapshots:   0 total\nTime:        0.5 s\n[exit 1]";

    #[test]
    fn jest_failures_keep_expected_received_and_one_location_and_are_not_repeated() {
        let d = digest(JEST).unwrap();
        assert_eq!(d.runner, "jest");
        assert_eq!(d.failures.len(), 1);
        assert_eq!(d.failures[0].name, "adds › returns the sum");
        assert_eq!(
            d.failures[0].detail,
            vec![
                "expect(received).toBe(expected) // Object.is equality",
                "Expected: 4",
                "Received: 3",
                "> 4 |     expect(add(1, 2)).toBe(4);",
                "  at Object.<anonymous> (src/x.test.ts:4:24)",
            ]
        );
        assert_eq!(d.totals, vec!["Tests:       1 failed, 1 passed, 2 total"]);
    }

    const VITEST: &str = " ❯ src/x.test.ts (2 tests | 1 failed) 5ms\n   × adds > returns the sum 2ms\n     → expected 3 to be 4 // Object.is equality\n\n⎯⎯⎯⎯⎯⎯⎯ Failed Tests 1 ⎯⎯⎯⎯⎯⎯⎯\n\n FAIL  src/x.test.ts > adds > returns the sum\nAssertionError: expected 3 to be 4 // Object.is equality\n\n- Expected\n+ Received\n\n- 4\n+ 3\n\n ❯ src/x.test.ts:4:24\n      2| describe('adds', () => {\n      3|   it('returns the sum', () => {\n      4|     expect(add(1, 2)).toBe(4);\n       |                        ^\n      5|   });\n\n⎯⎯⎯⎯⎯⎯⎯[1/1]⎯⎯⎯⎯⎯⎯⎯\n\n Test Files  1 failed (1)\n      Tests  1 failed | 1 passed (2)\n   Start at  10:00:00\n   Duration  300ms\n[exit 1]";

    #[test]
    fn vitest_failures_are_the_assertion_the_diff_and_where() {
        let d = digest(VITEST).unwrap();
        assert_eq!(d.runner, "vitest");
        assert_eq!(d.failures.len(), 1);
        assert_eq!(d.failures[0].name, "src/x.test.ts > adds > returns the sum");
        assert_eq!(
            d.failures[0].detail,
            vec![
                "AssertionError: expected 3 to be 4 // Object.is equality",
                "- Expected",
                "+ Received",
                "- 4",
                "+ 3",
                " ❯ src/x.test.ts:4:24"
            ]
        );
        assert_eq!(d.totals, vec!["Tests  1 failed | 1 passed (2)"]);
    }

    const MOCHA: &str = "\n  adds\n    ✓ handles zero\n    1) returns the sum\n\n\n  1 passing (10ms)\n  1 failing\n\n  1) adds\n       returns the sum:\n\n      AssertionError: expected 3 to equal 4\n      + expected - actual\n\n      -3\n      +4\n      \n      at Context.<anonymous> (test/x.js:5:28)\n      at process.processImmediate (node:internal/timers:483:21)\n\n[exit 1]";

    #[test]
    fn mocha_failures_are_named_by_suite_and_test_after_the_failing_line() {
        let d = digest(MOCHA).unwrap();
        assert_eq!(d.runner, "mocha");
        assert_eq!(d.failures.len(), 1);
        assert_eq!(d.failures[0].name, "adds › returns the sum");
        assert_eq!(
            d.failures[0].detail,
            vec![
                "AssertionError: expected 3 to equal 4",
                "+ expected - actual",
                "-3",
                "+4",
                "at Context.<anonymous> (test/x.js:5:28)"
            ]
        );
        assert_eq!(d.totals, vec!["1 passing (10ms)", "1 failing"]);
    }

    /// `cargo test -- --show-output` prints a block for every test, passing
    /// ones included: only those the closing list names are failures. More
    /// totals than fit keep the failing ones and the last ones.
    #[test]
    fn show_output_blocks_of_passing_tests_are_not_failures_and_failed_totals_are_kept() {
        let out = "running 2 tests\ntest passes ... ok\ntest fails ... FAILED\n\nsuccesses:\n\n---- passes stdout ----\nhello from pass\n\n\nsuccesses:\n    passes\n\nfailures:\n\n---- fails stdout ----\nthread 'fails' panicked at src/lib.rs:9:5:\nassertion failed: false\n\n\nfailures:\n    fails\n\ntest result: FAILED. 1 passed; 1 failed; 0 ignored\n";
        let d = digest(out).unwrap();
        assert_eq!(d.failures.len(), 1, "{:?}", d.failures);
        assert_eq!(d.failures[0].name, "fails");
        assert_eq!(
            d.failures[0].detail[0],
            "thread 'fails' panicked at src/lib.rs:9:5:"
        );
        // A block without the list and without a panic is a print, not a failure.
        let cut = "---- passes stdout ----\nhello\n\n---- fails stdout ----\nthread 'fails' panicked at src/lib.rs:9:5:\nno\n";
        let d = digest(cut).unwrap();
        assert_eq!(
            d.failures
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>(),
            ["fails"]
        );
        // Ten binaries, the third failed: its totals line is kept, and last
        // lines fill the rest, in order.
        let mut many = String::new();
        for i in 0..10 {
            many.push_str(&format!(
                "test result: {}. 1 passed; {} failed; 0 ignored\n",
                if i == 2 { "FAILED" } else { "ok" },
                if i == 2 { 1 } else { 0 }
            ));
        }
        let d = digest(&many).unwrap();
        assert_eq!(d.totals.len(), MAX_TOTALS);
        assert_eq!(
            d.totals[0],
            "test result: FAILED. 1 passed; 1 failed; 0 ignored"
        );
        assert!(
            d.totals[1..]
                .iter()
                .all(|t| t.starts_with("test result: ok."))
        );
        let kept = kept_totals(vec!["ok  \ta".into(), "FAIL\tb".into()]);
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn the_report_is_capped_in_failures_lines_and_size() {
        let many: String = (0..30)
            .map(|i| {
                format!("---- t{i} stdout ----\nthread 't{i}' panicked at src/l.rs:{i}:1:\nno\n\n")
            })
            .collect();
        let d = digest(&many).unwrap();
        assert_eq!(d.failures.len(), 30);
        let text = d.text();
        assert!(text.starts_with("30 failed\n"), "{text}");
        assert!(text.ends_with("… and 20 more"), "{text}");
        let long = format!(
            "---- t stdout ----\nthread 't' panicked at src/l.rs:1:1:\n{}",
            (0..40).map(|i| format!("line {i}\n")).collect::<String>()
        );
        let d = digest(&long).unwrap();
        assert_eq!(d.failures[0].detail.len(), DETAIL_LINES);
        assert_eq!(d.failures[0].detail.last().unwrap(), "… 30 more lines");
        let wide: String = (0..6)
            .map(|i| {
                format!(
                    "---- w{i} stdout ----\nthread 'w{i}' panicked at src/l.rs:1:1:\n{}\n\n",
                    "x".repeat(900)
                )
            })
            .collect();
        let text = digest(&wide).unwrap().text();
        assert!(text.len() < MAX_CHARS + 1_000, "{}", text.len());
        assert!(text.contains("… and"), "{text}");
    }
}
