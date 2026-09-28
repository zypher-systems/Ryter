//! The project's own build-and-test commands, read from its files.
//!
//! With no `[auditor] checks` set, nothing built or tested the crew's work
//! before it merged: every task rested on the auditor choosing to, and the
//! combined patch was never built at all. Ryter offers what it finds here,
//! and the user says yes once.

use std::fs;
use std::path::Path;

/// Commands that build and test the project at `dir`, from its manifest.
/// Empty when there is no manifest Ryter knows, or no tests to run.
pub fn detect(dir: &Path) -> Vec<String> {
    let has = |p: &str| dir.join(p).exists();
    let read = |p: &str| fs::read_to_string(dir.join(p)).unwrap_or_default();
    if has("Cargo.toml") {
        let workspace = read("Cargo.toml")
            .lines()
            .any(|l| l.trim() == "[workspace]");
        return vec![if workspace {
            "cargo test --workspace".into()
        } else {
            "cargo test".into()
        }];
    }
    if has("go.mod") {
        return vec!["go build ./...".into(), "go test ./...".into()];
    }
    if has("package.json") {
        return node(dir, &read("package.json"));
    }
    if has("pyproject.toml") || has("setup.py") {
        let pytest = has("pytest.ini")
            || has("conftest.py")
            || read("pyproject.toml").contains("pytest")
            || read("requirements.txt").contains("pytest")
            || read("requirements-dev.txt").contains("pytest");
        if pytest {
            return vec!["python3 -m pytest".into()];
        }
        if has("tests") || has("test") {
            return vec!["python3 -m unittest discover".into()];
        }
    }
    Vec::new()
}

/// `npm test` (or the lockfile's package manager), with `build` first when
/// the package has one. `npm init`'s placeholder test script is no test.
fn node(dir: &Path, manifest: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(manifest) else {
        return Vec::new();
    };
    let script = |name: &str| v["scripts"][name].as_str().map(str::to_string);
    let pm = if dir.join("pnpm-lock.yaml").exists() {
        "pnpm"
    } else if dir.join("yarn.lock").exists() {
        "yarn"
    } else if dir.join("bun.lockb").exists() || dir.join("bun.lock").exists() {
        "bun"
    } else {
        "npm"
    };
    let mut out = Vec::new();
    if script("build").is_some() {
        out.push(format!("{pm} run build"));
    }
    match script("test") {
        Some(t) if !t.contains("no test specified") => out.push(format!("{pm} run test")),
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (p, body) in files {
            let path = dir.path().join(p);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body).unwrap();
        }
        dir
    }

    #[test]
    fn each_manifest_names_its_commands() {
        type Case<'a> = (&'a [(&'a str, &'a str)], &'a [&'a str]);
        let cases: &[Case] = &[
            (
                &[("Cargo.toml", "[package]\nname = \"a\"\n")],
                &["cargo test"],
            ),
            (
                &[("Cargo.toml", "[workspace]\nmembers = [\"a\"]\n")],
                &["cargo test --workspace"],
            ),
            (
                &[("go.mod", "module x\n")],
                &["go build ./...", "go test ./..."],
            ),
            (
                &[
                    (
                        "package.json",
                        r#"{"scripts":{"build":"tsc","test":"vitest run"}}"#,
                    ),
                    ("pnpm-lock.yaml", ""),
                ],
                &["pnpm run build", "pnpm run test"],
            ),
            (
                &[(
                    "package.json",
                    r#"{"scripts":{"test":"echo \"Error: no test specified\" && exit 1"}}"#,
                )],
                &[],
            ),
            (
                &[("pyproject.toml", "[tool.pytest.ini_options]\n")],
                &["python3 -m pytest"],
            ),
            (
                &[("setup.py", ""), ("tests/test_a.py", "")],
                &["python3 -m unittest discover"],
            ),
            (&[("README.md", "hello")], &[]),
        ];
        for (files, want) in cases {
            let dir = tree(files);
            assert_eq!(detect(dir.path()), *want, "{files:?}");
        }
    }
}
