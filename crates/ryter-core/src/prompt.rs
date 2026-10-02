//! The prompt: shipped, or the user's (`~/.ryter/prompts/solo.md`), or the
//! project's (`.ryter/prompts/solo.md`), with what this session adds to it.

use std::fs;
use std::path::Path;

const SOLO: &str = include_str!("../../../prompts/solo.md");

/// The shipped prompt.
pub fn shipped() -> &'static str {
    SOLO
}

/// Load a prompt with overrides: project (if trusted) > user home > shipped.
pub fn load(home: &Path, project_root: Option<&Path>, trusted: bool) -> String {
    let name = "solo.md";
    if trusted {
        if let Some(root) = project_root {
            let p = root.join(".ryter").join("prompts").join(name);
            if let Ok(s) = fs::read_to_string(&p) {
                return s;
            }
        }
    }
    let user = home.join("prompts").join(name);
    if let Ok(s) = fs::read_to_string(&user) {
        return s;
    }
    SOLO.to_string()
}

/// `RYTER.md`, or `AGENTS.md` if that file is missing. Loaded from the project
/// root without a trust gate (markdown instructions, not executable hooks).
pub fn load_project_instructions(project_root: Option<&Path>) -> Option<String> {
    let root = project_root?;
    for name in ["RYTER.md", "AGENTS.md"] {
        let p = root.join(name);
        if let Ok(s) = fs::read_to_string(&p) {
            if !s.trim().is_empty() {
                return Some(s);
            }
        }
    }
    None
}

