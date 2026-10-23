//! The tasks, as the fixtures describe them.
use std::fmt;
use std::str::FromStr;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::fixtures;

/// How hard a task is: `level` in its `task.toml`, standard when it says none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    #[default]
    Standard,
    Hard,
}

impl Level {
    pub const ALL: [Level; 2] = [Level::Standard, Level::Hard];

    pub fn name(self) -> &'static str {
        match self {
            Level::Standard => "standard",
            Level::Hard => "hard",
        }
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Level {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "standard" => Ok(Level::Standard),
            "hard" => Ok(Level::Hard),
            other => Err(format!("unknown level {other:?} (standard|hard)")),
        }
    }
}

/// One file of a task's project or hidden tests.
#[derive(Clone, Debug)]
pub struct File {
    /// Relative to the project.
    pub path: String,
    pub bytes: &'static [u8],
    pub executable: bool,
}

/// One task: what it asks for, how it is checked, and the files it starts from.
#[derive(Clone, Debug)]
pub struct Task {
    pub name: String,
    pub title: String,
    pub brief: String,
    pub level: Level,
    /// The files the brief names.
    pub files: Vec<String>,
    /// The visible checks: the run file's `test`, the first one its `check`.
    pub checks: Vec<String>,
    /// The hidden tests' commands, run once the hidden files are copied in.
    pub accept: Vec<String>,
    /// Commands that must succeed on a machine for the task to run there,
    /// from the fixture directory (`repo/` under it).
    pub needs: Vec<String>,
    pub repo: Vec<File>,
    pub hidden: Vec<File>,
}

#[derive(Deserialize)]
struct Spec {
    title: String,
    brief: String,
    #[serde(default)]
    level: Level,
    #[serde(default)]
    files: Vec<String>,
    checks: Vec<String>,
    accept: Vec<String>,
    #[serde(default)]
    needs: Vec<String>,
    #[serde(default)]
    tasks: Vec<SubTask>,
}

#[derive(Deserialize)]
struct SubTask {
    #[serde(default)]
    brief: String,
}

impl Task {
    /// The one prompt every tool gets: the scope paragraph and the brief.
    pub fn prompt(&self) -> String {
        format!("{}\n\n{}\n", fixtures::SCOPE.trim(), self.brief.trim())
    }
}

/// Every task the binary carries, by name.
pub fn tasks() -> &'static [Task] {
    static TASKS: OnceLock<Vec<Task>> = OnceLock::new();
    TASKS.get_or_init(|| {
        let mut names: Vec<&str> = fixtures::FILES
            .iter()
            .filter_map(|(path, _, _)| path.split_once('/').map(|(name, _)| name))
            .collect();
        names.dedup();
        names.into_iter().filter_map(parse).collect()
    })
}

fn parse(name: &str) -> Option<Task> {
    let prefix = format!("{name}/");
    let mut spec = None;
    let mut repo = Vec::new();
    let mut hidden = Vec::new();
    for (path, bytes, executable) in fixtures::FILES {
        let Some(rel) = path.strip_prefix(&prefix) else {
            continue;
        };
        if rel == "task.toml" {
            spec = toml::from_str::<Spec>(std::str::from_utf8(bytes).ok()?).ok();
        } else if let Some(p) = rel.strip_prefix("repo/") {
            repo.push(File {
                path: p.to_string(),
                bytes,
                executable: *executable,
            });
        } else if let Some(p) = rel.strip_prefix("hidden/") {
            hidden.push(File {
                path: p.to_string(),
                bytes,
                executable: *executable,
            });
        }
    }
    let spec = spec?;
    let mut brief = spec.brief.trim().to_string();
    for sub in &spec.tasks {
        if !sub.brief.trim().is_empty() {
            brief.push('\n');
            brief.push_str(sub.brief.trim());
        }
    }
    Some(Task {
        name: name.to_string(),
        title: spec.title,
        brief,
        level: spec.level,
        files: spec.files,
        checks: spec.checks,
        accept: spec.accept,
        needs: spec.needs,
        repo,
        hidden,
    })
}

/// The tasks named (all when `names` is empty) at `level` (any when `None`),
/// in the bench's order.
pub fn select(names: &[String], level: Option<Level>) -> Vec<&'static Task> {
    tasks()
        .iter()
        .filter(|t| names.is_empty() || names.iter().any(|n| n == &t.name))
        .filter(|t| level.is_none_or(|l| l == t.level))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_binary_carries_every_task_with_its_files_and_no_solution() {
        let all = tasks();
        assert!(all.len() >= 14, "{}", all.len());
        let hard: Vec<_> = select(&[], Some(Level::Hard))
            .iter()
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(
            hard,
            [
                "expr-precedence",
                "migrate-dependencies",
                "rust-csv-borrow",
                "ts-debounce"
            ]
        );
        assert_eq!(select(&[], Some(Level::Standard)).len(), all.len() - 4);
        let t = select(&["ts-debounce".into()], None)[0];
        assert!(t.repo.iter().any(|f| f.path == "src/timing.ts"));
        assert!(t.hidden.iter().any(|f| f.path == "test/hidden.test.ts"));
        assert!(
            !t.repo
                .iter()
                .chain(&t.hidden)
                .any(|f| f.path.contains("solution"))
        );
        assert!(
            fixtures::FILES
                .iter()
                .all(|(p, _, _)| !p.contains("/solution/"))
        );
        assert_eq!(t.checks, ["node --test test/basic.test.ts"]);
        assert_eq!(t.needs, ["node repo/src/timing.ts"]);
        assert!(t.prompt().starts_with("Work only in this project"));
        assert!(t.prompt().ends_with("passes.\n"), "{}", t.prompt());
        // The script-run task keeps its executable bit.
        let r = select(&["runner-script".into()], None)[0];
        assert!(r.repo.iter().any(|f| f.path == "dev"));
        // Sub-task briefs are joined into the one brief.
        let s = select(&["split-stats-report".into()], None)[0];
        assert!(s.brief.matches('\n').count() > 5, "{}", s.brief);
        assert!(select(&["nope".into()], None).is_empty());
        assert!(select(&["ts-debounce".into()], Some(Level::Standard)).is_empty());
    }

    #[test]
    fn levels_parse_and_print() {
        assert_eq!("hard".parse::<Level>(), Ok(Level::Hard));
        assert_eq!(" Standard ".parse::<Level>(), Ok(Level::Standard));
        assert!("easy".parse::<Level>().is_err());
        assert_eq!(Level::Hard.to_string(), "hard");
    }
}
