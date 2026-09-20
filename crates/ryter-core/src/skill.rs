//! Skills (`SKILL.md`) and user slash commands (`commands/*.md`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// One `SKILL.md` (or `skills/<name>.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    /// Slash name (`/review`).
    pub name: String,
    /// One-line help.
    pub description: String,
    /// When true, offered in the slash palette.
    pub user_invocable: bool,
    /// Body injected as the user turn.
    pub body: String,
    /// File it was loaded from.
    pub source: PathBuf,
}

impl Skill {
    /// Prompt sent to the orchestrator when the skill is invoked.
    pub fn expand(&self, args: &str) -> String {
        let mut s = format!("# Skill: {}\n\n{}", self.name, self.body.trim());
        if !args.trim().is_empty() {
            s.push_str("\n\nArguments: ");
            s.push_str(args.trim());
        }
        s
    }
}

/// One `~/.ryter/commands/<name>.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserCommand {
    /// Slash name.
    pub name: String,
    /// Optional first-heading help.
    pub description: String,
    /// Template; `$ARGUMENTS` is replaced.
    pub body: String,
    /// File it was loaded from.
    pub source: PathBuf,
}

impl UserCommand {
    /// Expand `$ARGUMENTS` (or append args when the placeholder is absent).
    pub fn expand(&self, args: &str) -> String {
        let args = args.trim();
        if self.body.contains("$ARGUMENTS") {
            self.body.replace("$ARGUMENTS", args)
        } else if args.is_empty() {
            self.body.clone()
        } else {
            format!("{}\n\n{args}", self.body.trim_end())
        }
    }
}

/// Skills + user commands. Project (trusted) wins over `~/.ryter`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SlashCatalog {
    /// Loaded skills, name-sorted.
    pub skills: Vec<Skill>,
    /// Loaded user commands, name-sorted.
    pub commands: Vec<UserCommand>,
}

impl SlashCatalog {
    /// Names that belong in the slash palette (user-invocable skills + commands).
    pub fn names(&self) -> Vec<String> {
        let mut n: Vec<String> = self
            .skills
            .iter()
            .filter(|s| s.user_invocable)
            .map(|s| s.name.clone())
            .collect();
        for c in &self.commands {
            if !n.iter().any(|x| x == &c.name) {
                n.push(c.name.clone());
            }
        }
        n
    }

    /// Expand a skill or user command. Skills win on a name clash.
    pub fn expand(&self, name: &str, args: &str) -> Option<String> {
        let l = name.to_ascii_lowercase();
        if let Some(s) = self.skills.iter().find(|s| s.user_invocable && s.name == l) {
            return Some(s.expand(args));
        }
        self.commands
            .iter()
            .find(|c| c.name == l)
            .map(|c| c.expand(args))
    }

    /// Text for `/skills`.
    pub fn list_text(&self) -> String {
        if self.skills.is_empty() && self.commands.is_empty() {
            return "no skills or user commands (add ~/.ryter/skills/<name>/SKILL.md or ~/.ryter/commands/<name>.md)".into();
        }
        let mut s = String::new();
        if !self.skills.is_empty() {
            s.push_str("skills:\n");
            for sk in &self.skills {
                let flag = if sk.user_invocable {
                    ""
                } else {
                    "  (not slash)"
                };
                s.push_str("  /");
                s.push_str(&sk.name);
                if !sk.description.is_empty() {
                    s.push_str("  ");
                    s.push_str(&sk.description);
                }
                s.push_str(flag);
                s.push('\n');
            }
        }
        if !self.commands.is_empty() {
            s.push_str("commands:\n");
            for c in &self.commands {
                s.push_str("  /");
                s.push_str(&c.name);
                if !c.description.is_empty() {
                    s.push_str("  ");
                    s.push_str(&c.description);
                }
                s.push('\n');
            }
        }
        s
    }
}

/// Load user then trusted project overlays.
pub fn load_catalog(home: &Path, project_root: Option<&Path>, trusted: bool) -> SlashCatalog {
    let mut skills: BTreeMap<String, Skill> = BTreeMap::new();
    let mut commands: BTreeMap<String, UserCommand> = BTreeMap::new();
    load_skills_dir(&home.join("skills"), &mut skills);
    load_commands_dir(&home.join("commands"), &mut commands);
    if trusted {
        if let Some(root) = project_root {
            let base = root.join(".ryter");
            load_skills_dir(&base.join("skills"), &mut skills);
            load_commands_dir(&base.join("commands"), &mut commands);
        }
    }
    SlashCatalog {
        skills: skills.into_values().collect(),
        commands: commands.into_values().collect(),
    }
}

