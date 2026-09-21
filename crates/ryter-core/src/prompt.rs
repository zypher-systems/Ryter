//! Prompt files: shipped, user (`~/.ryter/prompts`), project (`.ryter/prompts`).

use std::fs;
use std::path::Path;

use crate::error::Result;
use crate::phase::Phase;
use crate::role::Role;
use crate::session::Session;

const ORCHESTRATOR: &str = include_str!("../../../prompts/orchestrator.md");
const ARCHITECT: &str = include_str!("../../../prompts/architect.md");
const BUILDER: &str = include_str!("../../../prompts/builder.md");
const AUDITOR: &str = include_str!("../../../prompts/auditor.md");
const SOLO: &str = include_str!("../../../prompts/solo.md");

/// Which prompt file to load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    /// User-facing orchestrator.
    Orchestrator,
    /// Architect specialist.
    Architect,
    /// Build specialist.
    Builder,
    /// Audit specialist / merge gate.
    Auditor,
    /// Solo mode: one model, three hats.
    Solo,
}

impl PromptKind {
    /// Filename stem.
    pub fn file_stem(self) -> &'static str {
        match self {
            Self::Orchestrator => "orchestrator",
            Self::Architect => "architect",
            Self::Builder => "builder",
            Self::Auditor => "auditor",
            Self::Solo => "solo",
        }
    }

    /// Shipped default body.
    pub fn shipped(self) -> &'static str {
        match self {
            Self::Orchestrator => ORCHESTRATOR,
            Self::Architect => ARCHITECT,
            Self::Builder => BUILDER,
            Self::Auditor => AUDITOR,
            Self::Solo => SOLO,
        }
    }

    /// Specialist kind for a role (orchestrator is not a specialist prompt).
    pub fn for_role(role: Role) -> Option<Self> {
        match role {
            Role::Orchestrator => Some(Self::Orchestrator),
            Role::Architect => Some(Self::Architect),
            Role::Builder => Some(Self::Builder),
            Role::Auditor => Some(Self::Auditor),
            Role::SoloPlan | Role::SoloBuild | Role::SoloReview => Some(Self::Solo),
        }
    }
}

