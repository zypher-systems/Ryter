//! Approved plans: `.ryter/plans/` in the project.
//!
//! A plan the user approved is a file, not a message. The build works from
//! it, a later review can hold the work against it, and the user decides
//! whether it is committed. One file per plan, named for the day and the
//! plan's title; an earlier plan is never written over.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// The folder, under the project's top.
pub const DIR: &str = ".ryter/plans";

/// The most a plan may be. One the user has to approve is one they can read.
pub const MAX_BYTES: usize = 64 * 1024;

/// `title` as a file name: lowercase words joined by dashes.
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
    if out.is_empty() { "plan".into() } else { out }
}

/// The plan as the file holds it: under its title, ending in one newline.
fn document(title: &str, plan: &str) -> String {
    let plan = plan.trim();
    // A plan that brings its own title keeps it.
    if plan.starts_with("# ") {
        format!("{plan}\n")
    } else {
        format!("# {}\n\n{plan}\n", title.trim())
    }
}

/// Save an approved plan under `root`, on `day` (`YYYY-MM-DD`). Returns the
/// file. A plan of the same name on the same day gets a number: `-2`, `-3`.
pub fn save_on(root: &Path, day: &str, title: &str, plan: &str) -> Result<PathBuf> {
    let dir = root.join(DIR);
    let io = |e: std::io::Error| Error::Io(format!("{}: {e}", dir.display()));
    std::fs::create_dir_all(&dir).map_err(io)?;
    let base = format!("{day}-{}", slug(title));
    let mut path = dir.join(format!("{base}.md"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{base}-{n}.md"));
        n += 1;
    }
    std::fs::write(&path, document(title, plan)).map_err(io)?;
    Ok(path)
}

/// [`save_on`] today.
pub fn save(root: &Path, title: &str, plan: &str) -> Result<PathBuf> {
    // The user's own date: a plan approved in the evening was filed under
    // tomorrow's.
    save_on(root, &crate::clock::today(), title, plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn a_plan_is_saved_under_its_day_and_title() {
        let root = TempDir::new().unwrap();
        let path = save_on(
            root.path(),
            "2026-10-01",
            "Add CSV export to reports!",
            "## Goal\nReports download as CSV.\n\n## Steps\n1. Add export_csv()\n",
        )
        .unwrap();
        assert_eq!(
            path,
            root.path()
                .join(".ryter/plans/2026-10-01-add-csv-export-to-reports.md")
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# Add CSV export to reports!\n\n## Goal\nReports download as CSV.\n\n## Steps\n1. Add export_csv()\n"
        );
        // The same title again that day is kept beside it, not over it.
        let again = save_on(
            root.path(),
            "2026-10-01",
            "Add CSV export to reports",
            "second",
        )
        .unwrap();
        assert!(again.ends_with("2026-10-01-add-csv-export-to-reports-2.md"));
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("export_csv")
        );
        // A plan with its own title keeps it; a title of no letters still names a file.
        let own = save_on(root.path(), "2026-10-02", "???", "# Mine\n\nbody").unwrap();
        assert!(own.ends_with("2026-10-02-plan.md"));
        assert_eq!(std::fs::read_to_string(&own).unwrap(), "# Mine\n\nbody\n");
    }
}