/// Write `~/.ryter/skills/<name>/SKILL.md` (user-invocable stub).
pub fn write_skill(home: &Path, name: &str, description: &str) -> Result<PathBuf> {
    let name = name.trim().to_ascii_lowercase();
    if !valid_name(&name) {
        return Err(Error::Config(
            "skill name must start with a letter (a-z, 0-9, -, _)".into(),
        ));
    }
    let dir = home.join("skills").join(&name);
    fs::create_dir_all(&dir).map_err(|e| Error::Io(e.to_string()))?;
    let path = dir.join("SKILL.md");
    if path.is_file() {
        return Err(Error::Config(format!("skill {name} already exists")));
    }
    let desc = description.trim();
    let body = format!(
        "---\nname: {name}\ndescription: {desc}\nuser-invocable: true\n---\n\nDescribe what the orchestrator should do.\n"
    );
    fs::write(&path, body).map_err(|e| Error::Io(e.to_string()))?;
    Ok(path)
}

/// Write `~/.ryter/commands/<name>.md` with `$ARGUMENTS`.
pub fn write_command(home: &Path, name: &str, description: &str) -> Result<PathBuf> {
    let name = name.trim().to_ascii_lowercase();
    if !valid_name(&name) {
        return Err(Error::Config(
            "command name must start with a letter (a-z, 0-9, -, _)".into(),
        ));
    }
    let dir = home.join("commands");
    fs::create_dir_all(&dir).map_err(|e| Error::Io(e.to_string()))?;
    let path = dir.join(format!("{name}.md"));
    if path.is_file() {
        return Err(Error::Config(format!("command {name} already exists")));
    }
    let desc = description.trim();
    let body = format!("---\nname: {name}\ndescription: {desc}\n---\n\n$ARGUMENTS\n");
    fs::write(&path, body).map_err(|e| Error::Io(e.to_string()))?;
    Ok(path)
}

/// Delete a user skill or command file. Refuses paths outside `home/skills` or `home/commands`.
pub fn remove_catalog_entry(home: &Path, source: &Path) -> Result<()> {
    let home = fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    let src = fs::canonicalize(source).map_err(|e| Error::Io(e.to_string()))?;
    let skills = home.join("skills");
    let commands = home.join("commands");
    if !src.starts_with(&skills) && !src.starts_with(&commands) {
        return Err(Error::Config(
            "refusing to delete a project overlay skill".into(),
        ));
    }
    fs::remove_file(&src).map_err(|e| Error::Io(e.to_string()))?;
    if src.file_name().is_some_and(|n| n == "SKILL.md") {
        if let Some(parent) = src.parent() {
            let _ = fs::remove_dir(parent);
        }
    }
    Ok(())
}

