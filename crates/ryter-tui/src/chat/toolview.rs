//! How a tool step reads in the chat: a verb people use ("edit", "run"),
//! what it touched, and what came of it ("new · 48 lines", "✓ 12 passed",
//! "✗ exit 1" with the last lines of output). Everything here is measured by
//! Ryter from the call and its result, so it is accurate whatever the model
//! says about its own work.

use serde_json::Value;

/// Lookups fold into one line; the chat is for the work.
pub fn is_lookup(tool: &str) -> bool {
    matches!(tool, "read_file" | "grep" | "glob" | "list_dir")
}

/// The verb shown for a tool.
pub fn verb(tool: &str) -> &str {
    match tool {
        "write" => "write",
        "search_replace" => "edit",
        "bash" => "run",
        "read_file" => "read",
        "grep" => "search",
        "glob" => "find",
        "list_dir" => "list",
        "request_hat" => "hat",
        "ask_user" => "ask",
        "todo_write" => "tasks",
        "web_fetch" => "fetch",
        "web_search" => "web",
        other => other,
    }
}

fn arg<'a>(args: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| args.get(*k).and_then(Value::as_str))
}

/// What the step touched: a path, a command, a pattern.
pub fn target(tool: &str, args: &Value) -> String {
    let t = match tool {
        "bash" => arg(args, &["command"]),
        "grep" | "glob" => arg(args, &["pattern"]),
        "request_hat" => arg(args, &["hat"]),
        "ask_user" => arg(args, &["question"]),
        "web_fetch" => arg(args, &["url"]),
        "web_search" => arg(args, &["query"]),
        _ => arg(args, &["path", "target_file"]),
    };
    t.unwrap_or("")
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Lines shown under an edit before it runs: what goes, what comes.
/// `- ` and `+ ` lead each line; a few of each, then `…`.
pub fn edit_preview(tool: &str, args: &Value) -> String {
    if tool != "search_replace" {
        return String::new();
    }
    const EACH: usize = 3;
    // Only what changes: unchanged lines the model quoted around the edit
    // for context aren't shown as removed and added again.
    let (gone, come) = ryter_core::tools::changed_lines(
        arg(args, &["old_string"]).unwrap_or(""),
        arg(args, &["new_string"]).unwrap_or(""),
    );
    let side = |lines: &[&str], mark: &str| -> Vec<String> {
        let lines: Vec<&str> = lines
            .iter()
            .copied()
            .filter(|l| !l.trim().is_empty())
            .collect();
        let mut out: Vec<String> = lines
            .iter()
            .take(EACH)
            .map(|l| format!("{mark} {}", l.trim_end()))
            .collect();
        if lines.len() > EACH {
            out.push(format!("{mark} … {} more", lines.len() - EACH));
        }
        out
    };
    let mut rows = side(&gone, "-");
    rows.extend(side(&come, "+"));
    rows.join("\n")
}

/// Test-runner summaries worth lifting out of a command's output.
fn test_summary(output: &str) -> Option<String> {
    // node:test prints TAP totals: `# pass 12` / `# fail 0`.
    let tap = |key: &str| {
        output.lines().rev().find_map(|l| {
            l.trim()
                .strip_prefix(key)
                .and_then(|n| n.trim().parse::<u64>().ok())
        })
    };
    if let Some(pass) = tap("# pass ") {
        return Some(match tap("# fail ").filter(|f| *f > 0) {
            Some(fail) => format!("{pass} passed, {fail} failed"),
            None => format!("{pass} passed"),
        });
    }
    let found = output.lines().rev().map(str::trim).find(|l| {
        let low = l.to_ascii_lowercase();
        (low.contains(" passed")
            || low.contains(" failed")
            || low.contains(" passing")
            || low.contains(" failing"))
            && low.chars().any(|c| c.is_ascii_digit())
            || low.starts_with("ran ") && low.contains(" test")
            || low.starts_with("tests:") && low.contains("passed")
            || low.starts_with("test result:")
    })?;
    let clean: String = found
        .trim_matches(|c: char| c == '=' || c == '-' || c.is_whitespace())
        .chars()
        .take(60)
        .collect();
    Some(clean)
}

fn last_lines(output: &str, n: usize, skip: impl Fn(&str) -> bool) -> Vec<String> {
    let lines: Vec<&str> = output
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty() && !skip(l))
        .collect();
    lines[lines.len().saturating_sub(n)..]
        .iter()
        .map(|l| l.chars().take(140).collect())
        .collect()
}

