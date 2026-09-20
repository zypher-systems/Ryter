//! Slash command dispatch.

use ryter_core::Phase;
use std::str::FromStr;

use crate::view::{ChoiceKind, LogLine, Overlay, SlashMenu, View, choice_options, filter_slash};

/// Side effect the event loop must handle.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Nothing extra.
    None,
    /// Leave the TUI.
    Quit,
    /// Start a new session.
    New,
    /// Submit composer to the orchestrator.
    Submit(String),
    /// Phase handoff with pass note.
    Handoff { to: Phase, note: String },
    /// Toggle auditor gate.
    SetAuditor(bool),
    /// Switch the TUI theme.
    SetTheme(String),
    /// Ask the worker for `/context`.
    Context,
    /// Force a compact pass.
    Compact,
    /// Run `ryter doctor` in-process.
    Doctor,
    /// Switch the active connection.
    UseConnection(String),
    /// Persist an API key for `name`.
    SetKey { name: String, key: String },
    /// Switch model id on the current connection.
    SetModel(String),
    /// Open the provider picker overlay.
    OpenProvider,
    /// Open the model picker overlay (worker should list models).
    OpenModels,
    /// Tool permission: `true` = always approve Ask.
    SetTools { always: bool },
    /// Open `/crew`.
    OpenCrew,
    /// Assign a specialist model (and persist).
    SetCrewRole {
        role: String,
        connection: String,
        model: String,
    },
    /// Save the live crew as a named preset.
    SaveCrewPreset(String),
    /// Load a named crew preset.
    LoadCrewPreset(String),
    /// List models for a specialist assignment (all keyed connections).
    ListCrewModels { role: String },
    /// Open the load-preset list.
    OpenCrewPresets,
    /// Stop the in-flight turn.
    Cancel,
    /// Open `/resume` picker.
    OpenResume,
    /// Load a saved session.
    Resume(String),
    /// Open `/delete` session picker.
    OpenDeleteSession,
    /// Delete a saved session.
    DeleteSession(String),
    /// Set the current session title.
    RenameSession(String),
    /// Open `/agents`.
    OpenAgents,
    /// Kill a running specialist.
    KillAgent(String),
    /// Open `/mcp`.
    OpenMcp,
    /// Persist MCP config and reconnect outbound servers.
    SaveMcp,
    /// Bind TCP inbound on the running TUI.
    McpListenTcp,
    /// Open `/skills`.
    OpenSkills,
    /// Write a skill stub and reload the catalog.
    WriteSkill {
        /// Slash name.
        name: String,
        /// One-line description.
        description: String,
    },
    /// Write a user-command stub.
    WriteCommand {
        /// Slash name.
        name: String,
    },
    /// Delete a skill or command file.
    RemoveCatalog(std::path::PathBuf),
    /// Open `/hooks`.
    OpenHooks,
    /// Persist hooks and reload the agent set.
    SaveHooks,
    /// Open `/spend` table.
    OpenSpend,
    /// Open `/settings`.
    OpenSettings,
    /// Persist settings.toml and apply live knobs.
    SaveSettings,
    /// Reply to a permission prompt.
    PermissionReply(ryter_core::Permission),
    /// Reply to `ask_user`.
    AskUserReply(String),
    /// Trust or skip project `.ryter/`.
    TrustProject(bool),
}

/// Known commands and a one-line help blurb.
pub fn help_text() -> &'static str {
    "/quit  /new  /resume  /rename  /delete  /agents  /spend  /settings  /provider  /models  /crew  /mcp  /skills  /hooks  /handoff  /phase  /plan  /architect  /build  /audit  /auditor  /tools  /theme  /context  /compact  /doctor  /cancel  /help"
}