/// Load a prompt with overrides: project (if trusted) > user home > shipped.
pub fn load(kind: PromptKind, home: &Path, project_root: Option<&Path>, trusted: bool) -> String {
    let name = format!("{}.md", kind.file_stem());
    if trusted {
        if let Some(root) = project_root {
            let p = root.join(".ryter").join("prompts").join(&name);
            if let Ok(s) = fs::read_to_string(&p) {
                return s;
            }
        }
    }
    let user = home.join("prompts").join(&name);
    if let Ok(s) = fs::read_to_string(&user) {
        return s;
    }
    kind.shipped().to_string()
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
fn today_utc() -> String {
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

/// Orchestrator system prompt for this session: body + phase + pass notes.
pub fn orchestrator_system(
    home: &Path,
    project_root: Option<&Path>,
    trusted: bool,
    session: &Session,
) -> Result<String> {
    conversation_system(
        PromptKind::Orchestrator,
        home,
        project_root,
        trusted,
        session,
    )
}

/// System prompt for the conversation the user types into: the crew lead, or
/// solo mode's one model. Both carry project instructions and memory.
pub fn conversation_system(
    kind: PromptKind,
    home: &Path,
    project_root: Option<&Path>,
    trusted: bool,
    session: &Session,
) -> Result<String> {
    // No phase section: the lead routes each task to a role itself, and a
    // "current phase" line only confused the models about what they could do.
    let mut s = load(kind, home, project_root, trusted);
    s.push('\n');
    // Without it the lead dated DECISIONS entries from its training data.
    // Changes once a day, so it costs the prompt cache nothing within a day.
    s.push_str(&format!("\nToday's date (UTC) is {}.\n", today_utc()));
    // The crew keeps its memory in the project; solo mode doesn't create
    // files the user didn't ask for (they appeared on "are you there?").
    if kind == PromptKind::Orchestrator {
        if let Some(root) = project_root {
            let _ = crate::memory::ensure_project_memory(root);
        }
    }
    if let Some(inst) = load_project_instructions(project_root) {
        s.push_str("\n## Project instructions\n");
        s.push_str(&inst);
        if !inst.ends_with('\n') {
            s.push('\n');
        }
    }
    if let Some(mem) = crate::memory::load_project_memory(project_root) {
        s.push_str("\n## Project memory (roadmap, decisions, notes)\n");
        s.push_str("When the user asks why something is the way it is, read this and the files named in it. Update ROADMAP.md and DECISIONS.md as work changes. Do not paste specialist transcripts here.\n\n");
        s.push_str(&mem);
    }

    let mut any = false;
    for phase in [Phase::Plan, Phase::Build, Phase::Audit] {
        let note = session.read_note(phase)?;
        if note.is_empty() {
            continue;
        }
        if !any {
            s.push_str("\n## Pass notes\n");
            any = true;
        }
        s.push_str("### ");
        s.push_str(phase.as_str());
        s.push('\n');
        s.push_str(&note);
        if !note.ends_with('\n') {
            s.push('\n');
        }
    }
    // The latest crew report is not repeated here: it already reaches the
    // orchestrator as a turn in the transcript, and a copy in the system prompt
    // changed the cached prefix after every build.
    Ok(s)
}

/// Fresh-window messages for a specialist (no orchestrator transcript).
pub fn specialist_messages(
    home: &Path,
    project_root: Option<&Path>,
    trusted: bool,
    role: Role,
    pass_note: &str,
    task: &str,
    scope: &[String],
) -> Vec<crate::llm::Message> {
    let kind = PromptKind::for_role(role).unwrap_or(PromptKind::Builder);
    let system = load(kind, home, project_root, trusted);
    let mut user = String::new();
    if let Some(root) = project_root {
        let _ = crate::memory::ensure_project_memory(root);
    }
    if let Some(inst) = load_project_instructions(project_root) {
        user.push_str("Project instructions:\n");
        user.push_str(&inst);
        if !inst.ends_with('\n') {
            user.push('\n');
        }
        user.push('\n');
    }
    // The architect writes project memory, so it reads all of it. Builders and
    // auditors get the design and the decisions about their files: the whole
    // log on every round was most of their fixed cost.
    let memory = if role == Role::Architect {
        crate::memory::load_project_memory(project_root)
            .map(|m| format!("Project memory (you keep ROADMAP.md and DECISIONS.md current):\n{m}"))
    } else {
        crate::memory::load_scoped_memory(project_root, scope)
            .map(|m| format!("Project memory relevant to this task:\n{m}"))
    };
    if let Some(mem) = memory {
        user.push_str(&mem);
        user.push('\n');
    }
    if !pass_note.is_empty() {
        user.push_str("Pass note:\n");
        user.push_str(pass_note);
        user.push_str("\n\n");
    }
    user.push_str("Task:\n");
    user.push_str(task);
    vec![
        crate::llm::Message {
            role: "system".into(),
            content: system,
            tool_call_id: None,
            tool_calls: None,
        },
        crate::llm::Message {
            role: "user".into(),
            content: user,
            tool_call_id: None,
            tool_calls: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase::Phase;
    use crate::session::Session;
    use tempfile::TempDir;

    #[test]
    fn civil_dates_are_right_across_leap_years() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(11_016), "2000-02-29");
        assert_eq!(civil_date(20_717), "2026-09-21");
        assert_eq!(civil_date(20_819), "2027-01-01");
    }

    #[test]
    fn shipped_orchestrator_forbids_writing_source() {
        let body = PromptKind::Orchestrator.shipped();
        assert!(
            body.to_ascii_lowercase()
                .contains("never write product source")
        );
    }

    /// The runtime parses what these roles return. The auditor prompt once
    /// said "return a clear pass or fail" while the parser wanted a first-line
    /// PASS, so good work failed the gate on formatting.
    #[test]
    fn prompts_state_the_contracts_the_runtime_parses() {
        let auditor = PromptKind::Auditor.shipped();
        assert!(auditor.contains("VERDICT: PASS") && auditor.contains("VERDICT: FAIL"));
        assert!(crate::crew::parse_verdict("- a.rs:3 nit\nVERDICT: PASS"));
        assert!(!crate::crew::parse_verdict("- a.rs:3 bug\nVERDICT: FAIL"));
        let builder = PromptKind::Builder.shipped();
        for field in ["STATUS:", "FILES:", "DECISIONS:", "NOTES:"] {
            assert!(builder.contains(field), "builder handback lost {field}");
        }
        // Builders are denied memory files; the prompt must not ask for them.
        assert!(builder.contains("Do not commit"));
        let orchestrator = PromptKind::Orchestrator.shipped();
        assert!(orchestrator.contains("brief") && orchestrator.contains("files"));
    }

    #[test]
    fn user_override_wins() {
        let home = TempDir::new().unwrap();
        fs::create_dir_all(home.path().join("prompts")).unwrap();
        fs::write(home.path().join("prompts/orchestrator.md"), "OVERRIDE ORCH").unwrap();
        let got = load(PromptKind::Orchestrator, home.path(), None, false);
        assert_eq!(got, "OVERRIDE ORCH");
    }

    #[test]
    fn project_override_only_when_trusted() {
        let home = TempDir::new().unwrap();
        let proj = TempDir::new().unwrap();
        fs::create_dir_all(proj.path().join(".ryter/prompts")).unwrap();
        fs::write(
            proj.path().join(".ryter/prompts/builder.md"),
            "PROJECT BUILDER",
        )
        .unwrap();
        let untrusted = load(PromptKind::Builder, home.path(), Some(proj.path()), false);
        assert_eq!(untrusted, PromptKind::Builder.shipped());
        let trusted = load(PromptKind::Builder, home.path(), Some(proj.path()), true);
        assert_eq!(trusted, "PROJECT BUILDER");
    }

    #[test]
    fn orchestrator_system_includes_phase_and_notes() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let mut s = Session::create(
            home.path(),
            cwd.path(),
            Phase::Plan,
            "spacexai".into(),
            "grok-4.6".into(),
        )
        .unwrap();
        s.handoff(Phase::Build, "we need auth", None).unwrap();
        let sys = orchestrator_system(home.path(), None, false, &s).unwrap();
        // Phases are gone from what the lead is told.
        assert!(!sys.contains("## Current phase"));
        assert!(!sys.contains("Allowed specialists"));
        // Notes left by older sessions still reach it.
        assert!(sys.contains("we need auth"));
        assert_eq!(s.transcript.len(), 0);
    }

    #[test]
    fn orchestrator_system_includes_roadmap() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        fs::write(
            cwd.path().join("ROADMAP.md"),
            "# Roadmap\n## Now\n- custom flag\n",
        )
        .unwrap();
        let s = Session::create(
            home.path(),
            cwd.path(),
            Phase::Build,
            "spacexai".into(),
            "grok-4.6".into(),
        )
        .unwrap();
        let sys = orchestrator_system(home.path(), Some(cwd.path()), false, &s).unwrap();
        assert!(sys.contains("Project memory"));
        assert!(sys.contains("custom flag"));
        assert!(cwd.path().join("DECISIONS.md").is_file());
    }

    #[test]
    fn project_instructions_from_ryter_md() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        fs::write(cwd.path().join("AGENTS.md"), "agents file").unwrap();
        fs::write(cwd.path().join("RYTER.md"), "never invent APIs").unwrap();
        let s = Session::create(
            home.path(),
            cwd.path(),
            Phase::Build,
            "spacexai".into(),
            "grok-4.6".into(),
        )
        .unwrap();
        let sys = orchestrator_system(home.path(), Some(cwd.path()), false, &s).unwrap();
        assert!(sys.contains("## Project instructions"));
        assert!(sys.contains("never invent APIs"));
        assert!(!sys.contains("agents file"));
        let msgs = specialist_messages(
            home.path(),
            Some(cwd.path()),
            false,
            Role::Builder,
            "",
            "add a flag",
            &[],
        );
        assert!(msgs[1].content.contains("never invent APIs"));
    }

    #[test]
    fn agents_md_when_ryter_md_absent() {
        let home = TempDir::new().unwrap();
        let proj = TempDir::new().unwrap();
        fs::write(proj.path().join("AGENTS.md"), "from agents").unwrap();
        assert_eq!(
            load_project_instructions(Some(proj.path())).as_deref(),
            Some("from agents")
        );
        drop(home);
    }

    #[test]
    fn specialist_messages_are_a_fresh_window() {
        let home = TempDir::new().unwrap();
        let msgs = specialist_messages(
            home.path(),
            None,
            false,
            Role::Builder,
            "brief",
            "add a flag",
            &[],
        );
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        assert!(msgs[1].content.contains("add a flag"));
        assert!(msgs[1].content.contains("brief"));
    }
}
