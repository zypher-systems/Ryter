//! Commands as data (`R-PAL-19`). `/help` and the palette are generated from here.

use crate::action::{Action, PanelId, SessionsMode};
use crate::panel::modal::Confirm;
use crate::view::View;

/// Palette category, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    /// Session lifecycle.
    Session,
    /// Model & providers.
    Model,
    /// Agents & phase.
    Agents,
    /// Context & spend.
    Context,
    /// Configuration.
    Config,
    /// Extensions.
    Extensions,
    /// Help & diagnostics.
    Help,
    /// Skills and user commands.
    User,
}

impl Category {
    /// All categories in order.
    pub const ALL: [Category; 8] = [
        Category::Session,
        Category::Model,
        Category::Agents,
        Category::Context,
        Category::Config,
        Category::Extensions,
        Category::Help,
        Category::User,
    ];

    /// Uppercase header text.
    pub fn title(self) -> &'static str {
        match self {
            Category::Session => "SESSION",
            Category::Model => "MODEL & PROVIDERS",
            Category::Agents => "AGENTS & PHASE",
            Category::Context => "CONTEXT",
            Category::Config => "CONFIGURATION",
            Category::Extensions => "EXTENSIONS",
            Category::Help => "HELP & DIAGNOSTICS",
            Category::User => "YOUR COMMANDS",
        }
    }
}

/// One built-in command.
pub struct CommandSpec {
    /// Name without the slash.
    pub name: &'static str,
    /// Matchable and runnable alternates.
    pub aliases: &'static [&'static str],
    /// Group.
    pub category: Category,
    /// One line.
    pub description: &'static str,
    /// Argument hint.
    pub usage: Option<&'static str>,
    /// Shows `▸` and supports `→`.
    pub opens_panel: bool,
    /// Keybinding column.
    pub keybinding: Option<&'static str>,
    /// Matchable but not listed.
    pub hidden: bool,
    /// Runner: receives the argument tail.
    pub run: fn(&mut View, &str) -> Action,
}

impl std::fmt::Debug for CommandSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "/{}", self.name)
    }
}

#[allow(clippy::too_many_arguments)]
const fn spec(
    name: &'static str,
    aliases: &'static [&'static str],
    category: Category,
    description: &'static str,
    usage: Option<&'static str>,
    opens_panel: bool,
    keybinding: Option<&'static str>,
    hidden: bool,
    run: fn(&mut View, &str) -> Action,
) -> CommandSpec {
    CommandSpec {
        name,
        aliases,
        category,
        description,
        usage,
        opens_panel,
        keybinding,
        hidden,
        run,
    }
}