fn load_skills_dir(dir: &Path, into: &mut BTreeMap<String, Skill>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for ent in rd.flatten() {
        let path = ent.path();
        if path.is_dir() {
            let skill = path.join("SKILL.md");
            if skill.is_file() {
                let fallback = ent.file_name().to_string_lossy().into_owned();
                if let Some(s) = parse_skill(&skill, &fallback) {
                    into.insert(s.name.clone(), s);
                }
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let fallback = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if let Some(s) = parse_skill(&path, &fallback) {
                into.insert(s.name.clone(), s);
            }
        }
    }
}

fn load_commands_dir(dir: &Path, into: &mut BTreeMap<String, UserCommand>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for ent in rd.flatten() {
        let path = ent.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let fallback = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if let Some(c) = parse_command(&path, &fallback) {
            into.insert(c.name.clone(), c);
        }
    }
}

fn parse_skill(path: &Path, fallback: &str) -> Option<Skill> {
    let text = fs::read_to_string(path).ok()?;
    let (fm, body) = split_frontmatter(&text);
    let raw_name = fm
        .get("name")
        .cloned()
        .unwrap_or_else(|| fallback.to_string());
    let name = raw_name.trim().to_ascii_lowercase();
    if !valid_name(&name) {
        return None;
    }
    let description = fm.get("description").cloned().unwrap_or_default();
    let user_invocable = fm
        .get("user-invocable")
        .or_else(|| fm.get("user_invocable"))
        .map(|s| parse_bool(s))
        .unwrap_or(true);
    Some(Skill {
        name,
        description,
        user_invocable,
        body: body.trim().to_string(),
        source: path.to_path_buf(),
    })
}

fn parse_command(path: &Path, fallback: &str) -> Option<UserCommand> {
    let text = fs::read_to_string(path).ok()?;
    let (fm, body) = split_frontmatter(&text);
    let raw_name = fm
        .get("name")
        .cloned()
        .unwrap_or_else(|| fallback.to_string());
    let name = raw_name.trim().to_ascii_lowercase();
    if !valid_name(&name) {
        return None;
    }
    let description = fm.get("description").cloned().unwrap_or_else(|| {
        body.lines()
            .find(|l| !l.trim().is_empty())
            .and_then(|l| l.trim().strip_prefix("# "))
            .unwrap_or("")
            .to_string()
    });
    Some(UserCommand {
        name,
        description,
        body,
        source: path.to_path_buf(),
    })
}

fn valid_name(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic() && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn parse_bool(s: &str) -> bool {
    matches!(
        s.trim().to_ascii_lowercase().as_str(),
        "true" | "yes" | "1" | "on"
    )
}

fn split_frontmatter(text: &str) -> (BTreeMap<String, String>, String) {
    let t = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Some(after) = t.strip_prefix("---") else {
        return (BTreeMap::new(), t.to_string());
    };
    let after = after
        .strip_prefix('\n')
        .or_else(|| after.strip_prefix("\r\n"))
        .unwrap_or(after);
    let mut fm = BTreeMap::new();
    let mut consumed = 0usize;
    let mut closed = false;
    for line in after.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        consumed += line.len();
        if content.trim() == "---" {
            closed = true;
            break;
        }
        if let Some((k, v)) = content.split_once(':') {
            let key = k.trim().to_ascii_lowercase();
            let mut val = v.trim().to_string();
            if val.len() >= 2
                && ((val.starts_with('"') && val.ends_with('"'))
                    || (val.starts_with('\'') && val.ends_with('\'')))
            {
                val = val[1..val.len() - 1].to_string();
            }
            if !key.is_empty() {
                fm.insert(key, val);
            }
        }
    }
    if !closed {
        return (BTreeMap::new(), t.to_string());
    }
    (fm, after[consumed..].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn skill_dir_and_frontmatter() {
        let home = TempDir::new().unwrap();
        let dir = home.path().join("skills/review");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            "---\nname: Review\ndescription: Review the diff\nuser-invocable: true\n---\nLook at git diff.\n",
        )
        .unwrap();
        let cat = load_catalog(home.path(), None, false);
        assert_eq!(cat.skills.len(), 1);
        assert_eq!(cat.skills[0].name, "review");
        assert_eq!(cat.skills[0].description, "Review the diff");
        assert!(cat.skills[0].user_invocable);
        let prompt = cat.expand("review", "src/lib.rs").unwrap();
        assert!(prompt.contains("Look at git diff."));
        assert!(prompt.contains("Arguments: src/lib.rs"));
        assert_eq!(cat.names(), vec!["review".to_string()]);
    }

    #[test]
    fn write_and_remove_skill_stub() {
        let home = TempDir::new().unwrap();
        let path = write_skill(home.path(), "summarize", "Sum up the diff").unwrap();
        assert!(path.ends_with("skills/summarize/SKILL.md"));
        let cat = load_catalog(home.path(), None, false);
        assert_eq!(cat.skills[0].name, "summarize");
        assert!(cat.skills[0].user_invocable);
        remove_catalog_entry(home.path(), &path).unwrap();
        assert!(load_catalog(home.path(), None, false).skills.is_empty());
    }

    #[test]
    fn non_invocable_skill_is_hidden_from_slash() {
        let home = TempDir::new().unwrap();
        fs::create_dir_all(home.path().join("skills")).unwrap();
        fs::write(
            home.path().join("skills/internal.md"),
            "---\nuser-invocable: false\n---\nsecret sauce\n",
        )
        .unwrap();
        let cat = load_catalog(home.path(), None, false);
        assert!(cat.names().is_empty());
        assert!(cat.expand("internal", "").is_none());
        assert!(cat.list_text().contains("(not slash)"));
    }

    #[test]
    fn user_command_arguments() {
        let home = TempDir::new().unwrap();
        fs::create_dir_all(home.path().join("commands")).unwrap();
        fs::write(
            home.path().join("commands/ticket.md"),
            "# Open a ticket\n\nFile $ARGUMENTS against the current bug.\n",
        )
        .unwrap();
        let cat = load_catalog(home.path(), None, false);
        assert_eq!(cat.commands[0].name, "ticket");
        assert_eq!(cat.commands[0].description, "Open a ticket");
        assert_eq!(
            cat.expand("ticket", "crash on boot").unwrap(),
            "# Open a ticket\n\nFile crash on boot against the current bug.\n"
        );
    }

    #[test]
    fn project_overlay_only_when_trusted() {
        let home = TempDir::new().unwrap();
        let proj = TempDir::new().unwrap();
        fs::create_dir_all(home.path().join("skills")).unwrap();
        fs::write(home.path().join("skills/foo.md"), "from home\n").unwrap();
        fs::create_dir_all(proj.path().join(".ryter/skills")).unwrap();
        fs::write(proj.path().join(".ryter/skills/foo.md"), "from project\n").unwrap();
        let untrusted = load_catalog(home.path(), Some(proj.path()), false);
        assert_eq!(untrusted.skills[0].body, "from home");
        let trusted = load_catalog(home.path(), Some(proj.path()), true);
        assert_eq!(trusted.skills[0].body, "from project");
    }

    #[test]
    fn invalid_name_is_skipped() {
        let home = TempDir::new().unwrap();
        fs::create_dir_all(home.path().join("skills")).unwrap();
        fs::write(
            home.path().join("skills/bad.md"),
            "---\nname: ../etc\n---\nnope\n",
        )
        .unwrap();
        let cat = load_catalog(home.path(), None, false);
        assert!(cat.skills.is_empty());
    }
}
