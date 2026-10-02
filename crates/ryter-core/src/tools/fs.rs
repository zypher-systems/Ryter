//! Filesystem tools.

use regex::RegexBuilder;
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};

use crate::diff::FileDiff;
use crate::error::{Error, Result};
use crate::tools::policy::resolve;
use crate::tools::{ToolContext, ToolOutput};

/// Lines returned when the caller does not ask for a range.
const DEFAULT_LINE_LIMIT: usize = 2_000;

pub fn read_file(args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    let path = require_path(args, ctx)?;
    let text = match open_for_read(&path, ctx).and_then(|mut file| {
        let mut text = String::new();
        file.read_to_string(&mut text)?;
        Ok(text)
    }) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            return Ok(ToolOutput::err(format!(
                "{} is not a text file (compiled or binary); read the source instead",
                path.display()
            )));
        }
        Err(e) => return Err(Error::Config(e.to_string())),
    };
    // `offset` is 1-based to match the line numbers this prints.
    let offset = args
        .get("offset")
        .and_then(Value::as_u64)
        .map(|n| n.max(1) as usize - 1)
        .unwrap_or(0);
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| n.max(1) as usize)
        .unwrap_or(DEFAULT_LINE_LIMIT);
    let all: Vec<&str> = text.lines().collect();
    let total = all.len();
    let end = offset.saturating_add(limit).min(total);
    let mut out: String = all
        .get(offset..end)
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(i, l)| format!("{:>4}|{l}", offset + i + 1))
        .collect::<Vec<_>>()
        .join("\n");
    if end < total {
        out.push_str(&format!(
            "\n… {} more lines; read again with offset {} …",
            total - end,
            end + 1
        ));
    }
    if offset >= total && total > 0 {
        out = format!(
            "offset {} is past the end of the file ({total} lines)",
            offset + 1
        );
    }
    Ok(ToolOutput::ok(out))
}

pub fn write_file(args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    let path = require_path(args, ctx)?;
    let content = args
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Config("write: missing content".into()))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| Error::Config(e.to_string()))?;
    }
    // Say what happened, for the model and the chat: a new file, or a
    // rewrite and how big it was before.
    let old = fs::read_to_string(&path).ok();
    let before = old.as_deref().map(|t| t.lines().count());
    let mut f = fs::File::create(&path).map_err(|e| Error::Config(e.to_string()))?;
    f.write_all(content.as_bytes())
        .map_err(|e| Error::Config(e.to_string()))?;
    let lines = content.lines().count();
    let diff = FileDiff::new(shown(&path, ctx), old.as_deref(), content);
    Ok(ToolOutput::ok(match before {
        None => format!("created {} · {lines} lines", path.display()),
        Some(was) => format!("rewrote {} · {lines} lines (was {was})", path.display()),
    })
    .with_diff(diff))
}

pub fn search_replace(args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    let path = require_path(args, ctx)?;
    let old = args
        .get("old_string")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Config("search_replace: missing old_string".into()))?;
    let new = args
        .get("new_string")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Config("search_replace: missing new_string".into()))?;
    let text = fs::read_to_string(&path).map_err(|e| Error::Config(e.to_string()))?;
    if old.is_empty() {
        return Ok(ToolOutput::err(
            "old_string is empty; quote the lines to replace (or use write for a new file)",
        ));
    }
    if old == new {
        return Ok(ToolOutput::err(
            "old_string and new_string are the same, so nothing would change",
        ));
    }
    // `read_file` shows lines without their `\r`, so a multi-line quote of
    // a CRLF file never matched, and the model tried again forever.
    let crlf = |t: &str| t.replace("\r\n", "\n").replace('\n', "\r\n");
    let (old, new) = if !text.contains(old) && text.contains("\r\n") && text.contains(&crlf(old)) {
        (crlf(old), crlf(new))
    } else {
        (old.to_string(), new.to_string())
    };
    let (old, new) = (old.as_str(), new.as_str());
    let count = text.matches(old).count();
    if count == 0 {
        return Ok(ToolOutput::err(not_found(&text, old)));
    }
    if count > 1 {
        let at: Vec<String> = text
            .match_indices(old)
            .take(6)
            .map(|(i, _)| (text[..i].matches('\n').count() + 1).to_string())
            .collect();
        return Ok(ToolOutput::err(format!(
            "old_string matched {count} times (lines {}); quote enough of the lines around \
             the one you mean to make it unique",
            at.join(", ")
        )));
    }
    let updated = text.replacen(old, new, 1);
    fs::write(&path, &updated).map_err(|e| Error::Config(e.to_string()))?;
    let (removed, added) = changed_lines(old, new);
    let diff = FileDiff::new(shown(&path, ctx), Some(&text), &updated);
    Ok(ToolOutput::ok(format!(
        "updated {} · −{} +{} lines",
        path.display(),
        removed.len(),
        added.len()
    ))
    .with_diff(diff))
}