/// The inventory (§10.4). Every 0.1 function survives.
pub const COMMANDS: &[CommandSpec] = &[
    // Session
    spec(
        "new",
        &[],
        Category::Session,
        "Start a fresh session",
        None,
        false,
        None,
        false,
        run_new,
    ),
    spec(
        "sessions",
        &["resume"],
        Category::Session,
        "Browse, resume, rename, or delete sessions",
        Some("[id]"),
        true,
        None,
        false,
        run_sessions,
    ),
    spec(
        "rename",
        &[],
        Category::Session,
        "Set the session title",
        Some("<title>"),
        true,
        None,
        false,
        run_rename,
    ),
    spec(
        "delete",
        &[],
        Category::Session,
        "Delete a saved session",
        Some("[id]"),
        true,
        None,
        false,
        run_delete,
    ),
    spec(
        "quit",
        &["exit"],
        Category::Session,
        "Leave Ryter",
        None,
        false,
        Some("Ctrl+D"),
        false,
        |_, _| Action::Quit,
    ),
    // Model & providers
    spec(
        "models",
        &["model"],
        Category::Model,
        "Switch the lead's model",
        Some("[id]"),
        true,
        None,
        false,
        run_models,
    ),
    spec(
        "provider",
        &["providers", "connections"],
        Category::Model,
        "Switch or add a connection, set API keys",
        Some("[use <name> | set-key [name]]"),
        true,
        None,
        false,
        run_provider,
    ),
    spec(
        "crew",
        &[],
        Category::Model,
        "Crew mode: a lead, an architect, parallel builders, independent auditors",
        None,
        true,
        None,
        false,
        run_crew,
    ),
    spec(
        "solo",
        &["normal"],
        Category::Model,
        "Leave crew mode: one model, Tab between build, plan, and review",
        None,
        false,
        None,
        false,
        |_, _| Action::SetMode(ryter_core::Role::SoloBuild),
    ),
    spec(
        "build",
        &[],
        Category::Model,
        "Build hat: make changes in your files",
        None,
        false,
        None,
        false,
        |_, _| Action::SetMode(ryter_core::Role::SoloBuild),
    ),
    spec(
        "plan",
        &[],
        Category::Model,
        "Plan hat: read and propose, change nothing",
        None,
        false,
        None,
        false,
        |_, _| Action::SetMode(ryter_core::Role::SoloPlan),
    ),
    spec(
        "review",
        &[],
        Category::Model,
        "Review hat: run the tests and critique what changed",
        None,
        false,
        None,
        false,
        |_, _| Action::SetMode(ryter_core::Role::SoloReview),
    ),
    spec(
        "undo",
        &[],
        Category::Session,
        "Put your files back as they were before the last build turn",
        None,
        false,
        None,
        false,
        |_, _| Action::Undo,
    ),
    spec(
        "changes",
        &["diff"],
        Category::Session,
        "What changed, file by file, with diffs; undo one file",
        None,
        true,
        None,
        false,
        |_, _| Action::OpenPanel(PanelId::Changes),
    ),
    spec(
        "commit",
        &[],
        Category::Session,
        "Commit your changes with a drafted message and a receipt",
        None,
        true,
        None,
        false,
        |_, _| Action::OpenPanel(PanelId::Commit),
    ),
    // Agents & phase
    spec(
        "agents",
        &[],
        Category::Agents,
        "Running specialists; kill one",
        None,
        true,
        None,
        false,
        |_, _| Action::OpenPanel(PanelId::Agents),
    ),
    spec(
        "cancel",
        &[],
        Category::Agents,
        "Stop the in-flight turn",
        None,
        false,
        Some("Esc"),
        false,
        |_, _| Action::Cancel,
    ),
    // Context
    spec(
        "context",
        &[],
        Category::Context,
        "Inspect window use, message counts, largest contributors",
        None,
        true,
        None,
        false,
        |_, _| Action::Many(vec![Action::Context, Action::OpenPanel(PanelId::Context)]),
    ),
    spec(
        "compact",
        &[],
        Category::Context,
        "Summarize and shrink the transcript",
        None,
        false,
        None,
        false,
        |_, _| Action::Compact,
    ),
    spec(
        "spend",
        &[],
        Category::Context,
        "Spend by role, connection, and turn",
        None,
        true,
        None,
        false,
        |_, _| Action::OpenPanel(PanelId::Spend),
    ),
    spec(
        "budget",
        &[],
        Category::Context,
        "Cap this session's spend, raise the cap, or turn it off",
        Some("[amount|+amount|off]"),
        true,
        None,
        false,
        run_budget,
    ),
    // Configuration
    spec(
        "settings",
        &[],
        Category::Config,
        "Budget, warn, max crew, sandbox, inbound MCP, web",
        None,
        true,
        None,
        false,
        |_, _| Action::OpenPanel(PanelId::Settings),
    ),
    spec(
        "theme",
        &[],
        Category::Config,
        "Switch and preview themes",
        Some("[name]"),
        true,
        None,
        false,
        run_theme,
    ),
    spec(
        "tools",
        &["permissions"],
        Category::Config,
        "Tool permission mode",
        Some("[ask|always]"),
        true,
        None,
        false,
        run_tools,
    ),
    spec(
        "auditor",
        &[],
        Category::Config,
        "Auditor gate on or off",
        Some("[on|off]"),
        true,
        None,
        false,
        run_auditor,
    ),
    spec(
        "ask",
        &[],
        Category::Config,
        "Tools: confirm destructive calls",
        None,
        false,
        None,
        true,
        |_, _| Action::SetTools { always: false },
    ),
    spec(
        "auto",
        &[],
        Category::Config,
        "Tools: auto-approve",
        None,
        false,
        None,
        true,
        |_, _| Action::SetTools { always: true },
    ),
    spec(
        "always",
        &[],
        Category::Config,
        "Tools: auto-approve",
        None,
        false,
        None,
        true,
        |_, _| Action::SetTools { always: true },
    ),
    // Extensions
    spec(
        "mcp",
        &[],
        Category::Extensions,
        "Inbound and outbound MCP servers, tokens, client links",
        None,
        true,
        None,
        false,
        |_, _| Action::OpenPanel(PanelId::Mcp),
    ),
    spec(
        "skills",
        &[],
        Category::Extensions,
        "Browse, run, create, and remove skills and user commands",
        Some("[name args]"),
        true,
        None,
        false,
        run_skills,
    ),
    spec(
        "hooks",
        &[],
        Category::Extensions,
        "Lifecycle hooks",
        None,
        true,
        None,
        false,
        |_, _| Action::OpenPanel(PanelId::Hooks),
    ),
    // Help & diagnostics
    spec(
        "help",
        &["?"],
        Category::Help,
        "Keymap and command reference",
        None,
        true,
        Some("F1"),
        false,
        |_, _| Action::OpenPanel(PanelId::Help),
    ),
    spec(
        "doctor",
        &[],
        Category::Help,
        "Environment, keys, sandbox, and provider checks",
        None,
        true,
        None,
        false,
        |_, _| Action::OpenPanel(PanelId::Doctor),
    ),
];

