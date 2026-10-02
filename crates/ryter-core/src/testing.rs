//! A test's report: what the tester tried, and how each thing went.
//!
//! The tester works in a conversation of its own, so its working (commands,
//! logs, dead ends) stays there. What comes back is the report: into the
//! conversation the other hats share, as one message the builder can fix
//! from; into a file under `.ryter/tests/`, which outlives the session; and
//! onto the commit's receipt.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// The folder, under the project's top.
pub const DIR: &str = ".ryter/tests";

/// The most scenarios one report holds. A report is read by a person.
pub const MAX_SCENARIOS: usize = 40;

/// The most one line of a scenario may be.
const MAX_LINE: usize = 300;

/// How a scenario went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// It did what it should.
    Pass,
    /// It didn't.
    Fail,
    /// It couldn't be tried, because something before it failed.
    NotReached,
}

/// One thing the tester tried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scenario {
    /// What was tried, in a few words.
    pub name: String,
    /// How it went.
    pub result: Outcome,
    /// What should have happened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    /// What did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub got: Option<String>,
    /// The steps to see it again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_see_it: Option<String>,
    /// A word more: `21 passed`, `needs 3`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A test's report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    /// What was tested, in a few words.
    pub title: String,
    /// What was tried, in order.
    pub scenarios: Vec<Scenario>,
    /// Anything else: what could not be tested, and why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// A report the tester has filed in this turn, on its way to the
/// conversation the other hats share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filed {
    /// The report.
    pub report: Report,
    /// Its file, as a path under the project's top.
    pub file: String,
}

/// Where a report is between the tester filing it and the others having
/// it.
#[derive(Debug, Clone, Default)]
pub struct Desk {
    /// Filed in the running turn, not yet given to the shared conversation.
    pub pending: Option<Filed>,
    /// The last report given: whether every scenario passed. `None` until
    /// one has been.
    pub delivered: Option<bool>,
}

/// One line, no longer than a line should be.
fn line(text: &str) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= MAX_LINE {
        return one;
    }
    let mut cut: String = one.chars().take(MAX_LINE - 1).collect();
    cut.push('…');
    cut
}

fn some_line(v: Option<&serde_json::Value>) -> Option<String> {
    v.and_then(serde_json::Value::as_str)
        .map(line)
        .filter(|s| !s.is_empty())
}

impl Report {
    /// Read a report from the tester's `report_test` call. `Err` says what
    /// is missing, for the model.
    pub fn from_args(args: &serde_json::Value) -> std::result::Result<Self, String> {
        let title = some_line(args.get("title")).ok_or("report_test needs `title`: a few words")?;
        let items = args
            .get("scenarios")
            .and_then(serde_json::Value::as_array)
            .filter(|a| !a.is_empty())
            .ok_or("report_test needs `scenarios`: each thing you tried, with how it went")?;
        if items.len() > MAX_SCENARIOS {
            return Err(format!(
                "that is {} scenarios. A report is read by a person: keep it to {MAX_SCENARIOS}, \
                 the ones that matter",
                items.len()
            ));
        }
        let mut scenarios = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let n = i + 1;
            let name = some_line(item.get("name"))
                .ok_or(format!("scenario {n} needs `name`: what you tried"))?;
            let result = match item.get("result").and_then(serde_json::Value::as_str) {
                Some("pass") => Outcome::Pass,
                Some("fail") => Outcome::Fail,
                Some("not_reached") => Outcome::NotReached,
                _ => {
                    return Err(format!(
                        "scenario {n} needs `result`: pass, fail, or not_reached"
                    ));
                }
            };
            let got = some_line(item.get("got"));
            // A failure the builder can't see again is not a finding yet.
            if result == Outcome::Fail && got.is_none() {
                return Err(format!(
                    "scenario {n} failed: say what happened in `got` (the status, the error \
                     line), and how to see it again in `to_see_it`"
                ));
            }
            scenarios.push(Scenario {
                name,
                result,
                expected: some_line(item.get("expected")),
                got,
                to_see_it: some_line(item.get("to_see_it")),
                note: some_line(item.get("note")),
            });
        }
        Ok(Self {
            title,
            scenarios,
            summary: args
                .get("summary")
                .and_then(serde_json::Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        })
    }

    /// How many scenarios did not pass.
    pub fn failed(&self) -> usize {
        self.scenarios
            .iter()
            .filter(|s| s.result != Outcome::Pass)
            .count()
    }

    /// Whether every scenario passed.
    pub fn passed(&self) -> bool {
        self.failed() == 0
    }