/// The lines of a failure worth reading: the first one that reads like an
/// error (usually the cause), then the last few. The tail alone is often
/// boilerplate: npm ends every failure with where its log went, after the
/// line that said `Missing script: "lint"`.
fn failure_lines(output: &str) -> String {
    const MARKERS: &[&str] = &[
        "error",
        "failed",
        "fail:",
        "missing",
        "not found",
        "cannot",
        "can't",
        "no such",
        "denied",
        "refused",
        "panicked",
        "traceback",
        "exception",
        "undefined",
    ];
    let lines: Vec<&str> = output
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty() && !l.trim().starts_with("[exit "))
        .collect();
    let cause = lines.iter().position(|l| {
        let low = l.to_ascii_lowercase();
        MARKERS.iter().any(|m| low.contains(m))
            // `npm error` alone, with nothing after it, says nothing.
            && low.split_whitespace().count() > 2
    });
    let tail_from = lines.len().saturating_sub(3);
    let mut picked: Vec<usize> = cause.into_iter().filter(|c| *c < tail_from).collect();
    picked.extend(tail_from..lines.len());
    let mut out = Vec::new();
    for (k, i) in picked.iter().enumerate() {
        if k > 0 && *i > picked[k - 1] + 1 {
            out.push("! …".to_string());
        }
        out.push(format!(
            "! {}",
            lines[*i].chars().take(140).collect::<String>()
        ));
    }
    out.join("\n")
}

