//! Live terminal loop.

use std::io::{self, stdout};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use crossterm::ExecutableCommand;
use crossterm::cursor::{Hide, SetCursorStyle};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ryter_core::config::{self, resolve_secret};
use ryter_core::ids::ConnectionId;
use ryter_core::llm::http_provider;
use ryter_core::sandbox::{self, SandboxProfile};
use ryter_core::session::Session;
use ryter_core::spend::PriceBook;
use ryter_core::tools::ToolContext;
use ryter_core::{
    Agent, AgentEvent, Cancel, Config, ConnectionConfig, HookSet, InboundHost, Permission, Phase,
    Provider, Role, StatusSnapshot, UserIo, UserRequest, load_catalog,
};

use crate::commands::{self, Action};
use crate::draw::draw;
use crate::theme::Theme;
use crate::view::{LogLine, View};

/// Launch flags from the CLI.
pub struct TuiOpts {
    /// Treat Ask as Allow.
    pub always_approve: bool,
    /// Connection override.
    pub connection: Option<String>,
    /// Model override.
    pub model: Option<String>,
    /// Phase override.
    pub phase: Option<String>,
    /// Landlock profile override (`off`/`workspace`/`read-only`).
    pub sandbox: Option<String>,
    /// Resume this session id (`latest` = most recent for cwd). `None` creates a new session.
    pub session: Option<String>,
}

enum Work {
    Turn {
        text: String,
        reply: Option<mpsc::Sender<String>>,
    },
    Handoff {
        to: Phase,
        note: String,
    },
    New,
    SetAuditor(bool),
    SetTools {
        always: bool,
    },
    Context,
    Compact,
    ListModels,
    ListCrewModels,
    SetCrew {
        specialists: std::collections::BTreeMap<String, ryter_core::RoleModel>,
    },
    SetMcp {
        servers: std::collections::BTreeMap<String, ryter_core::McpServerConfig>,
    },
    SetHooks {
        hooks: Vec<ryter_core::HookConfig>,
    },
    SetSettings {
        budget_usd: f64,
        max_crew: u32,
        web: bool,
    },
    Resume(String),
    Kill(String),
    Rename(String),
    Reconnect {
        name: String,
        model: String,
        key: String,
    },
    Shutdown,
}

struct TuiAttach {
    work: mpsc::Sender<Work>,
    cancel: Arc<Cancel>,
    status: Arc<std::sync::Mutex<StatusSnapshot>>,
    spend: Arc<std::sync::Mutex<String>>,
}

impl InboundHost for TuiAttach {
    fn prompt(&self, text: &str, phase: Option<Phase>) -> ryter_core::Result<String> {
        if let Some(p) = phase {
            let _ = self.work.send(Work::Handoff {
                to: p,
                note: String::new(),
            });
        }
        let (tx, rx) = mpsc::channel();
        self.work
            .send(Work::Turn {
                text: text.to_string(),
                reply: Some(tx),
            })
            .map_err(|e| ryter_core::Error::Io(e.to_string()))?;
        rx.recv().map_err(|e| ryter_core::Error::Io(e.to_string()))
    }

    fn status(&self) -> StatusSnapshot {
        self.status.lock().map(|g| g.clone()).unwrap_or_default()
    }

    fn spend(&self) -> String {
        self.spend.lock().map(|g| g.clone()).unwrap_or_default()
    }

    fn set_phase(&self, phase: Phase, note: &str) -> ryter_core::Result<()> {
        self.work
            .send(Work::Handoff {
                to: phase,
                note: note.to_string(),
            })
            .map_err(|e| ryter_core::Error::Io(e.to_string()))
    }

    fn cancel(&self) {
        self.cancel.cancel();
    }
}

/// Run the fullscreen TUI. Restores the terminal on exit.
pub fn run(opts: TuiOpts) -> ryter_core::Result<()> {
    let cwd = std::env::current_dir().map_err(|e| ryter_core::Error::Io(e.to_string()))?;
    let _ = ryter_core::ensure_project_memory(&cwd);
    let trusted = config::is_trusted(&cwd);
    let cfg = config::load(Some(&cwd), trusted)?;
    let home = config::home_dir();
    let last = config::load_last_route(&home);
    let (mut conn_name, mut model) = config::resolve_route(
        &cfg,
        last.as_ref(),
        opts.connection.as_deref(),
        opts.model.as_deref(),
    );
    let mut conn = cfg
        .connections
        .get(&conn_name)
        .ok_or_else(|| ryter_core::Error::Config(format!("unknown connection {conn_name}")))?
        .clone();
    let phase = opts
        .phase
        .as_deref()
        .map(Phase::from_str)
        .transpose()?
        .unwrap_or(Phase::Build);
    let profile = if let Some(s) = opts.sandbox.as_deref() {
        s.parse()?
    } else {
        cfg.sandbox.profile()?
    };
    if profile != SandboxProfile::Off {
        let probe = sandbox::probe();
        if probe != "available" {
            return Err(ryter_core::Error::Config(format!(
                "sandbox {profile} requested but Landlock is unavailable ({probe})"
            )));
        }
    }
    let home_ui = home.clone();
    let workspace_ui = cwd.clone();
    let (mut session, resumed) = match opts.session.as_deref() {
        Some("latest") | Some("") => match Session::latest(&home, &cwd)? {
            Some(s) => (s, true),
            None => (
                Session::create(&home, &cwd, phase, conn_name.clone(), model.clone())?,
                false,
            ),
        },
        Some(id) => (Session::find(&home, Some(&cwd), id)?, true),
        None => (
            Session::create(&home, &cwd, phase, conn_name.clone(), model.clone())?,
            false,
        ),
    };
    let phase = if resumed { session.meta.phase } else { phase };
    if !resumed {
        session.set_auditor(cfg.auditor.enabled)?;
    }
    if resumed && cfg.connections.contains_key(&session.meta.connection) {
        conn_name = session.meta.connection.clone();
        model = session.meta.model.clone();
        if let Some(c) = cfg.connections.get(&conn_name) {
            conn = c.clone();
        }
    }
    let key = resolve_secret(&cfg, &ConnectionId::new(&conn_name)).ok();

    let mut view = View::new(
        phase,
        conn_name.clone(),
        model.clone(),
        display_home_path(&cwd),
    );
    view.git_branch = ryter_core::git::branch(&cwd).ok();
    view.perm_mode = if opts.always_approve {
        "always".into()
    } else {
        "ask".into()
    };
    view.catalog = load_catalog(&home, Some(&cwd), trusted);
    view.hooks_help = HookSet::from_config(&cfg.hooks).summary();
    view.hooks = cfg.hooks.clone();
    view.theme_names = Theme::list(&home);
    view.theme_name = "dark".into();
    view.has_key = key.is_some();
    view.auditor_on = cfg.auditor.enabled;
    view.specialists = cfg.specialists.clone();
    view.budget_usd = cfg.spend.session_budget_usd;
    view.warn_usd = cfg.spend.warn_usd;
    view.max_crew = cfg.subagents.max;
    view.sandbox_profile = cfg.sandbox.profile.clone();
    view.web = cfg.features.web;
    view.mcp_inbound = cfg.mcp.inbound;
    view.mcp_bind = cfg.mcp.bind.clone();
    view.mcp_servers = cfg.mcp_servers.clone();
    view.mcp_tokens = config::list_mcp_tokens(&home).into_iter().collect();
    view.session_id = session.meta.id.to_string();
    view.ctx_tokens = Some(0);
    view.ctx_window = Some(ryter_core::window_for(&model));
    view.price_label = PriceBook::from_config(&cfg).format_model_rates(&model);
    if let Some(last) = &last {
        if last.model == model {
            if let Some(w) = last.context_length {
                view.ctx_window = Some(w);
            }
            if let (Some(i), Some(o)) = (last.input_per_million, last.output_per_million) {
                view.price_in = Some(i);
                view.price_out = Some(o);
                view.price_label =
                    ryter_core::format_rates(Some(ryter_core::Rates::per_million(i, o)));
            }
        }
    }
    view.connections = cfg
        .connections
        .iter()
        .map(|(n, c)| crate::view::ConnRow {
            name: n.clone(),
            kind: c.kind.clone(),
            model: c.default_model.clone().unwrap_or_default(),
            has_key: config::has_secret(&cfg, n),
        })
        .collect();
    if resumed {
        fill_view_from_session(&mut view, &session);
    }
    if key.is_none() {
        view.lines.push(LogLine::System(
            "no API key — /connections set-key  or export XAI_API_KEY / OPENROUTER_API_KEY".into(),
        ));
    }

    if cwd.join(".ryter").is_dir() && !trusted {
        view.overlay = Some(crate::view::Overlay::Choice {
            title: "trust this project's .ryter/?".into(),
            kind: crate::view::ChoiceKind::Trust,
            options: crate::view::choice_options(crate::view::ChoiceKind::Trust, &view),
            selected: 0,
            filter: String::new(),
        });
    }
    let (user_io, prompt_rx) = UserIo::pair();
    let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>();
    let (work_tx, work_rx) = mpsc::channel::<Work>();
    let cancel = Cancel::new();
    let live_status = Arc::new(std::sync::Mutex::new(StatusSnapshot {
        phase: phase.to_string(),
        model: model.clone(),
        connection: conn_name.clone(),
        session: session.meta.id.to_string(),
        last_error: String::new(),
    }));
    let live_spend = Arc::new(std::sync::Mutex::new(String::new()));

    let worker_cfg = cfg.clone();
    let worker_conn = conn;
    let worker_key = key.clone();
    let worker_cancel = cancel.clone();
    let worker_status = live_status.clone();
    let worker_spend = live_spend.clone();
    let worker_io = user_io.clone();
    let _ = work_tx.send(Work::ListModels);
    std::thread::spawn(move || {
        worker(
            worker_cfg,
            worker_conn,
            worker_key,
            session,
            cwd,
            home,
            trusted,
            conn_name,
            model,
            opts.always_approve,
            profile,
            work_rx,
            ev_tx,
            worker_cancel,
            worker_status,
            worker_spend,
            worker_io,
        );
    });

    let attach_host: Arc<dyn InboundHost> = Arc::new(TuiAttach {
        work: work_tx.clone(),
        cancel: cancel.clone(),
        status: live_status.clone(),
        spend: live_spend.clone(),
    });
    let mut sock_path: Option<PathBuf> = None;
    if cfg.mcp.inbound {
        #[cfg(unix)]
        {
            let path = cfg
                .mcp
                .socket
                .as_ref()
                .map(PathBuf::from)
                .unwrap_or_else(|| ryter_core::default_socket_path(&home_ui));
            if let Ok(listener) = ryter_core::bind_unix(&path) {
                view.mcp_listen = Some(path.display().to_string());
                sock_path = Some(path);
                let host = attach_host.clone();
                std::thread::spawn(move || {
                    for stream in listener.incoming() {
                        let Ok(stream) = stream else {
                            continue;
                        };
                        let host = host.clone();
                        std::thread::spawn(move || {
                            let Ok(clone) = stream.try_clone() else {
                                return;
                            };
                            let _ = ryter_core::serve_session(clone, stream, host.as_ref(), &[]);
                        });
                    }
                });
            }
        }
        if let Some(bind) = cfg.mcp.bind.as_deref() {
            if let Ok(addr) = bind.parse::<std::net::SocketAddr>() {
                let tokens: Vec<String> = view.mcp_tokens.iter().map(|(_, t)| t.clone()).collect();
                if !tokens.is_empty() && ryter_core::check_tcp(addr, &tokens[0], false).is_ok() {
                    let host = attach_host.clone();
                    view.mcp_tcp_listen = Some(bind.to_string());
                    std::thread::spawn(move || {
                        let _ = ryter_core::serve_tcp(addr, host, tokens);
                    });
                }
            }
        }
    }

    enable_raw_mode().map_err(|e| ryter_core::Error::Io(e.to_string()))?;
    stdout()
        .execute(EnterAlternateScreen)
        .and_then(|s| s.execute(Hide))
        .and_then(|s| s.execute(SetCursorStyle::SteadyBlock))
        .map_err(|e| ryter_core::Error::Io(e.to_string()))?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))
        .map_err(|e| ryter_core::Error::Io(e.to_string()))?;
    let mut theme = Theme::truecolor_dark();
    let result = loop_ui(
        &mut terminal,
        &mut view,
        &work_tx,
        &ev_rx,
        &mut theme,
        &home_ui,
        &workspace_ui,
        trusted,
        profile,
        cfg,
        cancel,
        attach_host,
        prompt_rx,
    );
    if let Some(p) = sock_path {
        let _ = std::fs::remove_file(p);
    }
    let _ = config::save_last_route(&home_ui, &last_from_view(&view));
    let _ = stdout().execute(SetCursorStyle::DefaultUserShape);
    let _ = disable_raw_mode();
    let _ = stdout().execute(LeaveAlternateScreen);
    let _ = work_tx.send(Work::Shutdown);
    result
}

