//! Ratatui frontend for Ryter.

#![forbid(unsafe_code)]

mod chat;
mod commands;
mod draw;
mod run;
mod theme;
mod view;

pub use draw::render_to_string;
pub use run::{TuiOpts, run};
pub use view::View;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{Action, run_slash};
    use crate::view::{CrewRow, LogLine};
    use ryter_core::Phase;

    fn sample() -> View {
        let mut v = View::new(
            Phase::Build,
            "spacexai".into(),
            "grok-4.6".into(),
            "~/workspace/ryter".into(),
        );
        v.git_branch = Some("main".into());
        v.spend = Some(0.42);
        v.ctx_tokens = Some(12_000);
        v.ctx_window = Some(500_000);
        v.price_label = "$2/M input / $6/M output".into();
        v.has_key = true;
        v.lines.push(LogLine::User("add a --json flag".into()));
        v.lines.push(LogLine::Assistant(
            "I'll put it on the CLI.\n\n- parse args\n- write help\n".into(),
        ));
        v.lines.push(LogLine::Tool("read Cargo.toml".into()));
        v.crew.push(CrewRow {
            id: "c1".into(),
            role: "builder".into(),
            label: "cli flags".into(),
            spend: Some(0.02),
            status: "running".into(),
        });
        v.todos.push(crate::view::TodoRow {
            title: "add --json flag".into(),
            status: "pending".into(),
        });
        v
    }

    #[test]
    fn snapshot_80x24_has_quiet_chrome() {
        let s = render_to_string(&sample(), 80, 24);
        assert!(s.contains("ryter"), "{s}");
        assert!(s.contains("orchestrator"), "{s}");
        assert!(s.contains("build"), "{s}");
        assert!(s.contains("spacexai"), "{s}");
        assert!(s.contains("grok-4.6"), "{s}");
        assert!(s.contains("$0.42"), "{s}");
        assert!(s.contains("12k/500k"), "{s}");
        assert!(s.contains("spend"), "{s}");
        assert!(s.contains("provider"), "{s}");
        assert!(s.contains("~/workspace/ryter"), "{s}");
        assert!(s.contains("main"), "{s}");
        assert!(s.contains("tools"), "{s}");
        assert!(s.contains('›'), "{s}");
        assert!(!s.contains('┌'), "no box corners:\n{s}");
        assert!(!s.contains('┐'), "no box corners:\n{s}");
        assert!(!s.contains('└'), "no box corners:\n{s}");
        assert!(!s.contains('┘'), "no box corners:\n{s}");
    }

    #[test]
    fn snapshot_120x40_keeps_spend_and_crew() {
        let s = render_to_string(&sample(), 120, 40);
        assert!(s.contains("$0.42"), "{s}");
        assert!(s.contains("cli flags"), "{s}");
        assert!(s.contains("builder"), "{s}");
        assert!(s.contains("tasks"), "{s}");
        assert!(s.contains("add --json"), "{s}");
        assert!(s.contains("$2/M input"), "{s}");
        assert!(!s.contains('╔'), "{s}");
    }

    #[test]
    fn slash_help_and_handoff() {
        let mut v = View::new(Phase::Plan, "openrouter".into(), "x".into(), "p".into());
        assert_eq!(run_slash(&mut v, "help"), Action::None);
        assert!(matches!(
            v.lines.last(),
            Some(LogLine::System(s)) if s.contains("/quit")
        ));
        assert_eq!(run_slash(&mut v, "quit"), Action::Quit);
        let mut v = View::new(Phase::Build, "x".into(), "x".into(), "p".into());
        assert_eq!(run_slash(&mut v, "auditor off"), Action::SetAuditor(false));
        assert_eq!(run_slash(&mut v, "auditor on"), Action::SetAuditor(true));
        let mut v = View::new(Phase::Plan, "openrouter".into(), "x".into(), "p".into());
        run_slash(&mut v, "handoff architect");
        assert_eq!(v.handoff_to, Some(Phase::Architect));
        v.composer = "we need a flag".into();
        let act = crate::commands::submit(&mut v);
        assert_eq!(
            act,
            Action::Handoff {
                to: Phase::Architect,
                note: "we need a flag".into()
            }
        );
    }

    #[test]
    fn slash_skills_and_theme() {
        let mut v = View::new(Phase::Build, "x".into(), "x".into(), "p".into());
        v.catalog.skills.push(ryter_core::Skill {
            name: "review".into(),
            description: "Review the diff".into(),
            user_invocable: true,
            body: "Look at git diff.".into(),
            source: std::path::PathBuf::from("/tmp/SKILL.md"),
        });
        assert!(crate::view::filter_slash("re", &v.catalog.names()).contains(&"review".into()));
        assert_eq!(run_slash(&mut v, "skills"), Action::OpenSkills);
        assert_eq!(run_slash(&mut v, "hooks"), Action::OpenHooks);
        let act = run_slash(&mut v, "review src/lib.rs");
        assert!(
            matches!(act, Action::Submit(s) if s.contains("Look at git diff.") && s.contains("src/lib.rs"))
        );
        assert_eq!(
            run_slash(&mut v, "theme dark"),
            Action::SetTheme("dark".into())
        );
        assert_eq!(run_slash(&mut v, "context"), Action::Context);
        assert_eq!(run_slash(&mut v, "compact"), Action::Compact);
        assert_eq!(run_slash(&mut v, "doctor"), Action::Doctor);
        assert_eq!(
            run_slash(&mut v, "connections use spacexai"),
            Action::UseConnection("spacexai".into())
        );
        assert_eq!(run_slash(&mut v, "provider"), Action::OpenProvider);
        assert_eq!(run_slash(&mut v, "crew"), Action::OpenCrew);
        assert_eq!(run_slash(&mut v, "mcp"), Action::OpenMcp);
        assert_eq!(run_slash(&mut v, "cancel"), Action::Cancel);
        assert_eq!(run_slash(&mut v, "resume"), Action::OpenResume);
        assert_eq!(run_slash(&mut v, "agents"), Action::OpenAgents);
        assert_eq!(
            run_slash(&mut v, "rename web app"),
            Action::RenameSession("web app".into())
        );
        assert_eq!(run_slash(&mut v, "models"), Action::OpenModels);
        assert_eq!(run_slash(&mut v, "spend"), Action::OpenSpend);
        assert_eq!(run_slash(&mut v, "settings"), Action::OpenSettings);
        assert_eq!(
            run_slash(&mut v, "always"),
            Action::SetTools { always: true }
        );
        assert_eq!(run_slash(&mut v, "ask"), Action::SetTools { always: false });
        assert_eq!(run_slash(&mut v, "auditor"), Action::None);
        assert!(matches!(
            v.overlay,
            Some(crate::view::Overlay::Choice {
                kind: crate::view::ChoiceKind::Auditor,
                ..
            })
        ));
        run_slash(&mut v, "connections set-key openrouter");
        assert_eq!(v.secret_for.as_deref(), Some("openrouter"));
    }

    #[test]
    fn slash_enter_runs_highlighted_command() {
        let mut v = View::new(
            Phase::Build,
            "spacexai".into(),
            "grok-4.6".into(),
            "p".into(),
        );
        v.composer = "/".into();
        crate::commands::refresh_slash(&mut v);
        let slash = v.slash.as_mut().expect("slash menu");
        let idx = slash
            .matches
            .iter()
            .position(|m| m == "provider")
            .expect("provider in menu");
        slash.selected = idx;
        assert_eq!(crate::commands::submit(&mut v), Action::OpenProvider);
        assert!(v.composer.is_empty());
        assert!(v.slash.is_none());
    }

    #[test]
    fn crew_overlay_defaults_every_role_to_orchestrator() {
        let mut v = sample();
        v.overlay = Some(crate::view::Overlay::Crew {
            selected: 0,
            save_buf: None,
        });
        let s = render_to_string(&v, 80, 24);
        assert!(s.contains("planner"), "{s}");
        assert!(s.contains("architect"), "{s}");
        assert!(s.contains("builder"), "{s}");
        assert!(s.contains("auditor"), "{s}");
        let defaults = s.matches("default (grok-4.6)").count();
        assert_eq!(defaults, 4, "factory crew must follow orchestrator:\n{s}");
        assert!(
            !s.contains("claude-sonnet"),
            "no factory OpenRouter split:\n{s}"
        );
    }

    #[test]
    fn crew_model_picker_leads_with_default_row() {
        let mut v = sample();
        v.overlay = Some(crate::view::Overlay::Model {
            filter: String::new(),
            selected: 0,
            items: vec![
                ryter_core::ModelInfo {
                    id: String::new(),
                    context_length: None,
                    input_per_million: None,
                    output_per_million: None,
                    connection: Some("spacexai".into()),
                },
                ryter_core::ModelInfo {
                    id: "anthropic/claude-sonnet-4.6".into(),
                    context_length: Some(200_000),
                    input_per_million: Some(3.0),
                    output_per_million: Some(15.0),
                    connection: Some("openrouter".into()),
                },
            ],
            loading: false,
            assign_role: Some("planner".into()),
        });
        let s = render_to_string(&v, 80, 24);
        assert!(s.contains("default (grok-4.6)"), "{s}");
        assert!(s.contains("claude-sonnet-4.6"), "{s}");
        let default_at = s.find("default (grok-4.6)").expect("default row");
        let claude_at = s.find("claude-sonnet-4.6").expect("assigned row");
        assert!(default_at < claude_at, "default row must be first:\n{s}");
    }

    #[test]
    fn permission_and_spend_overlays_are_quiet() {
        let mut v = sample();
        v.overlay = Some(crate::view::Overlay::Permission {
            tool: "bash".into(),
            summary: "rm -rf doomed".into(),
        });
        let s = render_to_string(&v, 80, 24);
        assert!(s.contains("permission"), "{s}");
        assert!(s.contains("y · allow"), "{s}");
        assert!(s.contains("rm -rf doomed"), "{s}");
        assert!(!s.contains('┌'), "{s}");
        v.spend_by_role.insert("orchestrator".into(), 0.12);
        v.spend_by_conn.insert("spacexai".into(), 0.12);
        v.overlay = Some(crate::view::Overlay::Spend);
        let s = render_to_string(&v, 80, 24);
        assert!(s.contains("spend"), "{s}");
        assert!(s.contains("orchestrator"), "{s}");
        assert!(s.contains("spacexai"), "{s}");
        v.overlay = Some(crate::view::Overlay::Settings {
            selected: 0,
            edit: None,
        });
        let s = render_to_string(&v, 80, 24);
        assert!(s.contains("settings"), "{s}");
        assert!(s.contains("budget"), "{s}");
        assert!(s.contains("sandbox"), "{s}");
        assert!(s.contains("web"), "{s}");
    }
}