/// What an edit would do, measured without doing it, for the person asked
/// to approve it. `None` when it wouldn't apply (the tool will say why).
pub fn preview(name: &str, args: &Value, ctx: &ToolContext) -> Option<FileDiff> {
    let path = require_path(args, ctx).ok()?;
    let old = fs::read_to_string(&path).ok();
    let new = match name {
        "write" => args.get("content")?.as_str()?.to_string(),
        "search_replace" | "propose_edit" => {
            let text = old.as_deref()?;
            let from = args.get("old_string")?.as_str()?;
            let to = args.get("new_string")?.as_str()?;
            if from.is_empty() || text.matches(from).count() != 1 {
                return None;
            }
            text.replacen(from, to, 1)
        }
        _ => return None,
    };
    Some(FileDiff::new(shown(&path, ctx), old.as_deref(), &new))
}

/// Why `old` isn't in `text`, and where to look. "old_string not found"
/// alone gave the model nothing to go on, and it sent the same edit again.
fn not_found(text: &str, old: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let first = old
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let near: Vec<usize> = if first.is_empty() {
        Vec::new()
    } else {
        lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.trim() == first || (first.len() >= 12 && l.contains(first)))
            .map(|(i, _)| i + 1)
            .take(3)
            .collect()
    };
    match near.as_slice() {
        [] => format!(
            "old_string not found, and its first line isn't in the file either ({} lines). \
             The file may have changed since you read it: read it again and quote it exactly.",
            lines.len()
        ),
        [n, ..] => format!(
            "old_string not found as written, but its first line is at line {}. Read lines \
             {}–{} again and quote them exactly: indentation, blank lines and trailing spaces \
             count.",
            near.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            n.saturating_sub(3).max(1),
            n + old.lines().count() + 3
        ),
    }
}

/// A path as the user knows it: relative inside the project. Resolved
/// paths are canonical, so a workspace reached through a symlink (every temp
/// folder on macOS: /var -> /private/var) is stripped in both spellings.
fn shown(path: &std::path::Path, ctx: &ToolContext) -> String {
    let real = fs::canonicalize(&ctx.workspace).unwrap_or_else(|_| ctx.workspace.clone());
    path.strip_prefix(&real)
        .or_else(|_| path.strip_prefix(&ctx.workspace))
        .unwrap_or(path)
        .display()
        .to_string()
}

/// The lines an edit really changes: what's left of `old` and `new` once
/// the lines they share at the start and end are set aside. Models quote
/// unchanged lines around an edit for context; those aren't changes.
pub fn changed_lines<'a>(old: &'a str, new: &'a str) -> (Vec<&'a str>, Vec<&'a str>) {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let tail = a[head..]
        .iter()
        .rev()
        .zip(b[head..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    (
        a[head..a.len() - tail].to_vec(),
        b[head..b.len() - tail].to_vec(),
    )
}

pub fn list_dir(args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    let path = match args.get("path").and_then(Value::as_str) {
        Some(p) if !p.is_empty() => require_resolved(ctx, p)?,
        _ => ctx.workspace.clone(),
    };
    let mut names = Vec::new();
    let rd = fs::read_dir(&path).map_err(|e| Error::Config(e.to_string()))?;
    for ent in rd.flatten() {
        let name = ent.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let suffix = if ent.path().is_dir() { "/" } else { "" };
        names.push(format!("{name}{suffix}"));
    }
    names.sort();
    Ok(ToolOutput::ok(names.join("\n")))
}

pub fn grep(args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    let pattern = args
        .get("pattern")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Config("grep: missing pattern".into()))?;
    let re = RegexBuilder::new(pattern)
        .case_insensitive(
            args.get("case_insensitive")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        )
        .build()
        .map_err(|e| Error::Config(e.to_string()))?;
    // Optional root and file filter. The root goes through the same
    // containment check as every other path argument.
    let root = match args.get("path").and_then(Value::as_str) {
        Some(p) if !p.is_empty() => crate::tools::policy::resolve(ctx, p)
            .ok_or_else(|| Error::Config(format!("grep: {p} is outside the workspace")))?,
        _ => ctx.workspace.clone(),
    };
    let mut builder = ignore::WalkBuilder::new(&root);
    builder.hidden(false).git_ignore(true);
    if let Some(include) = args.get("include").and_then(Value::as_str) {
        let mut ov = ignore::overrides::OverrideBuilder::new(&root);
        ov.add(include)
            .map_err(|e| Error::Config(format!("grep include: {e}")))?;
        builder.overrides(ov.build().map_err(|e| Error::Config(e.to_string()))?);
    }
    let walker = builder.build();
    let mut hits = Vec::new();
    for dent in walker.flatten() {
        let path = dent.path();
        if !dent.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        if crate::tools::policy::is_secret(path, ctx) {
            continue;
        }
        let Ok(mut file) = open_for_read(path, ctx) else {
            continue;
        };
        let mut text = String::new();
        if file.read_to_string(&mut text).is_err() {
            continue;
        }
        let rel = path.strip_prefix(&ctx.workspace).unwrap_or(path);
        for (i, line) in text.lines().enumerate() {
            if re.is_match(line) {
                hits.push(format!("{}:{}:{line}", rel.display(), i + 1));
                if hits.len() >= 200 {
                    break;
                }
            }
        }
        if hits.len() >= 200 {
            break;
        }
    }
    // An empty string reads as a tool failure to some models; say it plainly,
    // and say when the list was cut short.
    if hits.is_empty() {
        return Ok(ToolOutput::ok("no matches"));
    }
    let mut out = hits.join("\n");
    if hits.len() >= 200 {
        out.push_str("\n… stopped at 200 matches; narrow with path or include");
    }
    Ok(ToolOutput::ok(out))
}

