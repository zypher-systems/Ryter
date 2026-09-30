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
    /// When true, listed for the model, which loads it with `load_skill`
    /// when a task needs it (`model-invocable: false` keeps it slash-only).
    pub model_invocable: bool,
    /// Body injected as the user turn.
    pub body: String,
    /// File it was loaded from.
    pub source: PathBuf,
}

/// Skills that ship with Ryter. A user or project skill of the same name
/// replaces one.
const BUNDLED: &[(&str, &str)] = &[("canvas", include_str!("../skills/canvas/SKILL.md"))];

/// The skills Ryter ships with.
pub fn bundled_skills() -> Vec<Skill> {
    BUNDLED
        .iter()
        .filter_map(|(name, text)| {
            parse_skill_text(
                text,
                name,
                PathBuf::from(format!("(built in)/{name}/SKILL.md")),
            )
        })
        .collect()
}

impl Skill {
    /// What `load_skill` hands the model: the instructions, and the files
    /// in the skill's folder, which it reads with `load_skill` and `file`.
    /// Naming the folder was no use: the sandbox and the secrets rule keep
    /// `~/.ryter` out of `read`.
    pub fn load_text(&self) -> String {
        let mut s = format!(
            "# Skill: {}

{}
",
            self.name,
            self.body.trim()
        );
        let files = self.files();
        if !files.is_empty() {
            s.push_str(&format!(
                "\nThis skill's own files: {}. Read one with load_skill, giving its name and \
                 `file`.\n",
                files.join(", ")
            ));
        }
        s
    }

    /// The folder of a `<name>/SKILL.md` skill. A flat `<name>.md` skill and
    /// a built-in one have none.
    fn folder(&self) -> Option<&Path> {
        if self.source.file_name().is_some_and(|n| n == "SKILL.md") {
            self.source.parent().filter(|d| d.is_dir())
        } else {
            None
        }
    }

    /// Files in the skill's folder besides `SKILL.md`, by relative path.
    /// Hidden entries and links are left out; the list stops at 50.
    pub fn files(&self) -> Vec<String> {
        fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<String>) {
            let Ok(rd) = fs::read_dir(dir) else { return };
            let mut ents: Vec<_> = rd.flatten().collect();
            ents.sort_by_key(|e| e.file_name());
            for ent in ents {
                if out.len() >= 50 || ent.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
                let Ok(kind) = ent.file_type() else { continue };
                let path = ent.path();
                if kind.is_dir() && depth < 3 {
                    walk(root, &path, depth + 1, out);
                } else if kind.is_file() && !(depth == 0 && ent.file_name() == "SKILL.md") {
                    if let Ok(rel) = path.strip_prefix(root) {
                        out.push(rel.to_string_lossy().into_owned());
                    }
                }
            }
        }
        let mut out = Vec::new();
        if let Some(dir) = self.folder() {
            walk(dir, dir, 0, &mut out);
        }
        out
    }

    /// The text of `rel`, a file inside the skill's folder. Nothing outside
    /// it: no absolute paths, no `..`, and no link that leads out.
    pub fn read_file(&self, rel: &str) -> std::result::Result<String, String> {
        const MAX: u64 = 256 * 1024;
        let Some(dir) = self.folder() else {
            return Err(format!("the {} skill has no files of its own", self.name));
        };
        let rel_path = Path::new(rel);
        if rel.is_empty()
            || !rel_path
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
        {
            return Err(format!(
                "{rel:?} is not a file of the {} skill; give a name from its list",
                self.name
            ));
        }
        let (Ok(root), Ok(path)) = (fs::canonicalize(dir), fs::canonicalize(dir.join(rel_path)))
        else {
            return Err(format!("the {} skill has no file {rel:?}", self.name));
        };
        if !path.starts_with(&root) || !path.is_file() {
            return Err(format!("the {} skill has no file {rel:?}", self.name));
        }
        let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if size > MAX {
            return Err(format!(
                "{rel} is {} KB; the most load_skill reads is 256 KB",
                size / 1024
            ));
        }
        fs::read_to_string(&path).map_err(|e| format!("{rel}: {e}"))
    }

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

    /// Skills the model may load, name-sorted.
    pub fn for_model(&self) -> Vec<&Skill> {
        self.skills.iter().filter(|s| s.model_invocable).collect()
    }

    /// The skill `name` if the model may load it.
    pub fn model_skill(&self, name: &str) -> Option<&Skill> {
        let l = name.trim().trim_start_matches('/').to_ascii_lowercase();
        self.skills
            .iter()
            .find(|s| s.model_invocable && s.name == l)
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
    for s in bundled_skills() {
        skills.insert(s.name.clone(), s);
    }
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
    parse_skill_text(&text, fallback, path.to_path_buf())
}