/// `YYYY-MM-DD` in UTC, from the system clock.
pub(crate) fn today_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    civil_date(secs / 86_400)
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Hinnant's algorithm).
pub(crate) fn civil_date(days: u64) -> String {
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The system prompt for a conversation: the prompt, the date, the user's
/// rules, the project's instructions, the skills on offer, and the
/// project's memory. Nothing in it changes with the hat, so a hat switch
/// keeps the provider's cache.
pub fn system(home: &Path, project_root: Option<&Path>, trusted: bool) -> String {
    let mut s = load(home, project_root, trusted);
    s.push('\n');
    // Without it the model dated DECISIONS entries from its training data.
    // Changes once a day, so it costs the prompt cache nothing within a day.
    s.push_str(&format!("\nToday's date (UTC) is {}.\n", today_utc()));
    // The user's rules come before the project's, and say which wins. They
    // are part of the prompt, not a file to go and read: a model told to
    // read one may skip it, and tools can't read Ryter's home folder.
    if let Some(rules) = crate::rules::load(home) {
        s.push_str(&format!(
            "\n## The user's rules (every project)\nThe user's own standing rules, from \
             `{}`. Follow them in every project. Where this project's \
             instructions say otherwise, the project's win. Change them only with \
             `update_rules`, after loading the `rules` skill.\n\n",
            crate::rules::shown(home)
        ));
        s.push_str(&rules);
        s.push('\n');
    }
    if let Some(inst) = load_project_instructions(project_root) {
        s.push_str("\n## Project instructions\n");
        s.push_str(&inst);
        if !inst.ends_with('\n') {
            s.push('\n');
        }
    }
    // Listed, not loaded: the model reads one with `load_skill` when a task
    // needs it. The list changes only when skills do, so the cache holds.
    let catalog = crate::skill::load_catalog(home, project_root, trusted);
    let skills = catalog.for_model();
    if !skills.is_empty() {
        s.push_str(
            "\n## Skills\nInstructions for particular kinds of work. When a task matches \
             one, call load_skill with its name before you start, and follow it.\n",
        );
        for sk in skills {
            s.push_str(&format!("- {}: {}\n", sk.name, sk.description));
        }
    }
    if let Some(mem) = crate::memory::load_project_memory(project_root) {
        s.push_str("\n## Project memory (roadmap, decisions, notes)\n");
        s.push_str("When the user asks why something is the way it is, read this and the files named in it. Update ROADMAP.md and DECISIONS.md as work changes.\n\n");
        s.push_str(&mem);
    }

    s
}

/// Whether `docker` and `podman` are on `path` (the `PATH` variable).
pub fn container_tools_in(path: Option<&std::ffi::OsStr>) -> (bool, bool) {
    let has = |name: &str| {
        path.is_some_and(|p| std::env::split_paths(p).any(|dir| dir.join(name).is_file()))
    };
    (has("docker"), has("podman"))
}

/// What the model is told about the machine it is on: which container tool
/// to use, and what a sandbox profile shuts.
///
/// - **Containers.** With both installed, models picked either, and a
///   project built with one was started with the other. Docker when it is
///   there.
/// - **The sandbox.** A command refused by the profile fails with
///   "Permission denied", and the model reported a broken tool or a missing
///   one. It is told what the profile shuts and where scratch space is.
pub fn machine(
    (docker, podman): (bool, bool),
    sandbox: crate::sandbox::SandboxProfile,
    scratch: Option<&Path>,
) -> String {
    use crate::sandbox::SandboxProfile;
    let mut lines = Vec::new();
    match (docker, podman) {
        (true, true) => lines.push(
            "- Containers: Docker and Podman are both installed. Use Docker (`docker`, \
             `docker compose`) for everything, including what a project's files call \
             `podman`, unless the user asks for Podman."
                .to_string(),
        ),
        (true, false) => lines
            .push("- Containers: Docker is installed (`docker`, `docker compose`).".to_string()),
        (false, true) => lines.push(
            "- Containers: Podman is installed (`podman`, `podman compose`); Docker is not."
                .to_string(),
        ),
        (false, false) => {}
    }
    if sandbox != SandboxProfile::Off {
        let scratch = scratch
            .map(|s| {
                format!(
                    " Keep temporary files in `{}` (`TMPDIR` points there).",
                    s.display()
                )
            })
            .unwrap_or_default();
        let project = if sandbox == SandboxProfile::ReadOnly {
            "They can read the project and can't write it."
        } else {
            "They can write the project and the tools' download caches."
        };
        lines.push(format!(
            "- Sandbox: commands and file edits run under the `{sandbox}` profile, which the \
             user chose. {project} `/tmp` and the rest of the user's folder are shut: a \
             \"Permission denied\" there is the profile, not a broken or missing tool.{scratch} \
             If the work needs more, say so: the user changes the profile in /settings. \
             Docker's own work is not limited by it."
        ));
    }
    if lines.is_empty() {
        String::new()
    } else {
        format!("\n## This machine\n{}\n", lines.join("\n"))
    }
}

/// [`machine`], for the machine and the thread this is called on.
pub fn machine_here() -> String {
    machine(
        container_tools_in(std::env::var_os("PATH").as_deref()),
        crate::sandbox::active(),
        crate::sandbox::scratch().as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// With both installed the model is told to use Docker; with one, which
    /// one; with neither, nothing. Under a sandbox profile it is told what
    /// is shut, so a refusal isn't reported as a missing tool.
    #[test]
    fn the_model_is_told_about_containers_and_the_sandbox() {
        use crate::sandbox::SandboxProfile;
        let both = machine((true, true), SandboxProfile::Off, None);
        assert!(
            both.starts_with("\n## This machine\n- Containers: Docker and Podman"),
            "{both}"
        );
        assert!(
            both.contains("Use Docker") && !both.contains("Sandbox"),
            "{both}"
        );
        let only_podman = machine((false, true), SandboxProfile::Off, None);
        assert!(only_podman.contains("Podman is installed") && !only_podman.contains("Use Docker"));
        assert!(machine((true, false), SandboxProfile::Off, None).contains("Docker is installed"));
        assert_eq!(machine((false, false), SandboxProfile::Off, None), "");
        let boxed = machine(
            (false, false),
            SandboxProfile::Workspace,
            Some(Path::new("/home/u/.ryter/tmp")),
        );
        assert!(
            boxed.contains("under the `workspace` profile")
                && boxed.contains("`/tmp` and the rest of the user's folder are shut")
                && boxed.contains("Keep temporary files in `/home/u/.ryter/tmp`")
                && boxed.contains("They can write the project"),
            "{boxed}"
        );
        let ro = machine((false, false), SandboxProfile::ReadOnly, None);
        assert!(
            ro.contains("can't write it") && !ro.contains("TMPDIR"),
            "{ro}"
        );
    }

    #[test]
    fn container_tools_are_found_on_the_path() {
        let a = TempDir::new().unwrap();
        let b = TempDir::new().unwrap();
        std::fs::write(a.path().join("docker"), "").unwrap();
        std::fs::write(b.path().join("podman"), "").unwrap();
        let path = |dirs: &[&Path]| std::env::join_paths(dirs).unwrap();
        assert_eq!(
            container_tools_in(Some(&path(&[a.path(), b.path()]))),
            (true, true)
        );
        assert_eq!(container_tools_in(Some(&path(&[a.path()]))), (true, false));
        assert_eq!(container_tools_in(Some(&path(&[b.path()]))), (false, true));
        assert_eq!(container_tools_in(None), (false, false));
        // A folder named for the tool is not the tool.
        std::fs::create_dir(a.path().join("podman")).unwrap();
        assert_eq!(container_tools_in(Some(&path(&[a.path()]))), (true, false));
    }

    #[test]
    fn civil_dates_are_right_across_leap_years() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(11_016), "2000-02-29");
        assert_eq!(civil_date(20_717), "2026-09-21");
        assert_eq!(civil_date(20_819), "2027-01-01");
    }

    /// The runtime reads the verdict a review ends with: the prompt must
    /// ask for it in the words the reader takes.
    #[test]
    fn the_prompt_states_the_contract_the_runtime_parses() {
        let body = shipped();
        assert!(body.contains("VERDICT: PASS") && body.contains("VERDICT: FAIL"));
        assert_eq!(
            crate::gate::verdict("- a.rs:3 nit\nVERDICT: PASS"),
            Some(true)
        );
        assert_eq!(
            crate::gate::verdict("- a.rs:3 bug\nVERDICT: FAIL"),
            Some(false)
        );
        for tool in ["present_plan", "request_hat"] {
            assert!(body.contains(tool), "{tool}");
        }
    }

    #[test]
    fn user_override_wins() {
        let home = TempDir::new().unwrap();
        fs::create_dir_all(home.path().join("prompts")).unwrap();
        fs::write(home.path().join("prompts/solo.md"), "OVERRIDE").unwrap();
        assert_eq!(load(home.path(), None, false), "OVERRIDE");
    }

    #[test]
    fn project_override_only_when_trusted() {
        let home = TempDir::new().unwrap();
        let proj = TempDir::new().unwrap();
        fs::create_dir_all(proj.path().join(".ryter/prompts")).unwrap();
        fs::write(proj.path().join(".ryter/prompts/solo.md"), "PROJECT").unwrap();
        assert_eq!(load(home.path(), Some(proj.path()), false), shipped());
        assert_eq!(load(home.path(), Some(proj.path()), true), "PROJECT");
    }

    /// The project's memory is read when it is there, and never created:
    /// files appeared in the user's project on "are you there?".
    #[test]
    fn the_prompt_includes_the_roadmap_and_creates_nothing() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let sys = system(home.path(), Some(cwd.path()), false);
        assert!(!sys.contains("## Project memory (roadmap, decisions, notes)"));
        assert_eq!(fs::read_dir(cwd.path()).unwrap().count(), 0);
        fs::write(
            cwd.path().join("ROADMAP.md"),
            "# Roadmap\n## Now\n- custom flag\n",
        )
        .unwrap();
        let sys = system(home.path(), Some(cwd.path()), false);
        assert!(sys.contains("## Project memory (roadmap, decisions, notes)"));
        assert!(sys.contains("custom flag"));
        assert!(!cwd.path().join("DECISIONS.md").exists());
    }

    #[test]
    fn project_instructions_from_ryter_md() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        fs::write(cwd.path().join("AGENTS.md"), "agents file").unwrap();
        fs::write(cwd.path().join("RYTER.md"), "never invent APIs").unwrap();
        let sys = system(home.path(), Some(cwd.path()), false);
        assert!(sys.contains("## Project instructions"));
        assert!(sys.contains("never invent APIs"));
        assert!(!sys.contains("agents file"));
    }

    /// The user's rules are in the prompt, ahead of the project's, with
    /// which wins said; no file, no section.
    #[test]
    fn the_users_rules_are_in_the_prompt() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        fs::write(cwd.path().join("RYTER.md"), "never invent APIs").unwrap();
        let sys = |home: &Path| system(home, Some(cwd.path()), false);
        assert!(!sys(home.path()).contains("The user's rules"), "no file");
        crate::rules::save(home.path(), "- Use British spelling.").unwrap();
        let sys = sys(home.path());
        let rules = sys
            .find("## The user's rules (every project)")
            .expect("section");
        let project = sys.find("## Project instructions").unwrap();
        assert!(rules < project, "the user's rules come first");
        assert!(sys.contains("- Use British spelling."));
        assert!(sys.contains("the project's win") && sys.contains("`update_rules`"));
    }

    #[test]
    fn agents_md_when_ryter_md_absent() {
        let proj = TempDir::new().unwrap();
        fs::write(proj.path().join("AGENTS.md"), "from agents").unwrap();
        assert_eq!(
            load_project_instructions(Some(proj.path())).as_deref(),
            Some("from agents")
        );
    }
}