    /// `✗ 2 of 5 failed`, `✓ 5 of 5 passed`.
    pub fn headline(&self) -> String {
        let total = self.scenarios.len();
        match self.failed() {
            0 => format!("✓ {total} of {total} passed"),
            n => format!("✗ {n} of {total} failed"),
        }
    }

    /// The first scenario that failed, by its number: what "retest 3" names.
    pub fn first_failed(&self) -> Option<usize> {
        self.scenarios
            .iter()
            .position(|s| s.result == Outcome::Fail)
            .map(|i| i + 1)
    }

    /// The report as rows: a pass is one line, a failure is opened out with
    /// what was expected, what happened, and how to see it.
    pub fn rows(&self) -> Vec<String> {
        let mut rows = Vec::new();
        for (i, s) in self.scenarios.iter().enumerate() {
            let n = i + 1;
            let note = s
                .note
                .as_ref()
                .map(|t| format!(" · {t}"))
                .unwrap_or_default();
            match s.result {
                Outcome::Pass => rows.push(format!("✓ {n}  {}{note}", s.name)),
                Outcome::NotReached => {
                    let why = s.note.as_deref().unwrap_or("something before it failed");
                    rows.push(format!("✗ {n}  {} · not reached ({why})", s.name));
                }
                Outcome::Fail => {
                    rows.push(format!("✗ {n}  {}{note}", s.name));
                    if let Some(e) = &s.expected {
                        rows.push(format!("     expected {e}"));
                    }
                    if let Some(g) = &s.got {
                        rows.push(format!("     got {g}"));
                    }
                    if let Some(t) = &s.to_see_it {
                        rows.push(format!("     to see it: {t}"));
                    }
                }
            }
        }
        rows
    }

    /// The report as the conversation the other hats share is given it.
    pub fn message(&self, tester: &str, file: &str) -> String {
        let mut s = format!(
            "[Ryter] The test hat ({tester}) used the product and filed this report: {}.\n",
            self.headline()
        );
        for row in self.rows() {
            s.push_str(&row);
            s.push('\n');
        }
        if let Some(sum) = &self.summary {
            s.push_str(&format!("Also: {}\n", line(sum)));
        }
        s.push_str(&format!(
            "The full report is in `{file}`. The tester worked in a conversation of its own \
             and changed nothing."
        ));
        s
    }

    /// The report as its file holds it.
    pub fn document(&self, plan: Option<&str>, tester: &str, stamp: &str) -> String {
        let mut s = format!("# Test: {}\n\n", self.title);
        s.push_str(&format!("- Result: {}\n", self.headline()));
        s.push_str(&format!("- Tested by: {tester} · {stamp}\n"));
        match plan {
            Some(p) => s.push_str(&format!("- Plan: `{p}`\n")),
            None => s.push_str("- Plan: none approved\n"),
        }
        for (i, sc) in self.scenarios.iter().enumerate() {
            let how = match sc.result {
                Outcome::Pass => "pass",
                Outcome::Fail => "FAIL",
                Outcome::NotReached => "not reached",
            };
            s.push_str(&format!("\n## {}. {} — {how}\n", i + 1, sc.name));
            for (label, v) in [
                ("Expected", &sc.expected),
                ("Got", &sc.got),
                ("To see it", &sc.to_see_it),
                ("Note", &sc.note),
            ] {
                if let Some(v) = v {
                    s.push_str(&format!("- {label}: {v}\n"));
                }
            }
        }
        if let Some(sum) = &self.summary {
            s.push_str(&format!("\n## Notes\n{sum}\n"));
        }
        s
    }
}

/// `title` as part of a file name.
fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 48 {
            break;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "test".into() } else { out }
}

/// What a report's file is named for: the plan it tested, or its own title.
/// `.ryter/plans/2026-10-01-cms.md` gives `cms`.
pub fn subject(plan_file: Option<&str>, title: &str) -> String {
    let from_plan = plan_file.and_then(|p| {
        let stem = p.rsplit('/').next()?.strip_suffix(".md")?;
        // Past the plan's own date.
        let rest = stem
            .char_indices()
            .nth(10)
            .filter(|_| stem.as_bytes().get(10) == Some(&b'-'))
            .map_or(stem, |(i, _)| &stem[i + 1..]);
        let rest = rest.trim_matches('-');
        (!rest.is_empty()).then(|| rest.to_string())
    });
    from_plan.unwrap_or_else(|| slug(title))
}