#[allow(clippy::too_many_arguments)]
fn loop_ui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    view: &mut View,
    work_tx: &mpsc::Sender<Work>,
    ev_rx: &mpsc::Receiver<AgentEvent>,
    theme: &mut Theme,
    home: &std::path::Path,
    workspace: &std::path::Path,
    mut trusted: bool,
    sandbox: SandboxProfile,
    mut cfg: Config,
    cancel: Arc<Cancel>,
    mcp_host: Arc<dyn InboundHost>,
    prompt_rx: mpsc::Receiver<UserRequest>,
) -> ryter_core::Result<()> {
    let mut perm_reply: Option<mpsc::Sender<Permission>> = None;
    let mut ask_reply: Option<mpsc::Sender<String>> = None;
    loop {
        while let Ok(ev) = ev_rx.try_recv() {
            apply_event(view, ev);
        }
        drain_user_prompts(view, &prompt_rx, &mut perm_reply, &mut ask_reply);

        terminal
            .draw(|f| draw(f, view, *theme))
            .map_err(|e| ryter_core::Error::Io(e.to_string()))?;

        if !event::poll(Duration::from_millis(50))
            .map_err(|e| ryter_core::Error::Io(e.to_string()))?
        {
            continue;
        }
        let Event::Key(key) = event::read().map_err(|e| ryter_core::Error::Io(e.to_string()))?
        else {
            continue;
        };
        if key.kind != event::KeyEventKind::Press {
            continue;
        }
        match handle_key(view, key) {
            Action::Quit => return Ok(()),
            Action::Submit(text) => {
                let _ = work_tx.send(Work::Turn { text, reply: None });
            }
            Action::Cancel => {
                if view.busy {
                    cancel.cancel();
                    view.lines.push(LogLine::System("cancelling…".into()));
                }
            }
            Action::Handoff { to, note } => {
                let _ = work_tx.send(Work::Handoff { to, note });
            }
            Action::New => {
                view.lines.clear();
                view.composer.clear();
                view.busy = false;
                view.ctx_tokens = Some(0);
                view.crew.clear();
                view.todos.clear();
                view.spend = None;
                view.spend_unknown = false;
                view.spend_by_role.clear();
                view.spend_by_conn.clear();
                let _ = work_tx.send(Work::New);
                view.lines.push(LogLine::System("new session".into()));
            }
            Action::OpenResume => {
                open_session_choice(view, home, workspace, crate::view::ChoiceKind::Resume);
            }
            Action::Resume(id) => {
                if view.busy {
                    view.lines
                        .push(LogLine::System("cancel the running turn first".into()));
                } else {
                    match apply_resume(view, home, workspace, &id) {
                        Ok(()) => {
                            let _ = work_tx.send(Work::Resume(view.session_id.clone()));
                        }
                        Err(e) => view.lines.push(LogLine::System(e.to_string())),
                    }
                }
            }
            Action::OpenDeleteSession => {
                open_session_choice(
                    view,
                    home,
                    workspace,
                    crate::view::ChoiceKind::DeleteSession,
                );
            }
            Action::DeleteSession(id) => {
                match delete_session(view, home, workspace, &id, work_tx) {
                    Ok(msg) => view.lines.push(LogLine::System(msg)),
                    Err(e) => view.lines.push(LogLine::System(e.to_string())),
                }
            }
            Action::RenameSession(title) => {
                view.lines
                    .push(LogLine::System(format!("renamed · {title}")));
                let _ = work_tx.send(Work::Rename(title));
            }
            Action::OpenAgents => {
                view.overlay = Some(crate::view::Overlay::Agents { selected: 0 });
                view.slash = None;
                view.composer.clear();
            }
            Action::KillAgent(id) => {
                if let Some(c) = view.crew.iter().find(|c| c.id == id) {
                    view.lines
                        .push(LogLine::System(format!("killing {} · {}", c.role, c.label)));
                }
                let _ = work_tx.send(Work::Kill(id));
            }
            Action::SetAuditor(on) => {
                let _ = work_tx.send(Work::SetAuditor(on));
            }
            Action::SetTools { always } => {
                view.perm_mode = if always { "always" } else { "ask" }.into();
                let _ = work_tx.send(Work::SetTools { always });
            }
            Action::OpenCrew => {
                view.overlay = Some(crate::view::Overlay::Crew {
                    selected: 0,
                    save_buf: None,
                });
                view.slash = None;
                view.composer.clear();
            }
            Action::OpenMcp => {
                view.overlay = Some(crate::view::Overlay::Mcp {
                    selected: 0,
                    pane: crate::view::McpPane::Home,
                });
                view.slash = None;
                view.composer.clear();
            }
            Action::OpenSkills => {
                view.overlay = Some(crate::view::Overlay::Skills {
                    selected: 0,
                    pane: crate::view::SkillsPane::Home,
                });
                view.slash = None;
                view.composer.clear();
            }
            Action::OpenHooks => {
                view.overlay = Some(crate::view::Overlay::Hooks {
                    selected: 0,
                    pane: crate::view::HooksPane::Home,
                });
                view.slash = None;
                view.composer.clear();
            }
            Action::OpenSpend => {
                view.overlay = Some(crate::view::Overlay::Spend);
                view.slash = None;
                view.composer.clear();
            }
            Action::OpenSettings => {
                view.overlay = Some(crate::view::Overlay::Settings {
                    selected: 0,
                    edit: None,
                });
                view.slash = None;
                view.composer.clear();
            }
            Action::SaveSettings => {
                cfg.spend.session_budget_usd = view.budget_usd;
                cfg.spend.warn_usd = view.warn_usd;
                cfg.subagents.max = view.max_crew;
                cfg.sandbox.profile = view.sandbox_profile.clone();
                cfg.mcp.inbound = view.mcp_inbound;
                cfg.features.web = view.web;
                let _ = config::save_settings(home, &cfg);
                let _ = work_tx.send(Work::SetSettings {
                    budget_usd: view.budget_usd,
                    max_crew: view.max_crew,
                    web: view.web,
                });
            }
            Action::PermissionReply(p) => {
                if let Some(tx) = perm_reply.take() {
                    let _ = tx.send(p);
                }
                view.overlay = None;
            }
            Action::AskUserReply(s) => {
                if let Some(tx) = ask_reply.take() {
                    let _ = tx.send(s);
                }
                view.overlay = None;
            }
            Action::TrustProject(yes) => {
                view.overlay = None;
                if yes {
                    match config::trust(workspace) {
                        Ok(()) => {
                            trusted = true;
                            view.catalog = load_catalog(home, Some(workspace), true);
                            view.lines
                                .push(LogLine::System("trusted this project's .ryter/".into()));
                        }
                        Err(e) => view.lines.push(LogLine::System(e.to_string())),
                    }
                } else {
                    view.lines
                        .push(LogLine::System("left project untrusted".into()));
                }
            }
            Action::WriteSkill { name, description } => {
                match ryter_core::write_skill(home, &name, &description) {
                    Ok(path) => {
                        view.catalog = load_catalog(home, Some(workspace), trusted);
                        view.lines.push(LogLine::System(format!(
                            "wrote {}  · edit the body, then /{name}",
                            path.display()
                        )));
                        view.overlay = Some(crate::view::Overlay::Skills {
                            selected: 0,
                            pane: crate::view::SkillsPane::Home,
                        });
                    }
                    Err(e) => view.lines.push(LogLine::System(e.to_string())),
                }
            }
            Action::WriteCommand { name } => match ryter_core::write_command(home, &name, "") {
                Ok(path) => {
                    view.catalog = load_catalog(home, Some(workspace), trusted);
                    view.lines.push(LogLine::System(format!(
                        "wrote {}  · $ARGUMENTS is replaced on invoke",
                        path.display()
                    )));
                    view.overlay = Some(crate::view::Overlay::Skills {
                        selected: 0,
                        pane: crate::view::SkillsPane::Home,
                    });
                }
                Err(e) => view.lines.push(LogLine::System(e.to_string())),
            },
            Action::RemoveCatalog(path) => match ryter_core::remove_catalog_entry(home, &path) {
                Ok(()) => {
                    view.catalog = load_catalog(home, Some(workspace), trusted);
                    view.lines
                        .push(LogLine::System(format!("removed {}", path.display())));
                }
                Err(e) => view.lines.push(LogLine::System(e.to_string())),
            },
            Action::SaveHooks => {
                let _ = config::save_hooks(home, &view.hooks);
                view.hooks_help = HookSet::from_config(&view.hooks).summary();
                cfg.hooks = view.hooks.clone();
                let _ = work_tx.send(Work::SetHooks {
                    hooks: view.hooks.clone(),
                });
            }
            Action::SaveMcp => {
                persist_mcp(home, view, &mut cfg);
                let _ = work_tx.send(Work::SetMcp {
                    servers: view.mcp_servers.clone(),
                });
            }
            Action::McpListenTcp => {
                persist_mcp(home, view, &mut cfg);
                start_mcp_tcp(view, mcp_host.clone());
                let _ = work_tx.send(Work::SetMcp {
                    servers: view.mcp_servers.clone(),
                });
            }
            Action::SetCrewRole {
                role,
                connection,
                model,
            } => {
                if connection == view.connection && model == view.model {
                    view.specialists.remove(&role);
                } else {
                    view.specialists.insert(
                        role.clone(),
                        ryter_core::RoleModel {
                            connection: Some(connection),
                            model: Some(model),
                        },
                    );
                }
                let _ = config::save_crew(home, &view.specialists);
                let _ = work_tx.send(Work::SetCrew {
                    specialists: view.specialists.clone(),
                });
                view.overlay = Some(crate::view::Overlay::Crew {
                    selected: crate::view::CREW_ROLES
                        .iter()
                        .position(|r| *r == role)
                        .unwrap_or(0),
                    save_buf: None,
                });
            }
            Action::SaveCrewPreset(name) => {
                let _ = config::save_crew_preset(home, &name, &view.specialists);
                view.overlay = Some(crate::view::Overlay::Crew {
                    selected: 0,
                    save_buf: None,
                });
            }
            Action::LoadCrewPreset(name) => {
                let mut cfg_mut = cfg.clone();
                if config::load_crew_preset(home, &mut cfg_mut, &name).is_ok() {
                    view.specialists = cfg_mut.specialists.clone();
                    let _ = work_tx.send(Work::SetCrew {
                        specialists: view.specialists.clone(),
                    });
                }
                view.overlay = Some(crate::view::Overlay::Crew {
                    selected: 0,
                    save_buf: None,
                });
            }
            Action::ListCrewModels { role } => {
                open_crew_models(view, &cfg, work_tx, role);
            }
            Action::OpenCrewPresets => {
                let options: Vec<crate::view::ChoiceItem> = config::list_crew_presets(home)
                    .into_iter()
                    .map(|n| crate::view::ChoiceItem {
                        id: n.clone(),
                        label: n,
                    })
                    .collect();
                if options.is_empty() {
                    view.overlay = Some(crate::view::Overlay::Crew {
                        selected: crate::view::CREW_ROLES.len() + 1,
                        save_buf: None,
                    });
                } else {
                    view.overlay = Some(crate::view::Overlay::Choice {
                        title: "load crew preset".into(),
                        kind: crate::view::ChoiceKind::CrewPreset,
                        options,
                        selected: 0,
                        filter: String::new(),
                    });
                }
            }
            Action::Context => {
                let _ = work_tx.send(Work::Context);
            }
            Action::Compact => {
                let _ = work_tx.send(Work::Compact);
            }
            Action::Doctor => {
                let report = ryter_core::doctor::run(ryter_core::doctor::DoctorOpts {
                    home,
                    cwd: workspace,
                    trusted,
                    sandbox,
                });
                view.lines.push(LogLine::System(report.render()));
            }
            Action::SetTheme(name) => match Theme::load_named(home, &name) {
                Ok(t) => {
                    *theme = t;
                    view.theme_name = name.clone();
                }
                Err(e) => view.lines.push(LogLine::System(e)),
            },
            Action::UseConnection(name) => {
                apply_use_connection(view, work_tx, home, &cfg, &name);
            }
            Action::SetKey { name, key } => {
                apply_set_key(view, work_tx, home, &cfg, &name, &key);
            }
            Action::SetModel(model) => {
                apply_set_model(view, work_tx, home, &cfg, model);
            }
            Action::OpenProvider => {
                let selected = view
                    .connections
                    .iter()
                    .position(|c| c.name == view.connection)
                    .unwrap_or(0);
                view.overlay = Some(crate::view::Overlay::Provider {
                    selected,
                    filter: String::new(),
                });
                view.slash = None;
                view.composer.clear();
            }
            Action::OpenModels => {
                let kind = view
                    .connections
                    .iter()
                    .find(|c| c.name == view.connection)
                    .map(|c| c.kind.as_str())
                    .unwrap_or("");
                let mut items = crate::view::fallback_models(kind, &view.model);
                let book = PriceBook::from_config(&cfg);
                for m in &mut items {
                    if m.input_per_million.is_none() {
                        if let Some(r) = book.rates(&m.id) {
                            m.input_per_million = Some(r.input_per_million);
                            m.output_per_million = Some(r.output_per_million);
                        }
                    }
                }
                view.overlay = Some(crate::view::Overlay::Model {
                    filter: String::new(),
                    selected: 0,
                    items,
                    loading: true,
                    assign_role: None,
                });
                view.slash = None;
                view.composer.clear();
                let _ = work_tx.send(Work::ListModels);
            }
            Action::None => {}
        }
    }
}