/// What came of a step: a short detail for the row's right side, and lines
/// for under it. Error lines lead with `! `, plain ones with two spaces.
pub fn result(tool: &str, output: &str, is_error: bool) -> (String, String) {
    let first = output.lines().next().unwrap_or("");
    if is_error {
        // A failing test run leads with its totals; anything else with how
        // it exited.
        let exit = test_summary(output)
            .map(|s| format!("✗ {s}"))
            .or_else(|| {
                output
                    .lines()
                    .rev()
                    .find_map(|l| {
                        l.trim()
                            .strip_prefix("[exit ")
                            .and_then(|r| r.strip_suffix(']'))
                    })
                    .map(|c| format!("✗ exit {c}"))
            })
            .unwrap_or_else(|| "✗".into());
        return (exit, failure_lines(output));
    }
    match tool {
        "write" => {
            let detail = if let Some(rest) = first.strip_prefix("created ") {
                format!("new · {}", rest.rsplit(" · ").next().unwrap_or(""))
            } else if let Some(rest) = first.strip_prefix("rewrote ") {
                format!("rewrote · {}", rest.rsplit(" · ").next().unwrap_or(""))
            } else {
                String::new()
            };
            (detail, String::new())
        }
        "search_replace" => (
            first
                .rsplit(" · ")
                .next()
                .filter(|d| d.starts_with('−'))
                .map(|d| d.trim_end_matches(" lines").to_string())
                .unwrap_or_default(),
            String::new(),
        ),
        "bash" => match test_summary(output) {
            // The test totals say it; the raw tail (`# duration_ms …`) doesn't.
            Some(s) => (format!("✓ {s}"), String::new()),
            None => (
                "✓".into(),
                last_lines(output, 2, |_| false)
                    .into_iter()
                    .map(|l| format!("  {l}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
        },
        _ => (String::new(), String::new()),
    }
}

/// The folded lookup line: `read 3 files · 2 searches · 1 listing`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lookups {
    /// Files read.
    pub reads: usize,
    /// `grep` / `glob` calls.
    pub searches: usize,
    /// Directory listings.
    pub listings: usize,
    /// The most recent thing looked at.
    pub last: String,
}

impl Lookups {
    /// Count one lookup.
    pub fn add(&mut self, tool: &str, target: &str) {
        match tool {
            "read_file" => self.reads += 1,
            "list_dir" => self.listings += 1,
            _ => self.searches += 1,
        }
        self.last = target.to_string();
    }

    /// Lookups counted so far.
    pub fn count(&self) -> usize {
        self.reads + self.searches + self.listings
    }

    /// The row's label. One lookup reads plainly (`crates/…/mod.rs`); more
    /// become a count.
    pub fn label(&self) -> String {
        if self.count() == 1 {
            return self.last.clone();
        }
        let mut parts = Vec::new();
        let n =
            |k: usize, one: &str, many: &str| format!("{k} {}", if k == 1 { one } else { many });
        if self.reads > 0 {
            parts.push(format!("read {}", n(self.reads, "file", "files")));
        }
        if self.searches > 0 {
            parts.push(n(self.searches, "search", "searches"));
        }
        if self.listings > 0 {
            parts.push(n(self.listings, "listing", "listings"));
        }
        let mut s = parts.join(" · ");
        if !self.last.is_empty() {
            s.push_str(&format!("  ({})", self.last));
        }
        s
    }
}

/// What a turn did, for the line that closes it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnTally {
    /// Files created.
    pub created: usize,
    /// Files changed (rewritten or edited), counted once each.
    pub changed: usize,
    /// Lines added / removed by edits and new files.
    pub plus: usize,
    /// Lines removed.
    pub minus: usize,
    /// Commands that exited 0.
    pub ok: usize,
    /// Commands that failed.
    pub failed: usize,
    /// Paths already counted.
    seen: Vec<String>,
}

impl TurnTally {
    /// Count a finished step.
    pub fn add(&mut self, tool: &str, target: &str, output: &str, is_error: bool) {
        let first = output.lines().next().unwrap_or("");
        let num = |s: &str| {
            s.chars()
                .filter(char::is_ascii_digit)
                .collect::<String>()
                .parse::<usize>()
                .unwrap_or(0)
        };
        match tool {
            "bash" if is_error => self.failed += 1,
            "bash" => self.ok += 1,
            "write" if !is_error => {
                let lines = first.rsplit(" · ").next().unwrap_or("");
                let now = num(lines.split(" lines").next().unwrap_or(""));
                if first.starts_with("created ") {
                    self.created += 1;
                    self.seen.push(target.to_string());
                    self.plus += now;
                } else {
                    self.count_change(target);
                    let was = lines.split("(was ").nth(1).map(num).unwrap_or(0);
                    self.plus += now.saturating_sub(was);
                    self.minus += was.saturating_sub(now);
                }
            }
            "search_replace" if !is_error => {
                self.count_change(target);
                if let Some(d) = first.rsplit(" · ").next() {
                    let mut it = d.split_whitespace();
                    self.minus += it.next().map(num).unwrap_or(0);
                    self.plus += it.next().map(num).unwrap_or(0);
                }
            }
            _ => {}
        }
    }

    fn count_change(&mut self, target: &str) {
        if !self.seen.iter().any(|s| s == target) {
            self.seen.push(target.to_string());
            self.changed += 1;
        }
    }

    /// Nothing worth a closing line.
    pub fn is_empty(&self) -> bool {
        self.created + self.changed + self.ok + self.failed == 0
    }

    /// `9 files (6 new, 3 changed, +412 −18) · 4 commands (3 ok, 1 failed)`.
    pub fn line(&self) -> String {
        let mut parts = Vec::new();
        let files = self.created + self.changed;
        if files > 0 {
            let mut what = Vec::new();
            if self.created > 0 {
                what.push(format!("{} new", self.created));
            }
            if self.changed > 0 {
                what.push(format!("{} changed", self.changed));
            }
            what.push(format!("+{} −{}", self.plus, self.minus));
            parts.push(format!(
                "{files} file{} ({})",
                if files == 1 { "" } else { "s" },
                what.join(", ")
            ));
        }
        let cmds = self.ok + self.failed;
        if cmds > 0 {
            let status = if self.failed == 0 {
                format!("{} ok", self.ok)
            } else {
                format!("{} ok, {} failed", self.ok, self.failed)
            };
            parts.push(format!(
                "{cmds} command{} ({status})",
                if cmds == 1 { "" } else { "s" }
            ));
        }
        parts.join(" · ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn results_say_what_happened() {
        assert_eq!(
            result("write", "created /p/app/db.js · 48 lines", false).0,
            "new · 48 lines"
        );
        assert_eq!(
            result("write", "rewrote /p/a.js · 50 lines (was 40)", false).0,
            "rewrote · 50 lines (was 40)"
        );
        assert_eq!(
            result("search_replace", "updated /p/a.js · −3 +7 lines", false).0,
            "−3 +7"
        );
        let (d, body) = result(
            "bash",
            "collected 12 items\n...\n===== 12 passed in 0.31s =====\n",
            false,
        );
        assert_eq!(d, "✓ 12 passed in 0.31s");
        assert!(body.is_empty(), "the summary says it: {body}");
        let (d, body) = result(
            "bash",
            "Error: listen EADDRINUSE: address already in use :::8080\n    at Server.listen\n[exit 1]",
            true,
        );
        assert_eq!(d, "✗ exit 1");
        assert!(body.starts_with("! Error: listen EADDRINUSE"), "{body}");
        assert!(!body.contains("[exit"));
        let (d, _) = result("bash", "Ran 5 tests in 0.002s\n\nOK", false);
        assert_eq!(d, "✓ Ran 5 tests in 0.002s");
        // node:test (TAP totals), as npm test printed it in a real run.
        let node = "# tests 12\n# suites 0\n# pass 12\n# fail 0\n# cancelled 0\n# todo 0\n# duration_ms 184.36\n";
        assert_eq!(
            result("bash", node, false),
            ("✓ 12 passed".to_string(), String::new())
        );
        let (d, _) = result("bash", "# pass 11\n# fail 1\n[exit 1]", true);
        assert_eq!(d, "✗ 11 passed, 1 failed");
        assert_eq!(
            test_summary("# pass 11\n# fail 1"),
            Some("11 passed, 1 failed".into())
        );
        assert_eq!(
            test_summary("  12 passing (40ms)\n"),
            Some("12 passing (40ms)".into())
        );
    }

    /// From a real run: the cause was above npm's closing boilerplate.
    #[test]
    fn a_failure_shows_its_cause_not_just_its_tail() {
        let npm = "npm error Missing script: \"lint\"\nnpm error\nnpm error Did you mean this?\nnpm error   npm link # Symlink a package folder\nnpm error\nnpm error To see a list of scripts, run:\nnpm error   npm run\nnpm error A complete log of this run can be found in: /home/u/.npm/_logs/x.log\n[exit 1]";
        let (_, body) = result("bash", npm, true);
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines[0], "! npm error Missing script: \"lint\"");
        assert_eq!(lines[1], "! …");
        assert!(lines.last().unwrap().contains("complete log"));
        assert!(!body.contains("[exit"));
    }

    #[test]
    fn an_edit_previews_what_goes_and_comes() {
        let args = json!({"path": "a.js", "old_string": "const q = req.query.q\n", "new_string": "const q = (req.query.q || '').trim()\nif (!q) return all()\n"});
        assert_eq!(
            edit_preview("search_replace", &args),
            "- const q = req.query.q\n+ const q = (req.query.q || '').trim()\n+ if (!q) return all()"
        );
        let long = json!({"old_string": "a\nb\nc\nd\ne", "new_string": "x"});
        assert!(edit_preview("search_replace", &long).contains("- … 2 more"));
        // From a real run: the model quoted two unchanged lines to add one.
        let ctx = json!({"old_string": "target/\n.DS_Store\n", "new_string": "target/\n.DS_Store\ndata/\n"});
        assert_eq!(edit_preview("search_replace", &ctx), "+ data/");
    }

    #[test]
    fn lookups_fold_and_turns_add_up() {
        let mut l = Lookups::default();
        l.add("read_file", "app/db.js");
        l.add("read_file", "app/x.js");
        l.add("grep", "TODO");
        assert_eq!(l.label(), "read 2 files · 1 search  (TODO)");
        let mut one = Lookups::default();
        one.add("read_file", "app/db.js");
        assert_eq!(one.label(), "app/db.js");
        let mut t = TurnTally::default();
        t.add("write", "a.js", "created /p/a.js · 48 lines", false);
        t.add(
            "write",
            "b.js",
            "rewrote /p/b.js · 50 lines (was 40)",
            false,
        );
        t.add(
            "search_replace",
            "b.js",
            "updated /p/b.js · −3 +7 lines",
            false,
        );
        t.add("bash", "npm test", "5 passed", false);
        t.add("bash", "docker up", "boom\n[exit 1]", true);
        assert_eq!(
            t.line(),
            "2 files (1 new, 1 changed, +65 −3) · 2 commands (1 ok, 1 failed)"
        );
    }
}