fn parse_skill_text(text: &str, fallback: &str, source: PathBuf) -> Option<Skill> {
    let (fm, body) = split_frontmatter(text);
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
    let model_invocable = fm
        .get("model-invocable")
        .or_else(|| fm.get("model_invocable"))
        .map(|s| parse_bool(s))
        .unwrap_or(true);
    Some(Skill {
        name,
        description,
        user_invocable,
        model_invocable,
        body: body.trim().to_string(),
        source,
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

    /// The skills a test wrote: the catalog also holds the built-in ones.
    fn own(cat: &SlashCatalog) -> Vec<&Skill> {
        let built_in: Vec<String> = bundled_skills().into_iter().map(|s| s.name).collect();
        cat.skills
            .iter()
            .filter(|s| !built_in.contains(&s.name) || !s.source.starts_with("(built in)"))
            .collect()
    }

    /// Ryter ships the canvas skill: listed for the model and as `/canvas`.
    /// A skill of the same name in `~/.ryter/skills` replaces it.
    #[test]
    fn the_canvas_skill_ships_and_can_be_replaced() {
        let home = TempDir::new().unwrap();
        let cat = load_catalog(home.path(), None, false);
        let canvas = cat.model_skill("canvas").expect("built in");
        assert!(canvas.user_invocable && canvas.description.contains("page"));
        assert!(canvas.body.contains("show_page"));
        assert!(canvas.load_text().starts_with("# Skill: canvas"));
        assert!(!canvas.load_text().contains("own files"));
        assert_eq!(
            cat.model_skill("/Canvas").map(|s| &s.name),
            Some(&canvas.name)
        );
        fs::create_dir_all(home.path().join("skills/canvas")).unwrap();
        fs::write(
            home.path().join("skills/canvas/SKILL.md"),
            "---\ndescription: my pages\n---\nmine\n",
        )
        .unwrap();
        let cat = load_catalog(home.path(), None, false);
        let mine = cat.model_skill("canvas").unwrap();
        assert_eq!(mine.body, "mine");
        assert!(!mine.load_text().contains("own files"), "only SKILL.md");
    }

    /// A skill's own files are listed by name and read through the skill,
    /// and nothing outside its folder is: `..`, absolute paths and links
    /// that lead out are refused. The first version gave the model the
    /// folder's path, which the sandbox and the secrets rule keep it from
    /// reading.
    #[test]
    fn a_skill_reads_its_own_files_and_nothing_else() {
        let home = TempDir::new().unwrap();
        let dir = home.path().join("skills/deploy");
        fs::create_dir_all(dir.join("scripts")).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            "---\ndescription: d\n---\nsee notes.md\n",
        )
        .unwrap();
        fs::write(dir.join("notes.md"), "the notes").unwrap();
        fs::write(dir.join("scripts/check.sh"), "echo ok").unwrap();
        fs::write(dir.join(".hidden"), "x").unwrap();
        fs::write(dir.join("big.txt"), "x".repeat(300 * 1024)).unwrap();
        fs::create_dir_all(home.path().join("keys")).unwrap();
        fs::write(home.path().join("keys/spacexai"), "xai-secret").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(home.path().join("keys/spacexai"), dir.join("key")).unwrap();
        fs::write(home.path().join("skills/flat.md"), "flat\n").unwrap();

        let cat = load_catalog(home.path(), None, false);
        let skill = cat.model_skill("deploy").unwrap();
        assert_eq!(skill.files(), ["big.txt", "notes.md", "scripts/check.sh"]);
        let text = skill.load_text();
        assert!(
            text.contains("own files: big.txt, notes.md, scripts/check.sh")
                && !text.contains(&home.path().display().to_string()),
            "{text}"
        );
        assert_eq!(skill.read_file("notes.md").unwrap(), "the notes");
        assert_eq!(skill.read_file("scripts/check.sh").unwrap(), "echo ok");
        for bad in [
            "key",
            "../flat.md",
            "../../keys/spacexai",
            "/etc/hostname",
            "",
            "missing.md",
            "scripts",
        ] {
            let err = skill.read_file(bad).unwrap_err();
            assert!(!err.contains("xai-secret"), "{bad}: {err}");
        }
        assert!(skill.read_file("big.txt").unwrap_err().contains("256 KB"));
        let flat = cat.model_skill("flat").unwrap();
        assert!(flat.files().is_empty(), "a flat skill's folder is others'");
        assert!(flat.read_file("deploy/notes.md").is_err());
    }

    /// `model-invocable: false` keeps a skill to the slash palette.
    #[test]
    fn a_skill_can_be_kept_from_the_model() {
        let home = TempDir::new().unwrap();
        fs::create_dir_all(home.path().join("skills")).unwrap();
        fs::write(
            home.path().join("skills/deploy.md"),
            "---\ndescription: ship it\nmodel-invocable: false\n---\nrun the deploy\n",
        )
        .unwrap();
        let cat = load_catalog(home.path(), None, false);
        assert!(cat.names().contains(&"deploy".to_string()));
        assert!(cat.model_skill("deploy").is_none());
        assert!(!cat.for_model().iter().any(|s| s.name == "deploy"));
        assert!(cat.for_model().iter().any(|s| s.name == "canvas"));
    }

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
        let skills = own(&cat);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "review");
        assert_eq!(skills[0].description, "Review the diff");
        assert!(skills[0].user_invocable);
        let prompt = cat.expand("review", "src/lib.rs").unwrap();
        assert!(prompt.contains("Look at git diff."));
        assert!(prompt.contains("Arguments: src/lib.rs"));
        assert_eq!(
            cat.names(),
            vec!["canvas".to_string(), "review".to_string()]
        );
    }

    #[test]
    fn write_and_remove_skill_stub() {
        let home = TempDir::new().unwrap();
        let path = write_skill(home.path(), "summarize", "Sum up the diff").unwrap();
        assert!(path.ends_with("skills/summarize/SKILL.md"));
        let cat = load_catalog(home.path(), None, false);
        assert_eq!(own(&cat)[0].name, "summarize");
        assert!(own(&cat)[0].user_invocable);
        remove_catalog_entry(home.path(), &path).unwrap();
        assert!(own(&load_catalog(home.path(), None, false)).is_empty());
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
        assert!(!cat.names().contains(&"internal".to_string()));
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
        assert_eq!(own(&untrusted)[0].body, "from home");
        let trusted = load_catalog(home.path(), Some(proj.path()), true);
        assert_eq!(own(&trusted)[0].body, "from project");
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
        assert!(own(&cat).is_empty());
    }
}