fn open_for_read(path: &std::path::Path, ctx: &ToolContext) -> std::io::Result<fs::File> {
    for root in [&ctx.workspace, &ctx.notes_dir] {
        // Preserve the walked relative path: canonicalizing a file here
        // would hide the link that the anchored reader must refuse.
        for base in [
            root.clone(),
            fs::canonicalize(root).unwrap_or_else(|_| root.clone()),
        ] {
            if let Ok(rel) = path.strip_prefix(&base) {
                if crate::tools::policy::is_secret(path, ctx) {
                    return Err(std::io::Error::other("protected file"));
                }
                return crate::project_file::open(root, rel);
            }
        }
    }
    // Direct reads may also have been authorized outside the project by
    // require_resolved. They arrive canonicalized; retain that permission
    // while refusing links swapped in after the gate checked the path.
    if path.is_absolute() && !crate::tools::policy::is_secret(path, ctx) {
        return crate::project_file::open(
            std::path::Path::new("/"),
            path.strip_prefix("/").unwrap(),
        );
    }
    Err(std::io::Error::other("file is outside the workspace"))
}

pub fn glob_files(args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    let pattern = args
        .get("pattern")
        .or_else(|| args.get("glob"))
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Config("glob: missing pattern".into()))?;
    let full = ctx.workspace.join(pattern);
    let paths = glob::glob(&full.to_string_lossy()).map_err(|e| Error::Config(e.to_string()))?;
    let mut out = Vec::new();
    for p in paths.flatten() {
        if let Ok(rel) = p.strip_prefix(&ctx.workspace) {
            out.push(rel.display().to_string());
        }
        if out.len() >= 200 {
            break;
        }
    }
    out.sort();
    Ok(ToolOutput::ok(out.join("\n")))
}

fn require_path(args: &Value, ctx: &ToolContext) -> Result<std::path::PathBuf> {
    let raw = args
        .get("path")
        .or_else(|| args.get("target_file"))
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Config("missing path".into()))?;
    require_resolved(ctx, raw)
}

fn require_resolved(ctx: &ToolContext, raw: &str) -> Result<std::path::PathBuf> {
    resolve(ctx, raw)
        // Outside the project: scratch space and the user's own folder are
        // open to every hat. Anywhere else, the build hat's writes reach
        // here only after a person said yes to that exact path (policy:
        // AskOutside).
        .or_else(|| {
            crate::tools::policy::resolve_outside(ctx, raw).filter(|p| {
                ctx.role == crate::role::Role::SoloBuild
                    || crate::tools::policy::free_place(p, ctx, false)
            })
        })
        .ok_or_else(|| Error::Config(format!("path escapes workspace: {raw}")))
}

#[cfg(test)]
mod changed_tests {
    use super::changed_lines;

    #[test]
    fn shared_context_is_not_a_change() {
        let (r, a) = changed_lines("target/\n.DS_Store\n", "target/\n.DS_Store\ndata/\n");
        assert!(r.is_empty());
        assert_eq!(a, ["data/"]);
        let (r, a) = changed_lines("fn a() {\n  old()\n}\n", "fn a() {\n  new()\n  more()\n}\n");
        assert_eq!((r, a), (vec!["  old()"], vec!["  new()", "  more()"]));
    }
}