/// Apply a finished slash line (without requiring the leading slash).
pub fn run_slash(view: &mut View, raw: &str) -> Action {
    let raw = raw.trim().trim_start_matches('/');
    let (cmd, rest) = raw
        .split_once(char::is_whitespace)
        .map(|(a, b)| (a, b.trim()))
        .unwrap_or((raw, ""));
    match cmd {
        "quit" | "exit" => Action::Quit,
        "help" => {
            view.lines.push(LogLine::System(help_text().into()));
            Action::None
        }
        "new" => Action::New,
        "resume" | "sessions" => {
            if rest.is_empty() {
                Action::OpenResume
            } else {
                Action::Resume(rest.to_string())
            }
        }
        "rename" => {
            if rest.is_empty() {
                view.lines
                    .push(LogLine::System("usage: /rename <title>".into()));
                Action::None
            } else {
                Action::RenameSession(rest.to_string())
            }
        }
        "delete" => {
            if rest.is_empty() {
                Action::OpenDeleteSession
            } else {
                Action::DeleteSession(rest.to_string())
            }
        }
        "agents" => Action::OpenAgents,
        "spend" => Action::OpenSpend,
        "settings" => Action::OpenSettings,
        "crew" => Action::OpenCrew,
        "cancel" => Action::Cancel,
        "connections" | "provider" | "providers" => {
            if rest.is_empty() {
                Action::OpenProvider
            } else {
                connections_cmd(view, rest)
            }
        }
        "models" | "model" => {
            if rest.is_empty() {
                Action::OpenModels
            } else {
                Action::SetModel(rest.to_string())
            }
        }
        "theme" => {
            if rest.is_empty() {
                open_choice(view, ChoiceKind::Theme, "theme");
                Action::None
            } else {
                Action::SetTheme(rest.to_string())
            }
        }
        "skills" => {
            if rest.is_empty() {
                Action::OpenSkills
            } else {
                let (name, args) = rest
                    .split_once(char::is_whitespace)
                    .map(|(n, a)| (n, a.trim()))
                    .unwrap_or((rest, ""));
                if let Some(expanded) = view.catalog.expand(name, args) {
                    let shown = if args.is_empty() {
                        format!("/{name}")
                    } else {
                        format!("/{name} {args}")
                    };
                    view.lines.push(LogLine::User(shown));
                    view.busy = true;
                    Action::Submit(expanded)
                } else {
                    Action::OpenSkills
                }
            }
        }
        "hooks" => Action::OpenHooks,
        "mcp" => Action::OpenMcp,
        "context" => Action::Context,
        "compact" => Action::Compact,
        "doctor" => Action::Doctor,
        "auditor" => match rest {
            "on" => {
                view.auditor_on = true;
                Action::SetAuditor(true)
            }
            "off" => {
                view.auditor_on = false;
                Action::SetAuditor(false)
            }
            _ => {
                open_choice(view, ChoiceKind::Auditor, "auditor");
                Action::None
            }
        },
        "tools" | "permissions" => match rest {
            "ask" => Action::SetTools { always: false },
            "always" | "auto" | "on" => Action::SetTools { always: true },
            _ => {
                open_choice(view, ChoiceKind::Tools, "tools");
                Action::None
            }
        },
        "ask" => Action::SetTools { always: false },
        "auto" | "always" => Action::SetTools { always: true },
        "phase" => {
            open_choice(view, ChoiceKind::Phase, "phase");
            Action::None
        }
        "plan" | "architect" | "build" | "audit" => {
            if let Ok(p) = Phase::from_str(cmd) {
                begin_handoff(view, p);
            }
            Action::None
        }
        "handoff" => {
            let to = if rest.is_empty() || rest == "back" {
                None
            } else {
                Phase::from_str(rest.split_whitespace().next().unwrap_or("")).ok()
            };
            if let Some(p) = to {
                begin_handoff(view, p);
                Action::None
            } else if rest == "back" {
                if let Some(prev) = previous(view.phase) {
                    begin_handoff(view, prev);
                } else {
                    view.lines.push(LogLine::System("already at plan".into()));
                }
                Action::None
            } else {
                view.lines.push(LogLine::System(
                    "usage: /handoff plan|architect|build|audit|back".into(),
                ));
                Action::None
            }
        }
        other => {
            if let Some(expanded) = view.catalog.expand(other, rest) {
                let shown = if rest.is_empty() {
                    format!("/{other}")
                } else {
                    format!("/{other} {rest}")
                };
                view.lines.push(LogLine::User(shown));
                view.busy = true;
                Action::Submit(expanded)
            } else {
                view.lines
                    .push(LogLine::System(format!("unknown command /{other}")));
                Action::None
            }
        }
    }
}

fn connections_cmd(view: &mut View, rest: &str) -> Action {
    let rest = rest.trim();
    if rest.is_empty() {
        if view.connections.is_empty() {
            view.lines.push(LogLine::System(
                "no connections loaded  ·  /connections use spacexai|openrouter".into(),
            ));
        } else {
            for c in &view.connections {
                let mark = if c.name == view.connection {
                    "●"
                } else {
                    " "
                };
                let key = if c.has_key { "key set" } else { "key missing" };
                view.lines.push(LogLine::System(format!(
                    "{mark} {}  {}  {}  {key}",
                    c.name, c.kind, c.model
                )));
            }
            view.lines.push(LogLine::System(
                "usage: /provider  ·  /provider use <name>  ·  /provider set-key [name]".into(),
            ));
        }
        return Action::None;
    }
    let (sub, arg) = rest
        .split_once(char::is_whitespace)
        .map(|(a, b)| (a, b.trim()))
        .unwrap_or((rest, ""));
    match sub {
        "use" => {
            if arg.is_empty() {
                view.lines
                    .push(LogLine::System("usage: /provider use <name>".into()));
                Action::None
            } else {
                Action::UseConnection(arg.to_string())
            }
        }
        "set-key" | "setkey" | "key" => {
            let name = if arg.is_empty() {
                view.connection.clone()
            } else {
                arg.to_string()
            };
            begin_set_key(view, name);
            Action::None
        }
        other => {
            // Bare name: treat as `use`.
            Action::UseConnection(other.to_string())
        }
    }
}

