//! Filesystem tools.

use regex::RegexBuilder;
use serde_json::Value;
use std::fs;
use std::io::Write;

use crate::error::{Error, Result};
use crate::tools::policy::resolve;
use crate::tools::{ToolContext, ToolOutput};

pub fn read_file(args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    let path = require_path(args, ctx)?;
    let text = fs::read_to_string(&path).map_err(|e| Error::Config(e.to_string()))?;
    let numbered: String = text
        .lines()
        .enumerate()
        .map(|(i, l)| format!("{:>4}|{l}", i + 1))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(ToolOutput::ok(numbered))
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
    let mut hits = Vec::new();
    let walker = ignore::WalkBuilder::new(&ctx.workspace)
        .hidden(false)
        .git_ignore(true)
        .build();
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
    Ok(ToolOutput::ok(hits.join("\n")))
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
    resolve(ctx, raw).ok_or_else(|| Error::Config(format!("path escapes workspace: {raw}")))
}
