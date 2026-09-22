//! Filesystem tools.

use regex::RegexBuilder;
use serde_json::Value;
use std::fs;
use std::io::Write;

use crate::error::{Error, Result};
use crate::tools::policy::resolve;
use crate::tools::{ToolContext, ToolOutput};

/// Lines returned when the caller does not ask for a range.
const DEFAULT_LINE_LIMIT: usize = 2_000;

pub fn read_file(args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    let path = require_path(args, ctx)?;
    let text = match fs::read_to_string(&path) {
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
    let mut f = fs::File::create(&path).map_err(|e| Error::Config(e.to_string()))?;
    f.write_all(content.as_bytes())
        .map_err(|e| Error::Config(e.to_string()))?;
    Ok(ToolOutput::ok(format!("wrote {}", path.display())))
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
    let count = text.matches(old).count();
    if count == 0 {
        return Ok(ToolOutput::err("old_string not found"));
    }
    if count > 1 {
        return Ok(ToolOutput::err(format!(
            "old_string matched {count} times; must be unique"
        )));
    }
    let updated = text.replacen(old, new, 1);
    fs::write(&path, updated).map_err(|e| Error::Config(e.to_string()))?;
    Ok(ToolOutput::ok(format!("updated {}", path.display())))
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
        if !path.is_file() {
            continue;
        }
        if crate::tools::policy::is_secret(path, ctx) {
            continue;
        }
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
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
        // The build hat's outside writes reach here only after a person said
        // yes to that exact path (policy: AskOutside).
        .or_else(|| {
            (ctx.role == crate::role::Role::SoloBuild)
                .then(|| crate::tools::policy::resolve_outside(ctx, raw))
                .flatten()
        })
        .ok_or_else(|| Error::Config(format!("path escapes workspace: {raw}")))
}