fn apply_use_connection(
    view: &mut View,
    work_tx: &mpsc::Sender<Work>,
    home: &std::path::Path,
    cfg: &Config,
    name: &str,
) {
    let Some(conn) = cfg.connections.get(name) else {
        view.lines
            .push(LogLine::System(format!("unknown connection {name}")));
        return;
    };
    match resolve_secret(cfg, &ConnectionId::new(name)) {
        Ok(key) => {
            let model = conn
                .default_model
                .clone()
                .or_else(|| cfg.orchestrator.model.clone())
                .unwrap_or_else(|| "grok-4.6".into());
            view.connection = name.to_string();
            view.model = model.clone();
            view.has_key = true;
            view.overlay = None;
            view.ctx_window = Some(ryter_core::window_for(&model));
            view.price_label = PriceBook::from_config(cfg).format_model_rates(&model);
            if let Some(r) = PriceBook::from_config(cfg).rates(&model) {
                view.price_in = Some(r.input_per_million);
                view.price_out = Some(r.output_per_million);
            } else {
                view.price_in = None;
                view.price_out = None;
            }
            let _ = config::save_last_route(home, &last_from_view(view));
            let _ = work_tx.send(Work::Reconnect {
                name: name.to_string(),
                model,
                key,
            });
            let _ = work_tx.send(Work::ListModels);
        }
        Err(_) => {
            view.secret_for = Some(name.to_string());
            view.secret_buf.clear();
            view.composer.clear();
        }
    }
}

fn apply_set_key(
    view: &mut View,
    work_tx: &mpsc::Sender<Work>,
    home: &std::path::Path,
    cfg: &Config,
    name: &str,
    key: &str,
) {
    match config::store_secret_at(home, name, key) {
        Ok(()) => {
            if let Some(c) = view.connections.iter_mut().find(|c| c.name == name) {
                c.has_key = true;
            }
            view.has_key = true;
            view.overlay = None;
            apply_use_connection(view, work_tx, home, cfg, name);
        }
        Err(e) => view.lines.push(LogLine::System(e.to_string())),
    }
}

fn apply_set_model(
    view: &mut View,
    work_tx: &mpsc::Sender<Work>,
    home: &std::path::Path,
    cfg: &Config,
    model: String,
) {
    match resolve_secret(cfg, &ConnectionId::new(&view.connection)) {
        Ok(key) => {
            view.model = model.clone();
            if view.ctx_window.is_none() {
                view.ctx_window = Some(ryter_core::window_for(&model));
            }
            let book = PriceBook::from_config(cfg);
            if let Some(r) = book.rates(&model) {
                view.price_label = book.format_model_rates(&model);
                view.price_in = Some(r.input_per_million);
                view.price_out = Some(r.output_per_million);
            }
            view.overlay = None;
            let _ = config::save_last_route(home, &last_from_view(view));
            let _ = work_tx.send(Work::Reconnect {
                name: view.connection.clone(),
                model,
                key,
            });
        }
        Err(_) => {
            view.secret_for = Some(view.connection.clone());
            view.secret_buf.clear();
        }
    }
}

fn handle_key(view: &mut View, key: KeyEvent) -> Action {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        if view.busy {
            return Action::Cancel;
        }
        return Action::Quit;
    }
    if view.secret_for.is_some() {
        return handle_secret_key(view, key);
    }
    if view.overlay.is_some() {
        return handle_overlay_key(view, key);
    }
    match key.code {
        KeyCode::Esc => {
            if view.busy {
                Action::Cancel
            } else if view.handoff_to.take().is_some() {
                view.composer.clear();
                view.lines.push(LogLine::System("handoff cancelled".into()));
                Action::None
            } else {
                view.composer.clear();
                view.slash = None;
                Action::None
            }
        }
        KeyCode::Enter => commands::submit(view),
        KeyCode::Backspace => {
            view.composer.pop();
            commands::refresh_slash(view);
            Action::None
        }
        KeyCode::Up => {
            if let Some(s) = &mut view.slash {
                if s.selected > 0 {
                    s.selected -= 1;
                }
            }
            Action::None
        }
        KeyCode::Down => {
            if let Some(s) = &mut view.slash {
                if s.selected + 1 < s.matches.len() {
                    s.selected += 1;
                }
            }
            Action::None
        }
        KeyCode::Tab => {
            if let Some(s) = &view.slash {
                if let Some(name) = s.matches.get(s.selected) {
                    view.composer = format!("/{name} ");
                    view.slash = None;
                }
            }
            Action::None
        }
        KeyCode::Char(c) => {
            view.composer.push(c);
            commands::refresh_slash(view);
            Action::None
        }
        _ => Action::None,
    }
}

fn drain_user_prompts(
    view: &mut View,
    prompt_rx: &mpsc::Receiver<UserRequest>,
    perm_reply: &mut Option<mpsc::Sender<Permission>>,
    ask_reply: &mut Option<mpsc::Sender<String>>,
) {
    if matches!(
        view.overlay,
        Some(crate::view::Overlay::Permission { .. } | crate::view::Overlay::AskUser { .. })
    ) {
        return;
    }
    let Ok(req) = prompt_rx.try_recv() else {
        return;
    };
    match req {
        UserRequest::Permission {
            tool,
            summary,
            reply,
        } => {
            *perm_reply = Some(reply);
            view.overlay = Some(crate::view::Overlay::Permission { tool, summary });
        }
        UserRequest::Question {
            question,
            options,
            reply,
        } => {
            *ask_reply = Some(reply);
            view.overlay = Some(crate::view::Overlay::AskUser {
                question,
                options,
                selected: 0,
                buf: String::new(),
            });
        }
    }
}

fn handle_permission_key(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('y' | 'Y') | KeyCode::Enter => Action::PermissionReply(Permission::Allow),
        KeyCode::Char('n' | 'N') | KeyCode::Esc => Action::PermissionReply(Permission::Deny),
        KeyCode::Char('a' | 'A') => Action::PermissionReply(Permission::Always),
        _ => Action::None,
    }
}

fn handle_ask_user_key(view: &mut View, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => Action::AskUserReply(String::new()),
        KeyCode::Enter => ask_user_submit(view),
        KeyCode::Up => {
            overlay_move(view, -1);
            Action::None
        }
        KeyCode::Down => {
            overlay_move(view, 1);
            Action::None
        }
        KeyCode::Backspace => {
            overlay_pop_filter(view);
            Action::None
        }
        KeyCode::Char(c) if !c.is_control() => {
            overlay_push_filter(view, c);
            Action::None
        }
        _ => Action::None,
    }
}