fn begin_set_key(view: &mut View, name: String) {
    view.secret_for = Some(name.clone());
    view.secret_buf.clear();
    view.composer.clear();
    view.slash = None;
    view.lines.push(LogLine::System(format!(
        "set-key {name}  · paste the key, Enter to save (not shown)"
    )));
}

fn open_choice(view: &mut View, kind: ChoiceKind, title: &str) {
    let options = choice_options(kind, view);
    let current = match kind {
        ChoiceKind::Auditor => {
            if view.auditor_on {
                "on"
            } else {
                "off"
            }
        }
        ChoiceKind::Tools => view.perm_mode.as_str(),
        ChoiceKind::Theme => view.theme_name.as_str(),
        ChoiceKind::Phase => view.phase.as_str(),
        ChoiceKind::CrewPreset | ChoiceKind::Resume | ChoiceKind::DeleteSession => "",
        ChoiceKind::Trust => "",
        ChoiceKind::Sandbox => view.sandbox_profile.as_str(),
    };
    let selected = options.iter().position(|o| o.id == current).unwrap_or(0);
    view.overlay = Some(Overlay::Choice {
        title: title.into(),
        kind,
        options,
        selected,
        filter: String::new(),
    });
    view.slash = None;
    view.composer.clear();
}

fn begin_handoff(view: &mut View, to: Phase) {
    view.handoff_to = Some(to);
    view.composer.clear();
    view.slash = None;
    view.lines.push(LogLine::System(format!(
        "handoff → {to}  (edit note, Enter to accept)"
    )));
}

fn previous(phase: Phase) -> Option<Phase> {
    match phase {
        Phase::Plan => None,
        Phase::Architect => Some(Phase::Plan),
        Phase::Build => Some(Phase::Architect),
        Phase::Audit => Some(Phase::Build),
    }
}

/// After a keystroke, keep slash menu in sync with composer.
pub fn refresh_slash(view: &mut View) {
    if let Some(rest) = view.composer.strip_prefix('/') {
        if rest.contains(char::is_whitespace) {
            view.slash = None;
            return;
        }
        let extra = view.catalog.names();
        let matches = filter_slash(rest, &extra);
        let selected = view
            .slash
            .as_ref()
            .map(|s| s.selected.min(matches.len().saturating_sub(1)))
            .unwrap_or(0);
        view.slash = Some(SlashMenu {
            filter: rest.to_string(),
            matches,
            selected,
        });
    } else {
        view.slash = None;
    }
}

/// Handle Enter in the composer.
pub fn submit(view: &mut View) -> Action {
    if view.overlay.is_none() {
        if let Some(slash) = view.slash.as_ref() {
            if let Some(name) = slash.matches.get(slash.selected).cloned() {
                view.slash = None;
                view.composer.clear();
                return run_slash(view, &name);
            }
        }
    }
    if let Some(name) = view.secret_for.take() {
        let key = std::mem::take(&mut view.secret_buf);
        view.slash = None;
        if key.trim().is_empty() {
            view.lines.push(LogLine::System("set-key cancelled".into()));
            return Action::None;
        }
        return Action::SetKey { name, key };
    }
    if let Some(to) = view.handoff_to.take() {
        let note = std::mem::take(&mut view.composer);
        view.slash = None;
        return Action::Handoff { to, note };
    }
    let text = std::mem::take(&mut view.composer);
    view.slash = None;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Action::None;
    }
    if let Some(cmd) = trimmed.strip_prefix('/') {
        return run_slash(view, cmd);
    }
    view.lines.push(LogLine::User(trimmed.to_string()));
    view.busy = true;
    Action::Submit(trimmed.to_string())
}

/// Apply a streamed assistant delta.
pub fn on_token(view: &mut View, text: &str) {
    match view.lines.last_mut() {
        Some(LogLine::Assistant(buf)) => buf.push_str(text),
        _ => view.lines.push(LogLine::Assistant(text.to_string())),
    }
}
