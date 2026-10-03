//! An audit: what the audit hat found, filed with `file_audit`, rendered
//! to `.ryter/audit.md` and a dated copy, and shown on a popout.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// The most findings one audit may carry: a report a person reads.
pub const MAX_FINDINGS: usize = 40;
/// Where the latest audit is, as a path in the project.
pub const FILE: &str = ".ryter/audit.md";
/// Where every audit is kept, dated.
pub const DIR: &str = ".ryter/audits";

/// How one finding went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Pass,
    Fail,
    NotReached,
}

/// One thing the audit checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub result: Outcome,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#where: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saw: Option<String>,
}

/// An audit, as filed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Audit {
    /// Whether it passed.
    pub passed: bool,
    /// One line.
    pub summary: String,
    /// Worst first.
    pub findings: Vec<Finding>,
    /// The commands and tools it used.
    pub ran: Vec<String>,
}

const MAX_LINE: usize = 400;

/// One line of text, cut to a size a row can hold.
fn line(s: &str) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
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

impl Audit {
    /// Read an audit from the `file_audit` call. `Err` says what is
    /// missing, for the model.
    pub fn from_args(args: &serde_json::Value) -> std::result::Result<Self, String> {
        let passed = match args.get("verdict").and_then(serde_json::Value::as_str) {
            Some("pass") => true,
            Some("fail") => false,
            _ => return Err("file_audit needs `verdict`: pass or fail".into()),
        };
        let summary = some_line(args.get("summary"))
            .ok_or("file_audit needs `summary`: one line on what was found")?;
        let items = args
            .get("findings")
            .and_then(serde_json::Value::as_array)
            .filter(|a| !a.is_empty())
            .ok_or("file_audit needs `findings`: each thing checked, worst first")?;
        if items.len() > MAX_FINDINGS {
            return Err(format!(
                "that is {} findings. An audit is read by a person: keep it to {MAX_FINDINGS}, \
                 the ones that matter",
                items.len()
            ));
        }
        let mut findings = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let n = i + 1;
            let title = some_line(item.get("title"))
                .ok_or(format!("finding {n} needs `title`: what was checked"))?;
            let result = match item.get("result").and_then(serde_json::Value::as_str) {
                Some("pass") => Outcome::Pass,
                Some("fail") => Outcome::Fail,
                Some("not_reached") => Outcome::NotReached,
                _ => {
                    return Err(format!(
                        "finding {n} needs `result`: pass, fail, or not_reached"
                    ));
                }
            };
            let detail = some_line(item.get("detail"));
            let saw = some_line(item.get("saw"));
            if result == Outcome::Fail && detail.is_none() && saw.is_none() {
                return Err(format!(
                    "finding {n} failed: say what is wrong in `detail`, and what you ran and \
                     saw in `saw`, so the builder can see it again"
                ));
            }
            findings.push(Finding {
                result,
                title,
                r#where: some_line(item.get("where")),
                detail,
                saw,
            });
        }
        let ran = args
            .get("ran")
            .and_then(serde_json::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(line))
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            passed,
            summary,
            findings,
            ran,
        })
    }

    fn mark(result: Outcome) -> &'static str {
        match result {
            Outcome::Pass => "✓",
            Outcome::Fail => "✗",
            Outcome::NotReached => "○",
        }
    }

    /// `✗ 2 of 6 failed` or `✓ 6 of 6 passed`.
    pub fn headline(&self) -> String {
        let total = self.findings.len();
        let failed = self
            .findings
            .iter()
            .filter(|f| f.result == Outcome::Fail)
            .count();
        if failed > 0 || !self.passed {
            format!("✗ {failed} of {total} failed")
        } else {
            let passed = self
                .findings
                .iter()
                .filter(|f| f.result == Outcome::Pass)
                .count();
            format!("✓ {passed} of {total} passed")
        }
    }

    /// The findings as rows for the screen: `mark n\ttitle\twhere` (a
    /// not-reached finding carries its reason in the third field), then
    /// `    detail` and `    saw: …` rows for a failure.
    pub fn rows(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (i, f) in self.findings.iter().enumerate() {
            let n = i + 1;
            let right = match f.result {
                Outcome::NotReached => format!(
                    "not reached: {}",
                    f.detail.as_deref().unwrap_or("an earlier check failed")
                ),
                _ => f.r#where.clone().unwrap_or_default(),
            };
            out.push(format!(
                "{} {n}\t{}\t{right}",
                Self::mark(f.result),
                f.title
            ));
            if f.result == Outcome::Fail {
                if let Some(d) = &f.detail {
                    out.push(format!("    {d}"));
                }
                if let Some(s) = &f.saw {
                    out.push(format!("    saw: {s}"));
                }
            }
        }
        out
    }

    /// The audit as Markdown, for its file.
    pub fn document(
        &self,
        model: &str,
        stamp: &str,
        restored: &[String],
        checkpointed: bool,
    ) -> String {
        let mut s = String::new();
        let day = stamp.split_whitespace().next().unwrap_or(stamp);
        s.push_str(&format!(
            "# Audit · {day} · {}\n\n{}\n\n",
            if self.passed { "PASS" } else { "FAIL" },
            self.summary
        ));
        s.push_str("## Findings\n\n");
        for (i, f) in self.findings.iter().enumerate() {
            let n = i + 1;
            let where_ = f
                .r#where
                .as_deref()
                .map(|w| format!(" — `{w}`"))
                .unwrap_or_default();
            s.push_str(&format!(
                "{n}. {} **{}**{where_}\n",
                Self::mark(f.result),
                f.title
            ));
            if let Some(d) = &f.detail {
                s.push_str(&format!("   - {d}\n"));
            }
            if let Some(w) = &f.saw {
                s.push_str(&format!("   - saw: {w}\n"));
            }
        }
        s.push_str("\n## Ran\n\n");
        if self.ran.is_empty() {
            s.push_str("- nothing recorded\n");
        }
        for r in &self.ran {
            s.push_str(&format!("- `{r}`\n"));
        }
        s.push_str("\n## Tree\n\n");
        if !checkpointed {
            s.push_str("No checkpoint: this folder is not a git repository, so the audit ran read-only commands only.\n");
        } else if restored.is_empty() {
            s.push_str("Changed nothing; the checkpoint was kept.\n");
        } else {
            s.push_str(&format!(
                "The audit left {} file{} changed; they were put back from the checkpoint:\n",
                restored.len(),
                if restored.len() == 1 { "" } else { "s" }
            ));
            for p in restored {
                s.push_str(&format!("- `{p}`\n"));
            }
        }
        s.push_str(&format!("\n- Audited by: {model} · {stamp}\n"));
        s
    }
}