fn ask_user_submit(view: &mut View) -> Action {
    let Some(crate::view::Overlay::AskUser {
        options,
        selected,
        buf,
        ..
    }) = &view.overlay
    else {
        return Action::None;
    };
    if options.is_empty() {
        Action::AskUserReply(buf.clone())
    } else {
        Action::AskUserReply(options.get(*selected).cloned().unwrap_or_default())
    }
}

fn handle_settings_key(view: &mut View, key: KeyEvent) -> Action {
    let editing = matches!(
        view.overlay,
        Some(crate::view::Overlay::Settings { edit: Some(_), .. })
    );
    match key.code {
        KeyCode::Esc => {
            if let Some(crate::view::Overlay::Settings { edit, .. }) = &mut view.overlay {
                if edit.is_some() {
                    *edit = None;
                    return Action::None;
                }
            }
            view.overlay = None;
            Action::None
        }
        KeyCode::Enter => settings_enter(view),
        KeyCode::Up if !editing => {
            overlay_move(view, -1);
            Action::None
        }
        KeyCode::Down if !editing => {
            overlay_move(view, 1);
            Action::None
        }
        KeyCode::Backspace => {
            overlay_pop_filter(view);
            Action::None
        }
        KeyCode::Char(c) if !c.is_control() => {
            overlay_push_filter(view, c);
            Action::None
        }
        _ => Action::None,
    }
}

fn settings_enter(view: &mut View) -> Action {
    let Some(crate::view::Overlay::Settings { selected, edit }) = &mut view.overlay else {
        return Action::None;
    };
    let selected = *selected;
    if let Some(buf) = edit.take() {
        match selected {
            0 => {
                if let Ok(v) = buf.trim().parse::<f64>() {
                    view.budget_usd = v.max(0.0);
                }
            }
            1 => {
                if let Ok(v) = buf.trim().parse::<f64>() {
                    view.warn_usd = v.max(0.0);
                }
            }
            2 => {
                if let Ok(v) = buf.trim().parse::<u32>() {
                    view.max_crew = v.max(1);
                }
            }
            _ => {}
        }
        return Action::SaveSettings;
    }
    match selected {
        0 => {
            if let Some(crate::view::Overlay::Settings { edit, .. }) = &mut view.overlay {
                *edit = Some(format!("{:.2}", view.budget_usd));
            }
            Action::None
        }
        1 => {
            if let Some(crate::view::Overlay::Settings { edit, .. }) = &mut view.overlay {
                *edit = Some(format!("{:.2}", view.warn_usd));
            }
            Action::None
        }
        2 => {
            if let Some(crate::view::Overlay::Settings { edit, .. }) = &mut view.overlay {
                *edit = Some(view.max_crew.to_string());
            }
            Action::None
        }
        3 => {
            let options = crate::view::choice_options(crate::view::ChoiceKind::Sandbox, view);
            let selected = options
                .iter()
                .position(|c| c.id == view.sandbox_profile)
                .unwrap_or(0);
            view.overlay = Some(crate::view::Overlay::Choice {
                title: "sandbox".into(),
                kind: crate::view::ChoiceKind::Sandbox,
                options,
                selected,
                filter: String::new(),
            });
            Action::None
        }
        4 => {
            view.mcp_inbound = !view.mcp_inbound;
            Action::SaveSettings
        }
        5 => {
            view.web = !view.web;
            Action::SaveSettings
        }
        _ => Action::None,
    }
}

fn handle_overlay_key(view: &mut View, key: KeyEvent) -> Action {
    if matches!(view.overlay, Some(crate::view::Overlay::Permission { .. })) {
        return handle_permission_key(key);
    }
    if matches!(view.overlay, Some(crate::view::Overlay::AskUser { .. })) {
        return handle_ask_user_key(view, key);
    }
    if matches!(view.overlay, Some(crate::view::Overlay::Spend)) {
        if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
            view.overlay = None;
        }
        return Action::None;
    }
    if matches!(view.overlay, Some(crate::view::Overlay::Settings { .. })) {
        return handle_settings_key(view, key);
    }
    match key.code {
        KeyCode::Esc => {
            view.secret_for = None;
            view.secret_buf.clear();
            match &view.overlay {
                Some(crate::view::Overlay::Model {
                    assign_role: Some(_),
                    ..
                }) => {
                    view.overlay = Some(crate::view::Overlay::Crew {
                        selected: 0,
                        save_buf: None,
                    });
                }
                Some(crate::view::Overlay::Choice {
                    kind: crate::view::ChoiceKind::CrewPreset,
                    ..
                }) => {
                    view.overlay = Some(crate::view::Overlay::Crew {
                        selected: crate::view::CREW_ROLES.len() + 1,
                        save_buf: None,
                    });
                }
                Some(crate::view::Overlay::Crew {
                    save_buf: Some(_),
                    selected,
                }) => {
                    let selected = *selected;
                    view.overlay = Some(crate::view::Overlay::Crew {
                        selected,
                        save_buf: None,
                    });
                }
                Some(crate::view::Overlay::Mcp {
                    pane: crate::view::McpPane::Home,
                    ..
                }) => view.overlay = None,
                Some(crate::view::Overlay::Mcp { .. }) => {
                    view.overlay = Some(crate::view::Overlay::Mcp {
                        selected: 0,
                        pane: crate::view::McpPane::Home,
                    });
                }
                Some(crate::view::Overlay::Skills {
                    pane: crate::view::SkillsPane::Home,
                    ..
                }) => view.overlay = None,
                Some(crate::view::Overlay::Skills { .. }) => {
                    view.overlay = Some(crate::view::Overlay::Skills {
                        selected: 0,
                        pane: crate::view::SkillsPane::Home,
                    });
                }
                Some(crate::view::Overlay::Hooks {
                    pane: crate::view::HooksPane::Home,
                    ..
                }) => view.overlay = None,
                Some(crate::view::Overlay::Hooks { .. }) => {
                    view.overlay = Some(crate::view::Overlay::Hooks {
                        selected: 0,
                        pane: crate::view::HooksPane::Home,
                    });
                }
                Some(crate::view::Overlay::Choice {
                    kind: crate::view::ChoiceKind::Trust,
                    ..
                }) => {
                    view.overlay = None;
                    return Action::TrustProject(false);
                }
                _ => view.overlay = None,
            }
            Action::None
        }
        KeyCode::Up => {
            overlay_move(view, -1);
            Action::None
        }
        KeyCode::Down => {
            overlay_move(view, 1);
            Action::None
        }
        KeyCode::Enter => overlay_enter(view),
        KeyCode::Backspace => {
            if let Some(crate::view::Overlay::Mcp {
                pane: crate::view::McpPane::Home,
                selected,
            }) = &view.overlay
            {
                let selected = *selected;
                if selected >= 1 && selected <= view.mcp_servers.len() {
                    let name = view.mcp_server_names()[selected - 1].clone();
                    view.mcp_servers.remove(&name);
                    return Action::SaveMcp;
                }
            }
            if let Some(crate::view::Overlay::Skills {
                pane: crate::view::SkillsPane::Home,
                selected,
            }) = &view.overlay
            {
                let selected = *selected;
                if let Some(path) = catalog_source_at(view, selected) {
                    return Action::RemoveCatalog(path);
                }
            }
            if let Some(crate::view::Overlay::Hooks {
                pane: crate::view::HooksPane::Home,
                selected,
            }) = &view.overlay
            {
                let selected = *selected;
                if selected < view.hooks.len() {
                    view.hooks.remove(selected);
                    return Action::SaveHooks;
                }
            }
            overlay_pop_filter(view);
            Action::None
        }
        KeyCode::Char(c) if !c.is_control() => {
            overlay_push_filter(view, c);
            Action::None
        }
        _ => Action::None,
    }
}

fn overlay_move(view: &mut View, delta: i32) {
    let n = match &view.overlay {
        Some(crate::view::Overlay::Provider { .. }) => view.filtered_providers().len(),
        Some(crate::view::Overlay::Model { .. }) => view.filtered_models().len(),
        Some(crate::view::Overlay::Choice { .. }) => view.filtered_choices().len(),
        Some(crate::view::Overlay::Crew {
            save_buf: Some(_), ..
        }) => return,
        Some(crate::view::Overlay::Crew { .. }) => crate::view::CREW_ROLES.len() + 2,
        Some(crate::view::Overlay::Agents { .. }) => view.crew.len().max(1),
        Some(crate::view::Overlay::Mcp {
            pane: crate::view::McpPane::Home,
            ..
        }) => view.mcp_home_len(),
        Some(crate::view::Overlay::Mcp {
            pane: crate::view::McpPane::Inbound,
            ..
        }) => crate::view::MCP_INBOUND_ROWS,
        Some(crate::view::Overlay::Mcp { .. }) => return,
        Some(crate::view::Overlay::Skills {
            pane: crate::view::SkillsPane::Home,
            ..
        }) => view.skills_home_len(),
        Some(crate::view::Overlay::Skills { .. }) => return,
        Some(crate::view::Overlay::Hooks {
            pane: crate::view::HooksPane::Home,
            ..
        }) => view.hooks_home_len(),
        Some(crate::view::Overlay::Hooks {
            pane: crate::view::HooksPane::AddEvent,
            ..
        }) => ryter_core::HookEvent::all().len(),
        Some(crate::view::Overlay::Hooks { .. }) => return,
        Some(crate::view::Overlay::AskUser { options, .. }) if !options.is_empty() => options.len(),
        Some(crate::view::Overlay::Settings { edit: None, .. }) => crate::view::SETTINGS_ROWS,
        Some(
            crate::view::Overlay::Permission { .. }
            | crate::view::Overlay::Spend
            | crate::view::Overlay::AskUser { .. }
            | crate::view::Overlay::Settings { .. },
        ) => return,
        None => return,
    };
    if n == 0 {
        return;
    }
    match &mut view.overlay {
        Some(crate::view::Overlay::Provider { selected, .. })
        | Some(crate::view::Overlay::Model { selected, .. })
        | Some(crate::view::Overlay::Choice { selected, .. })
        | Some(crate::view::Overlay::Crew { selected, .. })
        | Some(crate::view::Overlay::Agents { selected })
        | Some(crate::view::Overlay::Mcp { selected, .. })
        | Some(crate::view::Overlay::Skills { selected, .. })
        | Some(crate::view::Overlay::Hooks { selected, .. })
        | Some(crate::view::Overlay::AskUser { selected, .. })
        | Some(crate::view::Overlay::Settings { selected, .. }) => {
            *selected = (*selected as i32 + delta).rem_euclid(n as i32) as usize;
        }
        Some(crate::view::Overlay::Permission { .. } | crate::view::Overlay::Spend) | None => {}
    }
}