/// Find a spec by name or alias.
pub fn find(name: &str) -> Option<&'static CommandSpec> {
    let n = name.trim().trim_start_matches('/').to_ascii_lowercase();
    COMMANDS
        .iter()
        .find(|c| c.name == n || c.aliases.iter().any(|a| *a == n))
}

/// Run a finished slash line (leading slash optional). Built-ins win over
/// user commands (`R-PAL-18`).
pub fn run_command(view: &mut View, raw: &str) -> Action {
    let raw = raw.trim().trim_start_matches('/');
    let (cmd, rest) = raw
        .split_once(char::is_whitespace)
        .map(|(a, b)| (a, b.trim()))
        .unwrap_or((raw, ""));
    if cmd.is_empty() {
        return Action::None;
    }
    if let Some(spec) = find(cmd) {
        view.note_recent(spec.name);
        return (spec.run)(view, rest);
    }
    if let Some(expanded) = view.catalog.expand(cmd, rest) {
        view.note_recent(cmd);
        let shown = if rest.is_empty() {
            format!("/{cmd}")
        } else {
            format!("/{cmd} {rest}")
        };
        return view.submit_user(shown, expanded);
    }
    view.warn(format!("unknown command /{cmd}"));
    Action::None
}

fn run_new(view: &mut View, _rest: &str) -> Action {
    if view.has_content() {
        view.panels.push(Box::new(Confirm::new(
            "new session",
            "start a fresh session? the current one stays saved on disk.",
            Action::New,
        )));
        Action::None
    } else {
        Action::New
    }
}

fn run_sessions(_view: &mut View, rest: &str) -> Action {
    if rest.is_empty() {
        Action::OpenPanel(PanelId::Sessions(SessionsMode::Browse))
    } else {
        Action::Resume(rest.to_string())
    }
}

/// `/crew` enters crew mode (the crew builder the first time); in crew mode
/// it opens the crew's settings.
fn run_crew(view: &mut View, _rest: &str) -> Action {
    if view.crew_mode() {
        Action::OpenPanel(PanelId::Crew)
    } else {
        Action::EnterCrew
    }
}

fn run_rename(_view: &mut View, rest: &str) -> Action {
    if rest.is_empty() {
        Action::OpenPanel(PanelId::Sessions(SessionsMode::Rename))
    } else {
        Action::RenameSession(rest.to_string())
    }
}

/// `/budget` opens the panel; `/budget 5`, `/budget +2`, and `/budget off`
/// change the cap directly.
fn run_budget(view: &mut View, rest: &str) -> Action {
    let arg = rest.trim().trim_start_matches('$');
    if arg.is_empty() {
        return Action::OpenPanel(PanelId::Budget);
    }
    if matches!(arg, "off" | "none" | "0") {
        return Action::SetBudget(0.0);
    }
    let (add, num) = match arg.strip_prefix('+') {
        Some(n) => (true, n.trim().trim_start_matches('$')),
        None => (false, arg),
    };
    match num.parse::<f64>() {
        Ok(v) if v.is_finite() && v > 0.0 => {
            // Raising a cap you just hit: add to what's set, or to what's spent
            // when nothing is.
            let base = if view.budget_usd > 0.0 {
                view.budget_usd
            } else {
                view.spend.unwrap_or(0.0)
            };
            Action::SetBudget(if add { base + v } else { v })
        }
        _ => {
            view.warn("usage: /budget [amount|+amount|off]   e.g. /budget 5, /budget +2");
            Action::None
        }
    }
}

fn run_delete(_view: &mut View, rest: &str) -> Action {
    if rest.is_empty() {
        Action::OpenPanel(PanelId::Sessions(SessionsMode::Delete))
    } else {
        Action::DeleteSession(rest.to_string())
    }
}