/// A file name from the audit's summary: lower case, words joined by
/// dashes, at most six words.
pub fn slug(summary: &str) -> String {
    let words: Vec<String> = summary
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(6)
        .map(|w| w.to_lowercase())
        .collect();
    if words.is_empty() {
        "audit".into()
    } else {
        words.join("-")
    }
}

/// Write the audit: `.ryter/audit.md`, replaced, and `.ryter/audits/<day>-<slug>.md`,
/// kept (a second one the same day gets a number). Returns both paths.
pub fn save(root: &Path, day: &str, slug: &str, text: &str) -> Result<(PathBuf, PathBuf)> {
    let dir = root.join(DIR);
    std::fs::create_dir_all(&dir).map_err(|e| Error::Io(e.to_string()))?;
    let mut dated = dir.join(format!("{day}-{slug}.md"));
    let mut n = 2;
    while dated.exists() {
        dated = dir.join(format!("{day}-{slug}-{n}.md"));
        n += 1;
    }
    std::fs::write(&dated, text).map_err(|e| Error::Io(e.to_string()))?;
    let latest = root.join(FILE);
    std::fs::write(&latest, text).map_err(|e| Error::Io(e.to_string()))?;
    Ok((latest, dated))
}

/// Whether `path` (project-relative or absolute under `root`) is one of
/// the audit's own files, which a rollback leaves alone and the audit hat
/// may write.
pub fn is_audit_file(root: &Path, path: &Path) -> bool {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel == Path::new(FILE) || rel.starts_with(DIR)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filed() -> Audit {
        Audit::from_args(&serde_json::json!({
            "verdict": "fail",
            "summary": "the upload limit and the delete route; the rest holds",
            "findings": [
                {"result": "fail", "title": "Size limit checked after the write", "where": "app/images.py:41",
                 "detail": "A 6 MB upload lands in uploads/ and is refused only then.", "saw": "curl -F picture=@big.png → 200"},
                {"result": "pass", "title": "Physics tests", "where": "6 passed"},
                {"result": "not_reached", "title": "Pause and restart keys", "detail": "headless run, no display"}
            ],
            "ran": ["run_project test", "curl -F picture=@big.png localhost:8001/items/new"]
        }))
        .unwrap()
    }

    #[test]
    fn the_shape_is_checked_and_said() {
        for (args, want) in [
            (serde_json::json!({}), "needs `verdict`"),
            (serde_json::json!({"verdict": "fail"}), "needs `summary`"),
            (
                serde_json::json!({"verdict": "pass", "summary": "x"}),
                "needs `findings`",
            ),
            (
                serde_json::json!({"verdict": "pass", "summary": "x", "findings": [{"result": "pass"}]}),
                "finding 1 needs `title`",
            ),
            (
                serde_json::json!({"verdict": "pass", "summary": "x", "findings": [{"title": "a", "result": "meh"}]}),
                "finding 1 needs `result`",
            ),
            (
                serde_json::json!({"verdict": "fail", "summary": "x", "findings": [{"title": "a", "result": "fail"}]}),
                "finding 1 failed: say what is wrong",
            ),
        ] {
            let err = Audit::from_args(&args).unwrap_err();
            assert!(err.contains(want), "{args}: {err}");
        }
        let many: Vec<_> = (0..MAX_FINDINGS + 1)
            .map(|i| serde_json::json!({"title": format!("f{i}"), "result": "pass"}))
            .collect();
        let err = Audit::from_args(
            &serde_json::json!({"verdict": "pass", "summary": "x", "findings": many}),
        )
        .unwrap_err();
        assert!(err.contains("keep it to 40"), "{err}");
        let a = filed();
        assert_eq!(a.headline(), "✗ 1 of 3 failed");
        assert_eq!(
            a.rows()[0],
            "✗ 1\tSize limit checked after the write\tapp/images.py:41"
        );
        assert_eq!(
            a.rows()[1],
            "    A 6 MB upload lands in uploads/ and is refused only then."
        );
        assert_eq!(a.rows()[2], "    saw: curl -F picture=@big.png → 200");
        assert_eq!(a.rows()[3], "✓ 2\tPhysics tests\t6 passed");
        assert_eq!(
            a.rows()[4],
            "○ 3\tPause and restart keys\tnot reached: headless run, no display"
        );
        let passing = Audit::from_args(&serde_json::json!({"verdict": "pass", "summary": "fine",
            "findings": [{"title": "a", "result": "pass"}, {"title": "b", "result": "not_reached"}]}))
        .unwrap();
        assert_eq!(passing.headline(), "✓ 1 of 2 passed");
    }

    #[test]
    fn the_document_and_the_files() {
        let a = filed();
        let text = a.document("kimi-k3", "2026-10-03 19:48", &[], true);
        assert!(text.starts_with("# Audit · 2026-10-03 · FAIL\n"));
        assert!(text.contains("1. ✗ **Size limit checked after the write** — `app/images.py:41`"));
        assert!(text.contains("   - saw: curl -F picture=@big.png → 200"));
        assert!(text.contains("- `run_project test`"));
        assert!(text.contains("Changed nothing; the checkpoint was kept."));
        let restored = a.document("m", "2026-10-03 19:48", &["app/x.py".into()], true);
        assert!(restored.contains("left 1 file changed") && restored.contains("- `app/x.py`"));
        let bare = a.document("m", "2026-10-03 19:48", &[], false);
        assert!(bare.contains("No checkpoint"));
        assert_eq!(slug(&a.summary), "the-upload-limit-and-the-delete");
        assert_eq!(slug("  "), "audit");
        let dir = tempfile::TempDir::new().unwrap();
        let (latest, first) = save(dir.path(), "2026-10-03", "x", "one").unwrap();
        assert_eq!(latest, dir.path().join(FILE));
        assert_eq!(first, dir.path().join(DIR).join("2026-10-03-x.md"));
        let (_, second) = save(dir.path(), "2026-10-03", "x", "two").unwrap();
        assert_eq!(second, dir.path().join(DIR).join("2026-10-03-x-2.md"));
        assert_eq!(std::fs::read_to_string(&latest).unwrap(), "two");
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "one");
        assert!(is_audit_file(dir.path(), &latest));
        assert!(is_audit_file(
            dir.path(),
            Path::new(".ryter/audits/2026-10-03-x.md")
        ));
        assert!(!is_audit_file(dir.path(), Path::new(".ryter/plan.md")));
        assert!(!is_audit_file(dir.path(), Path::new("src/main.rs")));
    }
}