fn overlay_push_filter(view: &mut View, c: char) {
    match &mut view.overlay {
        Some(crate::view::Overlay::Provider { filter, selected }) => {
            filter.push(c);
            *selected = 0;
        }
        Some(crate::view::Overlay::Model {
            filter, selected, ..
        }) => {
            filter.push(c);
            *selected = 0;
        }
        Some(crate::view::Overlay::Choice {
            filter, selected, ..
        }) => {
            filter.push(c);
            *selected = 0;
        }
        Some(crate::view::Overlay::Crew {
            save_buf: Some(buf),
            ..
        }) => buf.push(c),
        Some(crate::view::Overlay::Mcp {
            pane:
                crate::view::McpPane::AddName { buf }
                | crate::view::McpPane::AddCommand { buf, .. }
                | crate::view::McpPane::AddArgs { buf, .. },
            ..
        }) => buf.push(c),
        Some(crate::view::Overlay::Skills {
            pane:
                crate::view::SkillsPane::Args { buf, .. }
                | crate::view::SkillsPane::AddSkillName { buf }
                | crate::view::SkillsPane::AddSkillDesc { buf, .. }
                | crate::view::SkillsPane::AddCmdName { buf },
            ..
        }) => buf.push(c),
        Some(crate::view::Overlay::Hooks {
            pane:
                crate::view::HooksPane::AddTarget { buf, .. }
                | crate::view::HooksPane::AddMatcher { buf, .. },
            ..
        }) => buf.push(c),
        Some(crate::view::Overlay::AskUser { options, buf, .. }) if options.is_empty() => {
            buf.push(c)
        }
        Some(crate::view::Overlay::Settings {
            edit: Some(buf), ..
        }) => buf.push(c),
        _ => {}
    }
}

fn overlay_pop_filter(view: &mut View) {
    match &mut view.overlay {
        Some(crate::view::Overlay::Provider { filter, selected }) => {
            filter.pop();
            *selected = 0;
        }
        Some(crate::view::Overlay::Model {
            filter, selected, ..
        }) => {
            filter.pop();
            *selected = 0;
        }
        Some(crate::view::Overlay::Choice {
            filter, selected, ..
        }) => {
            filter.pop();
            *selected = 0;
        }
        Some(crate::view::Overlay::Crew {
            save_buf: Some(buf),
            ..
        }) => {
            buf.pop();
        }
        Some(crate::view::Overlay::Mcp {
            pane:
                crate::view::McpPane::AddName { buf }
                | crate::view::McpPane::AddCommand { buf, .. }
                | crate::view::McpPane::AddArgs { buf, .. },
            ..
        }) => {
            buf.pop();
        }
        Some(crate::view::Overlay::Skills {
            pane:
                crate::view::SkillsPane::Args { buf, .. }
                | crate::view::SkillsPane::AddSkillName { buf }
                | crate::view::SkillsPane::AddSkillDesc { buf, .. }
                | crate::view::SkillsPane::AddCmdName { buf },
            ..
        }) => {
            buf.pop();
        }
        Some(crate::view::Overlay::Hooks {
            pane:
                crate::view::HooksPane::AddTarget { buf, .. }
                | crate::view::HooksPane::AddMatcher { buf, .. },
            ..
        }) => {
            buf.pop();
        }
        Some(crate::view::Overlay::AskUser { options, buf, .. }) if options.is_empty() => {
            buf.pop();
        }
        Some(crate::view::Overlay::Settings {
            edit: Some(buf), ..
        }) => {
            buf.pop();
        }
        _ => {}
    }
}

fn overlay_enter(view: &mut View) -> Action {
    match &view.overlay {
        Some(crate::view::Overlay::Provider { selected, .. }) => {
            let selected = *selected;
            let picked = view
                .filtered_providers()
                .get(selected)
                .map(|c| (c.name.clone(), c.has_key));
            let Some((name, has_key)) = picked else {
                return Action::None;
            };
            if has_key {
                view.overlay = None;
                Action::UseConnection(name)
            } else {
                view.secret_for = Some(name);
                view.secret_buf.clear();
                Action::None
            }
        }
        Some(crate::view::Overlay::Model {
            selected,
            assign_role,
            ..
        }) => {
            let selected = *selected;
            let assign_role = assign_role.clone();
            let picked: Option<ryter_core::ModelInfo> =
                view.filtered_models().get(selected).map(|m| (*m).clone());
            let Some(m) = picked else {
                return Action::None;
            };
            if let Some(role) = assign_role {
                let connection = m
                    .connection
                    .clone()
                    .unwrap_or_else(|| view.connection.clone());
                let model = if m.id.is_empty() {
                    view.model.clone()
                } else {
                    m.id.clone()
                };
                let connection = if m.id.is_empty() {
                    view.connection.clone()
                } else {
                    connection
                };
                return Action::SetCrewRole {
                    role,
                    connection,
                    model,
                };
            }
            let id = m.id.clone();
            view.ctx_window = Some(
                m.context_length
                    .unwrap_or_else(|| ryter_core::window_for(&id)),
            );
            if let (Some(i), Some(o)) = (m.input_per_million, m.output_per_million) {
                view.price_in = Some(i);
                view.price_out = Some(o);
                view.price_label =
                    ryter_core::format_rates(Some(ryter_core::Rates::per_million(i, o)));
            }
            view.overlay = None;
            Action::SetModel(id)
        }
        Some(crate::view::Overlay::Crew {
            selected, save_buf, ..
        }) => {
            if let Some(name) = save_buf.clone() {
                if name.trim().is_empty() {
                    return Action::None;
                }
                return Action::SaveCrewPreset(name);
            }
            let selected = *selected;
            if selected < crate::view::CREW_ROLES.len() {
                Action::ListCrewModels {
                    role: crate::view::CREW_ROLES[selected].to_string(),
                }
            } else if selected == crate::view::CREW_ROLES.len() {
                if let Some(crate::view::Overlay::Crew { save_buf, .. }) = &mut view.overlay {
                    *save_buf = Some(String::new());
                }
                Action::None
            } else {
                Action::OpenCrewPresets
            }
        }
        Some(crate::view::Overlay::Choice { kind, selected, .. }) => {
            let kind = *kind;
            let selected = *selected;
            let id = view.filtered_choices().get(selected).map(|c| c.id.clone());
            let Some(id) = id else {
                return Action::None;
            };
            view.overlay = None;
            match kind {
                crate::view::ChoiceKind::Auditor => {
                    let on = id == "on";
                    view.auditor_on = on;
                    Action::SetAuditor(on)
                }
                crate::view::ChoiceKind::Tools => Action::SetTools {
                    always: id == "always",
                },
                crate::view::ChoiceKind::Theme => Action::SetTheme(id),
                crate::view::ChoiceKind::Phase => {
                    if let Ok(to) = Phase::from_str(&id) {
                        Action::Handoff {
                            to,
                            note: String::new(),
                        }
                    } else {
                        Action::None
                    }
                }
                crate::view::ChoiceKind::CrewPreset => Action::LoadCrewPreset(id),
                crate::view::ChoiceKind::Resume => Action::Resume(id),
                crate::view::ChoiceKind::DeleteSession => Action::DeleteSession(id),
                crate::view::ChoiceKind::Trust => Action::TrustProject(id == "yes"),
                crate::view::ChoiceKind::Sandbox => {
                    view.sandbox_profile = id;
                    view.overlay = Some(crate::view::Overlay::Settings {
                        selected: 3,
                        edit: None,
                    });
                    Action::SaveSettings
                }
            }
        }
        Some(crate::view::Overlay::Agents { selected }) => {
            let selected = *selected;
            view.overlay = None;
            match view.crew.get(selected) {
                Some(c) => Action::KillAgent(c.id.clone()),
                None => Action::None,
            }
        }
        Some(crate::view::Overlay::Mcp { .. }) => mcp_enter(view),
        Some(crate::view::Overlay::Skills { .. }) => skills_enter(view),
        Some(crate::view::Overlay::Hooks { .. }) => hooks_enter(view),
        Some(crate::view::Overlay::Permission { .. }) => Action::PermissionReply(Permission::Allow),
        Some(crate::view::Overlay::AskUser { .. }) => ask_user_submit(view),
        Some(crate::view::Overlay::Spend) => {
            view.overlay = None;
            Action::None
        }
        Some(crate::view::Overlay::Settings { .. }) => settings_enter(view),
        None => Action::None,
    }
}

fn handle_secret_key(view: &mut View, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            view.secret_for = None;
            view.secret_buf.clear();
            view.lines.push(LogLine::System("set-key cancelled".into()));
            Action::None
        }
        KeyCode::Enter => commands::submit(view),
        KeyCode::Backspace => {
            view.secret_buf.pop();
            Action::None
        }
        KeyCode::Char(c) => {
            if !c.is_control() {
                view.secret_buf.push(c);
            }
            Action::None
        }
        _ => Action::None,
    }
}

fn apply_event(view: &mut View, ev: AgentEvent) {
    match ev {
        AgentEvent::Token { text } => commands::on_token(view, &text),
        AgentEvent::Reasoning { .. } => {}
        AgentEvent::ToolCall { name, args, .. } => {
            view.lines.push(LogLine::Tool(name.clone()));
            if name == "todo_write" {
                view.todos = parse_todos(&args);
            }
        }
        AgentEvent::ToolResult { is_error, .. } => {
            if is_error {
                view.lines.push(LogLine::System("tool error".into()));
            }
        }
        AgentEvent::Spend {
            connection,
            role,
            total_usd,
            input_tokens,
            output_tokens,
            ..
        } => {
            if let Some(v) = total_usd {
                view.spend = Some(view.spend.unwrap_or(0.0) + v);
                *view.spend_by_role.entry(role.to_string()).or_insert(0.0) += v;
                *view.spend_by_conn.entry(connection).or_insert(0.0) += v;
            } else {
                view.spend_unknown = true;
            }
            let used = input_tokens.saturating_add(output_tokens);
            view.ctx_tokens = Some(view.ctx_tokens.unwrap_or(0).saturating_add(used));
            let window = view
                .ctx_window
                .unwrap_or_else(|| ryter_core::compact::window_for(&view.model));
            view.ctx_window = Some(window);
            view.ctx_pct =
                Some(((view.ctx_tokens.unwrap_or(0).min(window) * 100) / window.max(1)) as u8);
            view.busy = false;
        }
        AgentEvent::PhaseChanged { phase } => {
            view.phase = phase;
        }
        AgentEvent::SubagentStarted {
            id,
            role,
            description,
        } => {
            view.crew.push(crate::view::CrewRow {
                id: id.to_string(),
                role: role.to_string(),
                label: description,
                spend: None,
                status: "running".into(),
            });
        }
        AgentEvent::SubagentFinished {
            id,
            role,
            summary,
            body,
        } => {
            if !body.trim().is_empty() {
                view.lines.push(LogLine::Specialist {
                    role: role.as_str().to_string(),
                    text: body,
                });
            }
            let label = view
                .crew
                .iter()
                .find(|c| c.id == id.as_str())
                .map(|c| c.label.clone())
                .unwrap_or_else(|| role.to_string());
            if role == ryter_core::Role::Builder {
                view.lines
                    .push(LogLine::Merge(format!("{label} · {summary}")));
            }
            view.crew.retain(|c| c.id != id.as_str());
        }
        AgentEvent::Session { id, phase, title } => {
            view.session_id = id;
            view.phase = phase;
            if !title.is_empty() {
                view.lines.push(LogLine::System(format!(
                    "session {} · {title}",
                    &view.session_id.chars().take(8).collect::<String>()
                )));
            }
        }
        AgentEvent::Error { message } => {
            view.busy = false;
            view.lines.push(LogLine::System(message));
        }
        AgentEvent::Cancelled => {
            view.busy = false;
            view.crew.clear();
            view.lines.push(LogLine::System("cancelled".into()));
        }
        AgentEvent::Context {
            tokens,
            window,
            pct,
            ..
        } => {
            view.ctx_pct = Some(pct);
            view.ctx_tokens = Some(tokens);
            view.ctx_window = Some(window);
        }
        AgentEvent::Compacted { after, window, .. } => {
            let pct = if window == 0 {
                0
            } else {
                ((after.saturating_mul(100)) / window).min(100) as u8
            };
            view.ctx_pct = Some(pct);
            view.ctx_tokens = Some(after);
            view.ctx_window = Some(window);
        }
        AgentEvent::ModelsListed { models } => {
            if let Some(m) = models.iter().find(|m| m.id == view.model).cloned() {
                apply_model_catalog(view, &m);
            }
            if let Some(crate::view::Overlay::Model {
                items,
                loading,
                assign_role,
                ..
            }) = &mut view.overlay
            {
                if !models.is_empty() {
                    if assign_role.is_some() {
                        let mut v = vec![ryter_core::ModelInfo {
                            id: String::new(),
                            context_length: None,
                            input_per_million: None,
                            output_per_million: None,
                            connection: Some(view.connection.clone()),
                        }];
                        v.extend(models);
                        *items = v;
                    } else {
                        *items = models;
                    }
                }
                *loading = false;
            }
        }
    }
}