fn run_models(_view: &mut View, rest: &str) -> Action {
    if rest.is_empty() {
        Action::OpenPanel(PanelId::Models)
    } else {
        Action::SetModel(rest.to_string())
    }
}

fn run_provider(view: &mut View, rest: &str) -> Action {
    let rest = rest.trim();
    if rest.is_empty() {
        return Action::OpenPanel(PanelId::Providers);
    }
    let (sub, arg) = rest
        .split_once(char::is_whitespace)
        .map(|(a, b)| (a, b.trim()))
        .unwrap_or((rest, ""));
    match sub {
        "use" => {
            if arg.is_empty() {
                view.warn("usage: /provider use <name>");
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
            Action::BeginSetKey(name)
        }
        "list" => {
            for c in &view.connections.clone() {
                let mark = if c.name == view.connection {
                    "●"
                } else {
                    "○"
                };
                let key = if c.has_key { "key set" } else { "no key" };
                view.system(format!("{mark} {}  {}  {}  {key}", c.name, c.kind, c.model));
            }
            Action::None
        }
        other => Action::UseConnection(other.to_string()),
    }
}

fn run_theme(_view: &mut View, rest: &str) -> Action {
    if rest.is_empty() {
        Action::OpenPanel(PanelId::Theme)
    } else {
        Action::SetTheme(rest.to_string())
    }
}

fn run_tools(_view: &mut View, rest: &str) -> Action {
    match rest {
        "ask" => Action::SetTools { always: false },
        "always" | "auto" | "on" => Action::SetTools { always: true },
        _ => Action::OpenPanel(PanelId::Tools),
    }
}

fn run_auditor(view: &mut View, rest: &str) -> Action {
    match rest {
        "on" => {
            view.auditor_on = true;
            Action::SetAuditor(true)
        }
        "off" => {
            view.auditor_on = false;
            Action::SetAuditor(false)
        }
        _ => Action::OpenPanel(PanelId::Auditor),
    }
}

fn run_skills(view: &mut View, rest: &str) -> Action {
    if rest.is_empty() {
        return Action::OpenPanel(PanelId::Skills);
    }
    let (name, args) = rest
        .split_once(char::is_whitespace)
        .map(|(n, a)| (n, a.trim()))
        .unwrap_or((rest, ""));
    match view.catalog.expand(name, args) {
        Some(expanded) => {
            let shown = if args.is_empty() {
                format!("/{name}")
            } else {
                format!("/{name} {args}")
            };
            view.submit_user(shown, expanded)
        }
        None => Action::OpenPanel(PanelId::Skills),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn specs_are_described_unique_and_reachable() {
        let mut names: HashSet<&str> = HashSet::new();
        for c in COMMANDS {
            assert!(!c.description.is_empty(), "/{} has no description", c.name);
            assert!(names.insert(c.name), "duplicate name /{}", c.name);
            for a in c.aliases {
                assert!(names.insert(a), "duplicate alias /{a} on /{}", c.name);
            }
        }
        // Every non-hidden spec is reachable from an empty palette filter.
        for c in COMMANDS.iter().filter(|c| !c.hidden) {
            assert!(
                crate::palette::matcher::score("", c.name, c.aliases, c.description, false)
                    .is_some()
            );
            assert!(find(c.name).is_some());
        }
    }

    #[test]
    fn budget_sets_raises_and_turns_off() {
        let mut v = View::new(
            ryter_core::Phase::Build,
            "c".into(),
            "m".into(),
            "/tmp".into(),
        );
        v.budget_usd = 5.0;
        v.spend = Some(5.2);
        let set = |v: &mut View, arg: &str| match run_budget(v, arg) {
            Action::SetBudget(x) => Some(x),
            _ => None,
        };
        assert_eq!(set(&mut v, "10"), Some(10.0));
        assert_eq!(set(&mut v, "$2.50"), Some(2.5));
        assert_eq!(set(&mut v, "+2"), Some(7.0), "raise the cap you hit");
        assert_eq!(set(&mut v, "off"), Some(0.0));
        assert_eq!(set(&mut v, "-3"), None);
        assert_eq!(set(&mut v, "lots"), None);
        assert!(v.messages.last().unwrap().body.contains("usage: /budget"));
        // With no cap, "+2" means two more than already spent.
        v.budget_usd = 0.0;
        assert_eq!(set(&mut v, "+2"), Some(7.2));
        // Bare `/budget` opens the panel.
        assert_eq!(run_budget(&mut v, ""), Action::OpenPanel(PanelId::Budget));
    }
}