/// Save a report's text under `root`, on `day` (`YYYY-MM-DD`), named for
/// `subject`. A second report of the same subject on the same day is `-2`,
/// then `-3`: an earlier one is never written over. Returns the file.
pub fn save_on(root: &Path, day: &str, subject: &str, text: &str) -> Result<PathBuf> {
    crate::plan::own_folder(root)?;
    let dir = root.join(DIR);
    let io = |e: std::io::Error| Error::Io(format!("{}: {e}", dir.display()));
    std::fs::create_dir_all(&dir).map_err(io)?;
    let base = format!("{day}-{}", slug(subject));
    let mut path = dir.join(format!("{base}.md"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{base}-{n}.md"));
        n += 1;
    }
    std::fs::write(&path, text).map_err(io)?;
    Ok(path)
}

/// The newest report in the project, as a path under its top.
pub fn latest(root: &Path) -> Option<String> {
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(root.join(DIR)).ok()?.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        let at = entry.metadata().and_then(|m| m.modified()).ok()?;
        if newest.as_ref().is_none_or(|(t, _)| at >= *t) {
            newest = Some((at, path));
        }
    }
    let (_, path) = newest?;
    Some(
        path.strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string(),
    )
}

/// How many things a plan says to verify: the list items under its "How to
/// verify" heading. `None` when it has no such list.
pub fn scenarios_in_plan(plan: &str) -> Option<usize> {
    let mut inside = false;
    let mut n = 0;
    for l in plan.lines() {
        let t = l.trim();
        if let Some(h) = t.strip_prefix('#') {
            let h = h.trim_start_matches('#').trim().to_ascii_lowercase();
            inside = h.starts_with("how to verify") || h.starts_with("verify");
            continue;
        }
        if !inside {
            continue;
        }
        let item = t.starts_with("- ")
            || t.starts_with("* ")
            || t.split_once(['.', ')'])
                .is_some_and(|(d, _)| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()));
        if item {
            n += 1;
        }
    }
    (n > 0).then_some(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn cms() -> serde_json::Value {
        serde_json::json!({
            "title": "CMS page list",
            "scenarios": [
                {"name": "the stack starts and is healthy", "result": "pass"},
                {"name": "first-run setup creates the admin", "result": "pass"},
                {"name": "/manage/ after login", "result": "fail",
                 "expected": "the page list",
                 "got": "500: NoReverseMatch 'pages:list'",
                 "to_see_it": "start the stack, log in, open /manage/"},
                {"name": "publish a page", "result": "not_reached", "note": "needs 3"},
                {"name": "pytest in the container", "result": "pass", "note": "21 passed"},
            ],
            "summary": "The media library was not tested:\nit needs a file upload.",
        })
    }

    /// The report reads as the user approved it: failures opened out,
    /// passes one line each.
    #[test]
    fn a_report_opens_its_failures_and_folds_its_passes() {
        let r = Report::from_args(&cms()).unwrap();
        assert_eq!(r.headline(), "✗ 2 of 5 failed");
        assert_eq!(
            (r.failed(), r.passed(), r.first_failed()),
            (2, false, Some(3))
        );
        assert_eq!(
            r.rows(),
            [
                "✓ 1  the stack starts and is healthy",
                "✓ 2  first-run setup creates the admin",
                "✗ 3  /manage/ after login",
                "     expected the page list",
                "     got 500: NoReverseMatch 'pages:list'",
                "     to see it: start the stack, log in, open /manage/",
                "✗ 4  publish a page · not reached (needs 3)",
                "✓ 5  pytest in the container · 21 passed",
            ]
        );
        let msg = r.message("kimi-k3", ".ryter/tests/2026-10-01-cms-2.md");
        assert!(
            msg.starts_with(
                "[Ryter] The test hat (kimi-k3) used the product and filed this report: ✗ 2 of 5 \
                 failed.\n✓ 1  the stack starts and is healthy\n"
            ),
            "{msg}"
        );
        assert!(
            msg.contains("Also: The media library was not tested: it needs a file upload.\n")
                && msg.contains("The full report is in `.ryter/tests/2026-10-01-cms-2.md`."),
            "{msg}"
        );
        let doc = r.document(
            Some(".ryter/plans/2026-10-01-cms.md"),
            "kimi-k3",
            "2026-10-01 14:05",
        );
        assert!(
            doc.starts_with(
                "# Test: CMS page list\n\n- Result: ✗ 2 of 5 failed\n- Tested by: kimi-k3 · \
                 2026-10-01 14:05\n- Plan: `.ryter/plans/2026-10-01-cms.md`\n\n## 1. the stack \
                 starts and is healthy — pass\n"
            ),
            "{doc}"
        );
        assert!(
            doc.contains(
                "## 3. /manage/ after login — FAIL\n- Expected: the page list\n- Got: 500: \
                 NoReverseMatch 'pages:list'\n- To see it: start the stack, log in, open \
                 /manage/\n"
            ) && doc.contains("## 4. publish a page — not reached\n- Note: needs 3\n")
                && doc.ends_with(
                    "## Notes\nThe media library was not tested:\nit needs a file upload.\n"
                ),
            "{doc}"
        );
        // All passing.
        let ok = Report::from_args(&serde_json::json!({
            "title": "x", "scenarios": [{"name": "a", "result": "pass"}]
        }))
        .unwrap();
        assert_eq!(ok.headline(), "✓ 1 of 1 passed");
        assert!(ok.passed() && ok.first_failed().is_none());
    }

    /// A report has to be one a person can act on: a title, something
    /// tried, a result for each, and for a failure what happened.
    #[test]
    fn a_report_the_builder_cannot_act_on_is_sent_back() {
        for (args, want) in [
            (serde_json::json!({"scenarios": []}), "needs `title`"),
            (serde_json::json!({"title": "x"}), "needs `scenarios`"),
            (
                serde_json::json!({"title": "x", "scenarios": []}),
                "needs `scenarios`",
            ),
            (
                serde_json::json!({"title": "x", "scenarios": [{"result": "pass"}]}),
                "scenario 1 needs `name`",
            ),
            (
                serde_json::json!({"title": "x", "scenarios": [{"name": "a", "result": "ok"}]}),
                "scenario 1 needs `result`",
            ),
            (
                serde_json::json!({"title": "x", "scenarios": [
                    {"name": "a", "result": "pass"}, {"name": "b", "result": "fail"}]}),
                "scenario 2 failed: say what happened",
            ),
        ] {
            let err = Report::from_args(&args).unwrap_err();
            assert!(err.contains(want), "{args}: {err}");
        }
        let many: Vec<_> = (0..MAX_SCENARIOS + 1)
            .map(|i| serde_json::json!({"name": format!("s{i}"), "result": "pass"}))
            .collect();
        let err =
            Report::from_args(&serde_json::json!({"title": "x", "scenarios": many})).unwrap_err();
        assert!(err.contains("keep it to 40"), "{err}");
        // A scenario can't break the report's shape.
        let r = Report::from_args(&serde_json::json!({"title": "x", "scenarios": [
            {"name": "two\nlines", "result": "fail", "got": "x".repeat(900)}]}))
        .unwrap();
        assert_eq!(r.scenarios[0].name, "two lines");
        assert!(r.scenarios[0].got.as_ref().unwrap().chars().count() <= MAX_LINE);
    }

    /// A report is named for the plan it tested and the day, and a second
    /// one that day is `-2`: an earlier report is never written over.
    #[test]
    fn a_report_is_filed_by_its_plan_and_day() {
        assert_eq!(subject(Some(".ryter/plans/2026-10-01-cms.md"), "x"), "cms");
        assert_eq!(
            subject(Some(".ryter/plans/2026-10-01-add-csv-export-2.md"), "x"),
            "add-csv-export-2"
        );
        assert_eq!(subject(Some(".ryter/plans/notes.md"), "x"), "notes");
        assert_eq!(subject(None, "The Page List!"), "the-page-list");
        assert_eq!(subject(None, "  "), "test");
        let root = TempDir::new().unwrap();
        assert_eq!(latest(root.path()), None);
        let first = save_on(root.path(), "2026-10-01", "cms", "one").unwrap();
        let second = save_on(root.path(), "2026-10-01", "cms", "two").unwrap();
        assert_eq!(first, root.path().join(".ryter/tests/2026-10-01-cms.md"));
        assert_eq!(second, root.path().join(".ryter/tests/2026-10-01-cms-2.md"));
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "one");
        assert!(latest(root.path()).is_some_and(|p| p.starts_with(".ryter/tests/2026-10-01-cms")));
    }

    #[test]
    fn a_plans_scenarios_are_counted_from_how_to_verify() {
        let plan = "## Goal\nx\n\n## Steps\n1. a\n2. b\n\n## How to verify\n- the stack \
                    starts\n- setup creates the admin\n1. log in\n2) open /manage/\n\n## Risks\n- none\n";
        assert_eq!(scenarios_in_plan(plan), Some(4));
        assert_eq!(scenarios_in_plan("## How to verify\nRead it.\n"), None);
        assert_eq!(scenarios_in_plan("## Goal\n- x\n"), None);
    }
}