fn open_crew_models(
    view: &mut View,
    cfg: &ryter_core::Config,
    work_tx: &mpsc::Sender<Work>,
    role: String,
) {
    let mut items = vec![ryter_core::ModelInfo {
        id: String::new(),
        context_length: None,
        input_per_million: None,
        output_per_million: None,
        connection: Some(view.connection.clone()),
    }];
    let book = PriceBook::from_config(cfg);
    for c in &view.connections {
        if !c.has_key && c.name != view.connection {
            continue;
        }
        let kind = cfg
            .connections
            .get(&c.name)
            .map(|x| x.kind.as_str())
            .unwrap_or("");
        let mut fb = crate::view::fallback_models(kind, &c.model);
        for m in &mut fb {
            if m.input_per_million.is_none() {
                if let Some(r) = book.rates(&m.id) {
                    m.input_per_million = Some(r.input_per_million);
                    m.output_per_million = Some(r.output_per_million);
                }
            }
            m.connection = Some(c.name.clone());
        }
        items.extend(fb);
    }
    view.overlay = Some(crate::view::Overlay::Model {
        filter: String::new(),
        selected: 0,
        items,
        loading: true,
        assign_role: Some(role.clone()),
    });
    let _ = work_tx.send(Work::ListCrewModels);
}

fn last_from_view(view: &View) -> config::LastRoute {
    config::LastRoute {
        connection: view.connection.clone(),
        model: view.model.clone(),
        context_length: view.ctx_window,
        input_per_million: view.price_in,
        output_per_million: view.price_out,
    }
}

fn apply_model_catalog(view: &mut View, m: &ryter_core::ModelInfo) {
    if let Some(w) = m.context_length {
        view.ctx_window = Some(w);
    }
    if let (Some(i), Some(o)) = (m.input_per_million, m.output_per_million) {
        view.price_in = Some(i);
        view.price_out = Some(o);
        view.price_label = ryter_core::format_rates(Some(ryter_core::Rates::per_million(i, o)));
    }
}

fn parse_todos(args: &serde_json::Value) -> Vec<crate::view::TodoRow> {
    let Some(items) = args.get("items").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .map(|item| {
            if let Some(s) = item.as_str() {
                crate::view::TodoRow {
                    title: s.to_string(),
                    status: "pending".into(),
                }
            } else {
                crate::view::TodoRow {
                    title: item
                        .get("title")
                        .or_else(|| item.get("content"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("task")
                        .to_string(),
                    status: item
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("pending")
                        .to_string(),
                }
            }
        })
        .collect()
}

fn display_home_path(cwd: &std::path::Path) -> String {
    let home = std::env::var("HOME").ok().map(std::path::PathBuf::from);
    if let Some(home) = home {
        if let Ok(rel) = cwd.strip_prefix(&home) {
            if rel.as_os_str().is_empty() {
                return "~".into();
            }
            return format!("~/{}", rel.display());
        }
    }
    cwd.display().to_string()
}

#[allow(clippy::too_many_arguments)]
fn worker(
    cfg: ryter_core::Config,
    conn: ryter_core::ConnectionConfig,
    key: Option<String>,
    session: Session,
    cwd: PathBuf,
    home: PathBuf,
    trusted: bool,
    conn_name: String,
    model: String,
    mut always_approve: bool,
    profile: SandboxProfile,
    work_rx: mpsc::Receiver<Work>,
    ev_tx: mpsc::Sender<AgentEvent>,
    cancel: Arc<Cancel>,
    live_status: Arc<std::sync::Mutex<StatusSnapshot>>,
    live_spend: Arc<std::sync::Mutex<String>>,
    user_io: UserIo,
) {
    if let Err(e) = sandbox::apply(profile, &cwd, &home) {
        let _ = ev_tx.send(AgentEvent::Error {
            message: e.to_string(),
        });
        return;
    }
    let rt = match sandbox::runtime(profile) {
        Ok(rt) => rt,
        Err(e) => {
            let _ = ev_tx.send(AgentEvent::Error {
                message: e.to_string(),
            });
            return;
        }
    };
    let mut session_hold = Some(session);
    let mut agent: Option<Agent> = None;
    if let Some(key) = key {
        if let Some(s) = session_hold.take() {
            let notes_dir = s.notes_dir();
            let a = build_agent(
                &cfg,
                conn,
                key,
                s,
                notes_dir,
                &cwd,
                &home,
                trusted,
                conn_name.clone(),
                model.clone(),
                always_approve,
                ev_tx.clone(),
                cancel.clone(),
                user_io.clone(),
            );
            if let Err(e) = a.fire_session_start() {
                let _ = ev_tx.send(AgentEvent::Error {
                    message: e.to_string(),
                });
            }
            refresh_live(&a, &live_status, &live_spend);
            agent = Some(a);
        }
    }
    loop {
        match work_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Work::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => continue,
            Ok(Work::Turn { text, reply }) => {
                if let Some(a) = &mut agent {
                    a.ctx.cancel.reset();
                    let out = match rt.block_on(a.turn(&text)) {
                        Ok(r) => r.text,
                        Err(e) => {
                            let _ = ev_tx.send(AgentEvent::Error {
                                message: e.to_string(),
                            });
                            String::new()
                        }
                    };
                    refresh_live(a, &live_status, &live_spend);
                    if let Some(reply) = reply {
                        let _ = reply.send(out);
                    }
                } else {
                    let _ = ev_tx.send(AgentEvent::Error {
                        message: "no API key — /connections set-key".into(),
                    });
                    if let Some(reply) = reply {
                        let _ = reply.send(String::new());
                    }
                }
            }
            Ok(Work::Handoff { to, note }) => {
                if let Some(a) = &mut agent {
                    let _ = a.handoff(to, &note, None);
                    refresh_live(a, &live_status, &live_spend);
                } else if let Some(s) = &mut session_hold {
                    let _ = s.handoff(to, &note, None);
                    let _ = ev_tx.send(AgentEvent::PhaseChanged { phase: to });
                }
            }
            Ok(Work::SetAuditor(on)) => {
                if let Some(a) = &mut agent {
                    let _ = a.session.set_auditor(on);
                } else if let Some(s) = &mut session_hold {
                    let _ = s.set_auditor(on);
                }
            }
            Ok(Work::SetTools { always }) => {
                always_approve = always;
                if let Some(a) = &mut agent {
                    a.ctx.always_approve = always;
                }
            }
            Ok(Work::New) => {
                if let Some(a) = &mut agent {
                    if let Ok(s) = Session::create(
                        &home,
                        &cwd,
                        Phase::Build,
                        a.connection.clone(),
                        a.model.clone(),
                    ) {
                        a.session = s;
                        a.ctx.notes_dir = a.session.notes_dir();
                        let q = std::sync::Arc::new(std::sync::Mutex::new(
                            ryter_core::queue::TaskQueue::open(a.session.dir.join("tasks.json")),
                        ));
                        a.ctx.queue = q.clone();
                        a.queue = q;
                        let _ = ev_tx.send(AgentEvent::Session {
                            id: a.session.meta.id.to_string(),
                            phase: a.session.meta.phase,
                            title: a.session.meta.title.clone(),
                        });
                    }
                }
            }
            Ok(Work::Resume(id)) => {
                if let Some(a) = &mut agent {
                    match Session::find(&home, Some(&cwd), &id) {
                        Ok(s) => {
                            a.session = s;
                            a.ctx.notes_dir = a.session.notes_dir();
                            let q = std::sync::Arc::new(std::sync::Mutex::new(
                                ryter_core::queue::TaskQueue::open(
                                    a.session.dir.join("tasks.json"),
                                ),
                            ));
                            a.ctx.queue = q.clone();
                            a.queue = q;
                            a.model = a.session.meta.model.clone();
                            a.connection = a.session.meta.connection.clone();
                            refresh_live(a, &live_status, &live_spend);
                            let _ = ev_tx.send(AgentEvent::Session {
                                id: a.session.meta.id.to_string(),
                                phase: a.session.meta.phase,
                                title: a.session.meta.title.clone(),
                            });
                        }
                        Err(e) => {
                            let _ = ev_tx.send(AgentEvent::Error {
                                message: e.to_string(),
                            });
                        }
                    }
                }
            }
            Ok(Work::Kill(id)) => {
                if let Some(a) = &agent {
                    if !a.kill_child(&id) {
                        let _ = ev_tx.send(AgentEvent::Error {
                            message: format!("no running specialist {id}"),
                        });
                    }
                }
            }
            Ok(Work::Rename(title)) => {
                if let Some(a) = &mut agent {
                    let _ = a.session.set_title(&title);
                }
            }
            Ok(Work::Context) => {
                if let Some(a) = &mut agent {
                    let _ = a.emit_context();
                }
            }
            Ok(Work::Compact) => {
                if let Some(a) = &mut agent {
                    let _ = a.compact_now();
                }
            }
            Ok(Work::SetCrew { specialists }) => {
                if let Some(a) = &mut agent {
                    if let Some(c) = &mut a.cfg {
                        c.specialists = specialists;
                    }
                }
            }
            Ok(Work::SetMcp { servers }) => {
                if let Some(a) = &mut agent {
                    if let Some(c) = &mut a.cfg {
                        c.mcp_servers = servers.clone();
                    }
                    a.ctx.mcp = ryter_core::McpHub::connect(&servers)
                        .ok()
                        .map(|h| std::sync::Arc::new(std::sync::Mutex::new(h)));
                }
            }
            Ok(Work::SetHooks { hooks }) => {
                if let Some(a) = &mut agent {
                    if let Some(c) = &mut a.cfg {
                        c.hooks = hooks.clone();
                    }
                    a.ctx.hooks = if hooks.is_empty() {
                        None
                    } else {
                        Some(std::sync::Arc::new(HookSet::from_config(&hooks)))
                    };
                }
            }
            Ok(Work::SetSettings {
                budget_usd,
                max_crew,
                web,
            }) => {
                if let Some(a) = &mut agent {
                    a.budget_usd = budget_usd;
                    a.max_crew = max_crew;
                    a.ctx.web = web;
                    if let Some(c) = &mut a.cfg {
                        c.spend.session_budget_usd = budget_usd;
                        c.subagents.max = max_crew;
                        c.features.web = web;
                    }
                }
            }
            Ok(Work::ListCrewModels) => {
                let book = PriceBook::from_config(&cfg);
                let mut all = Vec::new();
                for (name, conn) in &cfg.connections {
                    let Ok(key) = resolve_secret(&cfg, &ConnectionId::new(name)) else {
                        continue;
                    };
                    let provider = http_provider(conn, key);
                    if let Ok(models) = rt.block_on(provider.list_models()) {
                        for mut m in models {
                            if m.input_per_million.is_none() {
                                if let Some(r) = book.rates(&m.id) {
                                    m.input_per_million = Some(r.input_per_million);
                                    m.output_per_million = Some(r.output_per_million);
                                }
                            }
                            if m.context_length.is_none() {
                                m.context_length = Some(ryter_core::window_for(&m.id));
                            }
                            m.connection = Some(name.clone());
                            all.push(m);
                        }
                    }
                }
                let _ = ev_tx.send(AgentEvent::ModelsListed { models: all });
            }
            Ok(Work::ListModels) => {
                if let Some(a) = &agent {
                    match rt.block_on(a.provider.list_models()) {
                        Ok(models) => {
                            let book = PriceBook::from_config(&cfg);
                            let models: Vec<_> = models
                                .into_iter()
                                .map(|mut m| {
                                    if m.input_per_million.is_none() {
                                        if let Some(r) = book.rates(&m.id) {
                                            m.input_per_million = Some(r.input_per_million);
                                            m.output_per_million = Some(r.output_per_million);
                                        }
                                    }
                                    if m.context_length.is_none() {
                                        m.context_length = Some(ryter_core::window_for(&m.id));
                                    }
                                    m
                                })
                                .collect();
                            let _ = ev_tx.send(AgentEvent::ModelsListed { models });
                        }
                        Err(e) => {
                            let _ = ev_tx.send(AgentEvent::Error {
                                message: format!("models: {e}"),
                            });
                            let _ = ev_tx.send(AgentEvent::ModelsListed { models: vec![] });
                        }
                    }
                } else {
                    let _ = ev_tx.send(AgentEvent::ModelsListed { models: vec![] });
                }
            }
            Ok(Work::Reconnect {
                name,
                model: new_model,
                key: new_key,
            }) => {
                let Some(c) = cfg.connections.get(&name).cloned() else {
                    let _ = ev_tx.send(AgentEvent::Error {
                        message: format!("unknown connection {name}"),
                    });
                    continue;
                };
                if let Some(a) = &mut agent {
                    a.provider = Arc::new(http_provider(&c, new_key));
                    a.connection = name.clone();
                    a.model = new_model.clone();
                    let _ = a.session.set_route(name, new_model);
                } else if let Some(mut s) = session_hold.take() {
                    let _ = s.set_route(name.clone(), new_model.clone());
                    let notes_dir = s.notes_dir();
                    let a = build_agent(
                        &cfg,
                        c,
                        new_key,
                        s,
                        notes_dir,
                        &cwd,
                        &home,
                        trusted,
                        name,
                        new_model,
                        always_approve,
                        ev_tx.clone(),
                        cancel.clone(),
                        user_io.clone(),
                    );
                    if let Err(e) = a.fire_session_start() {
                        let _ = ev_tx.send(AgentEvent::Error {
                            message: e.to_string(),
                        });
                    }
                    refresh_live(&a, &live_status, &live_spend);
                    agent = Some(a);
                }
            }
        }
    }
}

fn persist_mcp(home: &std::path::Path, view: &View, cfg: &mut Config) {
    cfg.mcp.inbound = view.mcp_inbound;
    cfg.mcp.bind = view.mcp_bind.clone();
    if let Some(p) = &view.mcp_listen {
        cfg.mcp.socket = Some(p.clone());
    }
    cfg.mcp_servers = view.mcp_servers.clone();
    let _ = config::save_mcp(home, cfg);
    let tokens: std::collections::BTreeMap<String, String> =
        view.mcp_tokens.iter().cloned().collect();
    let _ = config::save_mcp_tokens(home, &tokens);
}

fn start_mcp_tcp(view: &mut View, host: Arc<dyn InboundHost>) {
    if view.mcp_tcp_listen.is_some() {
        return;
    }
    let Some(bind) = view.mcp_bind.clone() else {
        return;
    };
    let Ok(addr) = bind.parse::<std::net::SocketAddr>() else {
        view.lines
            .push(LogLine::System(format!("bad MCP bind {bind}")));
        return;
    };
    let tokens: Vec<String> = view.mcp_tokens.iter().map(|(_, t)| t.clone()).collect();
    if tokens.is_empty() {
        view.lines
            .push(LogLine::System("create an inbound token first".into()));
        return;
    }
    if let Err(e) = ryter_core::check_tcp(addr, &tokens[0], false) {
        view.lines.push(LogLine::System(e.to_string()));
        return;
    }
    view.mcp_tcp_listen = Some(bind.clone());
    std::thread::spawn(move || {
        let _ = ryter_core::serve_tcp(addr, host, tokens);
    });
    view.lines.push(LogLine::System(format!("MCP tcp {bind}")));
}

fn sanitize_mcp_name(name: &str) -> String {
    let s: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    s.trim_matches('-').to_string()
}

fn mcp_enter(view: &mut View) -> Action {
    let Some(crate::view::Overlay::Mcp { selected, pane }) = &view.overlay else {
        return Action::None;
    };
    let selected = *selected;
    let pane = pane.clone();
    match pane {
        crate::view::McpPane::Home => {
            if selected == 0 {
                view.overlay = Some(crate::view::Overlay::Mcp {
                    selected: 0,
                    pane: crate::view::McpPane::Inbound,
                });
                Action::None
            } else if selected >= 1 && selected <= view.mcp_servers.len() {
                let name = view.mcp_server_names()[selected - 1].clone();
                if let Some(s) = view.mcp_servers.get_mut(&name) {
                    s.enabled = !s.enabled;
                }
                Action::SaveMcp
            } else {
                view.overlay = Some(crate::view::Overlay::Mcp {
                    selected: 0,
                    pane: crate::view::McpPane::AddName { buf: String::new() },
                });
                Action::None
            }
        }
        crate::view::McpPane::Inbound => match selected {
            0 => {
                view.mcp_inbound = !view.mcp_inbound;
                Action::SaveMcp
            }
            1 => {
                view.lines
                    .push(LogLine::System(view.mcp_stdio_cmd().into()));
                Action::None
            }
            2 => {
                let uri = view.mcp_unix_uri().unwrap_or_else(|| {
                    "unix socket not listening (restart with inbound on)".into()
                });
                view.lines.push(LogLine::System(uri));
                Action::None
            }
            3 => {
                if view.mcp_bind.is_some() {
                    view.mcp_bind = None;
                    Action::SaveMcp
                } else {
                    view.mcp_bind = Some("127.0.0.1:8765".into());
                    if view.mcp_tokens.is_empty() {
                        view.mcp_tokens
                            .push(("default".into(), config::new_inbound_token()));
                        view.mcp_reveal = Some("default".into());
                    }
                    Action::McpListenTcp
                }
            }
            4 => {
                let tok = config::new_inbound_token();
                if let Some(row) = view.mcp_tokens.iter_mut().find(|(n, _)| n == "default") {
                    row.1 = tok;
                } else {
                    view.mcp_tokens.push(("default".into(), tok));
                }
                view.mcp_reveal = Some("default".into());
                Action::SaveMcp
            }
            5 => {
                view.lines.push(LogLine::System(view.mcp_client_snippet()));
                Action::None
            }
            _ => Action::None,
        },
        crate::view::McpPane::AddName { buf } => {
            let name = sanitize_mcp_name(&buf);
            if name.is_empty() {
                return Action::None;
            }
            view.overlay = Some(crate::view::Overlay::Mcp {
                selected: 0,
                pane: crate::view::McpPane::AddCommand {
                    name,
                    buf: String::new(),
                },
            });
            Action::None
        }
        crate::view::McpPane::AddCommand { name, buf } => {
            let command = buf.trim().to_string();
            if command.is_empty() {
                return Action::None;
            }
            view.overlay = Some(crate::view::Overlay::Mcp {
                selected: 0,
                pane: crate::view::McpPane::AddArgs {
                    name: name.clone(),
                    command,
                    buf: String::new(),
                },
            });
            Action::None
        }
        crate::view::McpPane::AddArgs { name, command, buf } => {
            let args: Vec<String> = buf.split_whitespace().map(str::to_string).collect();
            view.mcp_servers.insert(
                name.clone(),
                ryter_core::McpServerConfig {
                    command: command.clone(),
                    args,
                    enabled: true,
                    env: std::collections::BTreeMap::new(),
                },
            );
            view.overlay = Some(crate::view::Overlay::Mcp {
                selected: 0,
                pane: crate::view::McpPane::Home,
            });
            Action::SaveMcp
        }
    }
}

fn catalog_source_at(view: &View, selected: usize) -> Option<std::path::PathBuf> {
    if selected < view.catalog.skills.len() {
        return Some(view.catalog.skills[selected].source.clone());
    }
    let i = selected - view.catalog.skills.len();
    if i < view.catalog.commands.len() {
        return Some(view.catalog.commands[i].source.clone());
    }
    None
}

fn skills_enter(view: &mut View) -> Action {
    let Some(crate::view::Overlay::Skills { selected, pane }) = &view.overlay else {
        return Action::None;
    };
    let selected = *selected;
    let pane = pane.clone();
    match pane {
        crate::view::SkillsPane::Home => {
            let n_sk = view.catalog.skills.len();
            let n_cmd = view.catalog.commands.len();
            if selected < n_sk {
                let name = view.catalog.skills[selected].name.clone();
                view.overlay = Some(crate::view::Overlay::Skills {
                    selected: 0,
                    pane: crate::view::SkillsPane::Args {
                        name,
                        buf: String::new(),
                    },
                });
                Action::None
            } else if selected < n_sk + n_cmd {
                let name = view.catalog.commands[selected - n_sk].name.clone();
                view.overlay = Some(crate::view::Overlay::Skills {
                    selected: 0,
                    pane: crate::view::SkillsPane::Args {
                        name,
                        buf: String::new(),
                    },
                });
                Action::None
            } else if selected == n_sk + n_cmd {
                view.overlay = Some(crate::view::Overlay::Skills {
                    selected: 0,
                    pane: crate::view::SkillsPane::AddSkillName { buf: String::new() },
                });
                Action::None
            } else {
                view.overlay = Some(crate::view::Overlay::Skills {
                    selected: 0,
                    pane: crate::view::SkillsPane::AddCmdName { buf: String::new() },
                });
                Action::None
            }
        }
        crate::view::SkillsPane::Args { name, buf } => {
            let args = buf.clone();
            let expanded = view
                .catalog
                .skills
                .iter()
                .find(|s| s.name == name)
                .map(|s| s.expand(&args))
                .or_else(|| {
                    view.catalog
                        .commands
                        .iter()
                        .find(|c| c.name == name)
                        .map(|c| c.expand(&args))
                });
            let Some(expanded) = expanded else {
                return Action::None;
            };
            let shown = if args.trim().is_empty() {
                format!("/{name}")
            } else {
                format!("/{name} {args}")
            };
            view.overlay = None;
            view.lines.push(LogLine::User(shown));
            view.busy = true;
            Action::Submit(expanded)
        }
        crate::view::SkillsPane::AddSkillName { buf } => {
            let name = buf.trim().to_ascii_lowercase();
            if name.is_empty() {
                return Action::None;
            }
            view.overlay = Some(crate::view::Overlay::Skills {
                selected: 0,
                pane: crate::view::SkillsPane::AddSkillDesc {
                    name,
                    buf: String::new(),
                },
            });
            Action::None
        }
        crate::view::SkillsPane::AddSkillDesc { name, buf } => Action::WriteSkill {
            name,
            description: buf.trim().to_string(),
        },
        crate::view::SkillsPane::AddCmdName { buf } => {
            let name = buf.trim().to_ascii_lowercase();
            if name.is_empty() {
                return Action::None;
            }
            Action::WriteCommand { name }
        }
    }
}

fn hooks_enter(view: &mut View) -> Action {
    let Some(crate::view::Overlay::Hooks { selected, pane }) = &view.overlay else {
        return Action::None;
    };
    let selected = *selected;
    let pane = pane.clone();
    match pane {
        crate::view::HooksPane::Home => {
            if selected < view.hooks.len() {
                let h = &view.hooks[selected];
                let mut line = h.event.clone();
                if let Some(c) = &h.command {
                    line.push_str("  command=");
                    line.push_str(c);
                }
                if let Some(u) = &h.url {
                    line.push_str("  url=");
                    line.push_str(u);
                }
                if let Some(m) = &h.matcher {
                    line.push_str("  matcher=");
                    line.push_str(m);
                }
                view.lines.push(LogLine::System(line));
                Action::None
            } else {
                view.overlay = Some(crate::view::Overlay::Hooks {
                    selected: 0,
                    pane: crate::view::HooksPane::AddEvent,
                });
                Action::None
            }
        }
        crate::view::HooksPane::AddEvent => {
            let Some(ev) = ryter_core::HookEvent::all().get(selected) else {
                return Action::None;
            };
            view.overlay = Some(crate::view::Overlay::Hooks {
                selected: 0,
                pane: crate::view::HooksPane::AddTarget {
                    event: ev.as_str().to_string(),
                    buf: String::new(),
                },
            });
            Action::None
        }
        crate::view::HooksPane::AddTarget { event, buf } => {
            let target = buf.trim().to_string();
            if target.is_empty() {
                return Action::None;
            }
            view.overlay = Some(crate::view::Overlay::Hooks {
                selected: 0,
                pane: crate::view::HooksPane::AddMatcher {
                    event,
                    target,
                    buf: String::new(),
                },
            });
            Action::None
        }
        crate::view::HooksPane::AddMatcher { event, target, buf } => {
            let matcher = buf.trim();
            let matcher = if matcher.is_empty() {
                None
            } else {
                Some(matcher.to_string())
            };
            let (command, url) = if target.starts_with("http://") || target.starts_with("https://")
            {
                (None, Some(target))
            } else {
                (Some(target), None)
            };
            view.hooks.push(ryter_core::HookConfig {
                event,
                command,
                url,
                matcher,
            });
            view.overlay = Some(crate::view::Overlay::Hooks {
                selected: 0,
                pane: crate::view::HooksPane::Home,
            });
            Action::SaveHooks
        }
    }
}

fn fill_view_from_session(view: &mut View, session: &Session) {
    view.session_id = session.meta.id.to_string();
    view.phase = session.meta.phase;
    view.spend = session.meta.spend_usd_total;
    view.spend_unknown = session.meta.spend_unknown;
    view.auditor_on = session.meta.auditor_enabled;
    view.connection = session.meta.connection.clone();
    view.model = session.meta.model.clone();
    view.lines.clear();
    for m in &session.transcript {
        match m.role.as_str() {
            "user" if !m.content.is_empty() => view.lines.push(LogLine::User(m.content.clone())),
            "assistant" if !m.content.is_empty() => {
                view.lines.push(LogLine::Assistant(m.content.clone()));
            }
            _ => {}
        }
    }
    let q = ryter_core::queue::TaskQueue::open(session.dir.join("tasks.json"));
    view.todos = q
        .tasks
        .iter()
        .map(|t| crate::view::TodoRow {
            title: t.title.clone(),
            status: format!("{:?}", t.status).to_ascii_lowercase(),
        })
        .collect();
    view.spend_by_role.clear();
    view.spend_by_conn.clear();
    if let Ok(recs) = session.spend_log() {
        for r in recs {
            if let Some(v) = r.total_usd {
                *view.spend_by_role.entry(r.role.to_string()).or_insert(0.0) += v;
                *view.spend_by_conn.entry(r.connection).or_insert(0.0) += v;
            }
        }
    }
}

fn session_choice_items(
    home: &std::path::Path,
    cwd: &std::path::Path,
) -> Vec<crate::view::ChoiceItem> {
    Session::list(home, cwd)
        .unwrap_or_default()
        .into_iter()
        .map(|s| {
            let spend = ryter_core::format_usd(s.meta.spend_usd_total);
            crate::view::ChoiceItem {
                id: s.meta.id.to_string(),
                label: format!("{}  {}  {}  {spend}", s.short_id(), s.preview, s.meta.phase),
            }
        })
        .collect()
}

fn open_session_choice(
    view: &mut View,
    home: &std::path::Path,
    cwd: &std::path::Path,
    kind: crate::view::ChoiceKind,
) {
    let options = session_choice_items(home, cwd);
    if options.is_empty() {
        view.lines.push(LogLine::System(
            "no saved sessions in this directory".into(),
        ));
        view.overlay = None;
        return;
    }
    let title = match kind {
        crate::view::ChoiceKind::DeleteSession => "delete session",
        _ => "resume session",
    };
    let selected = options
        .iter()
        .position(|o| o.id == view.session_id)
        .unwrap_or(0);
    view.overlay = Some(crate::view::Overlay::Choice {
        title: title.into(),
        kind,
        options,
        selected,
        filter: String::new(),
    });
    view.slash = None;
    view.composer.clear();
}

fn apply_resume(
    view: &mut View,
    home: &std::path::Path,
    cwd: &std::path::Path,
    id: &str,
) -> ryter_core::Result<()> {
    let s = Session::find(home, Some(cwd), id)?;
    fill_view_from_session(view, &s);
    view.overlay = None;
    view.busy = false;
    view.crew.clear();
    Ok(())
}

fn delete_session(
    view: &mut View,
    home: &std::path::Path,
    cwd: &std::path::Path,
    id: &str,
    work_tx: &mpsc::Sender<Work>,
) -> ryter_core::Result<String> {
    let s = Session::find(home, Some(cwd), id)?;
    let dir = s.dir.clone();
    let short: String = s.meta.id.as_str().chars().take(8).collect();
    let current = s.meta.id.as_str() == view.session_id;
    Session::remove(&dir)?;
    if current {
        view.lines.clear();
        view.todos.clear();
        view.crew.clear();
        view.spend = None;
        let _ = work_tx.send(Work::New);
    }
    view.overlay = None;
    Ok(format!("deleted session {short}"))
}

fn refresh_live(
    agent: &Agent,
    status: &std::sync::Mutex<StatusSnapshot>,
    spend: &std::sync::Mutex<String>,
) {
    if let Ok(mut s) = status.lock() {
        *s = StatusSnapshot {
            phase: agent.session.meta.phase.to_string(),
            model: agent.model.clone(),
            connection: agent.connection.clone(),
            session: agent.session.meta.id.to_string(),
            last_error: String::new(),
        };
    }
    if let Ok(mut s) = spend.lock() {
        *s = ryter_core::format_usd(agent.session.meta.spend_usd_total);
    }
}

#[allow(clippy::too_many_arguments)]
fn build_agent(
    cfg: &Config,
    conn: ConnectionConfig,
    key: String,
    session: Session,
    notes: PathBuf,
    cwd: &std::path::Path,
    home: &std::path::Path,
    trusted: bool,
    conn_name: String,
    model: String,
    always_approve: bool,
    ev_tx: mpsc::Sender<AgentEvent>,
    cancel: Arc<Cancel>,
    user_io: UserIo,
) -> Agent {
    let provider = http_provider(&conn, key);
    let queue = std::sync::Arc::new(std::sync::Mutex::new(ryter_core::queue::TaskQueue::open(
        session.dir.join("tasks.json"),
    )));
    Agent {
        provider: Arc::new(provider),
        book: PriceBook::from_config(cfg),
        session,
        ctx: ToolContext {
            workspace: cwd.to_path_buf(),
            notes_dir: notes,
            role: Role::Orchestrator,
            always_approve,
            queue: queue.clone(),
            mcp: ryter_core::McpHub::connect(&cfg.mcp_servers)
                .ok()
                .map(|h| std::sync::Arc::new(std::sync::Mutex::new(h))),
            hooks: if cfg.hooks.is_empty() {
                None
            } else {
                Some(Arc::new(HookSet::from_config(&cfg.hooks)))
            },
            cancel,
            user_io: Some(user_io),
            sticky_approve: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            web: cfg.features.web,
        },
        connection: conn_name,
        model,
        role: Role::Orchestrator,
        max_turns: 40,
        budget_usd: cfg.spend.session_budget_usd,
        sink: Some(ev_tx),
        home: home.to_path_buf(),
        project_root: Some(cwd.to_path_buf()),
        trusted,
        queue,
        max_crew: cfg.subagents.max,
        max_retries: cfg.auditor.max_retries,
        context_window: 0,
        cfg: Some(cfg.clone()),
        running: Arc::new(std::sync::Mutex::new(Vec::new())),
    }
}
