//! Agent thread: owns the `Agent`, the tokio runtime, and the sandbox.
//! The UI talks to it over [`Work`]; it answers with [`AgentEvent`]s.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ryter_core::config::resolve_secret;
use ryter_core::ids::ConnectionId;
use ryter_core::llm::{Provider, http_provider};
use ryter_core::sandbox::{self, SandboxProfile};
use ryter_core::session::Session;
use ryter_core::spend::PriceBook;
use ryter_core::tools::ToolContext;
use ryter_core::{
    Agent, AgentEvent, Cancel, Config, ConnectionConfig, HookSet, McpServerConfig, Phase, Role,
    RoleModel, StatusSnapshot, UserIo,
};

/// Requests from the UI thread.
pub enum Work {
    /// Run a user turn. `reply` carries the final text back to an MCP caller.
    Turn {
        /// Prompt.
        text: String,
        /// Optional reply channel.
        reply: Option<mpsc::Sender<String>>,
    },
    /// Fresh session.
    New,
    /// Auditor gate.
    SetAuditor(bool),
    /// Tool permission mode.
    SetTools {
        /// Always approve `Ask`.
        always: bool,
    },
    /// Emit a `Context` event.
    Context,
    /// Compact now.
    Compact,
    /// `GET /models` on the active connection.
    ListModels,
    /// `GET /models` on every keyed connection (crew picker).
    ListCrewModels,
    /// Replace the live specialist table.
    SetCrew {
        /// Assignments.
        specialists: BTreeMap<String, RoleModel>,
    },
    /// Reconnect outbound MCP servers.
    SetMcp {
        /// Servers.
        servers: BTreeMap<String, McpServerConfig>,
    },
    /// Replace the hook set.
    SetHooks {
        /// Hooks.
        hooks: Vec<ryter_core::HookConfig>,
    },
    /// Switch hats, or to the crew's lead.
    SetRole(ryter_core::Role),
    /// `/undo`.
    Undo,
    /// Live settings knobs.
    SetSettings {
        /// Budget cap.
        budget_usd: f64,
        /// Per-task cap.
        task_budget_usd: f64,
        /// Max parallel specialists.
        max_crew: u32,
        /// Web tools.
        web: bool,
    },
    /// Load a saved session.
    Resume(String),
    /// Kill one specialist.
    Kill(String),
    /// Kill every specialist.
    KillAll,
    /// Set the session title.
    Rename(String),
    /// Switch provider / model.
    Reconnect {
        /// Connection name.
        name: String,
        /// Model id.
        model: String,
        /// Resolved key.
        key: String,
    },
    /// Exit the thread.
    Shutdown,
}

/// Everything the worker thread needs at start.
pub struct WorkerInit {
    /// Loaded config.
    pub cfg: Config,
    /// Active connection.
    pub conn: ConnectionConfig,
    /// Resolved key, if any.
    pub key: Option<String>,
    /// Session to drive.
    pub session: Session,
    /// Project root.
    pub cwd: PathBuf,
    /// `~/.ryter`.
    pub home: PathBuf,
    /// Trusted project.
    pub trusted: bool,
    /// Connection name.
    pub conn_name: String,
    /// Model id.
    pub model: String,
    /// Treat `Ask` as `Allow`.
    pub always_approve: bool,
    /// Landlock profile.
    pub profile: SandboxProfile,
    /// Request channel.
    pub work_rx: mpsc::Receiver<Work>,
    /// Event sink.
    pub ev_tx: mpsc::Sender<AgentEvent>,
    /// Shared cancel flag.
    pub cancel: Arc<Cancel>,
    /// Status snapshot for inbound MCP.
    pub live_status: Arc<Mutex<StatusSnapshot>>,
    /// Spend text for inbound MCP.
    pub live_spend: Arc<Mutex<String>>,
    /// Permission / question channel to the UI.
    pub user_io: UserIo,
}

/// Thread body.
pub fn run(init: WorkerInit) {
    let WorkerInit {
        mut cfg,
        conn,
        key,
        session,
        cwd,
        home,
        trusted,
        conn_name,
        model,
        mut always_approve,
        profile,
        work_rx,
        ev_tx,
        cancel,
        live_status,
        live_spend,
        user_io,
    } = init;
    if let Err(e) = sandbox::apply(profile, &cwd, &home) {
        send_err(&ev_tx, e.to_string());
        return;
    }
    let rt = match sandbox::runtime(profile) {
        Ok(rt) => rt,
        Err(e) => {
            send_err(&ev_tx, e.to_string());
            return;
        }
    };
    let mut session_hold = Some(session);
    let mut agent: Option<Agent> = None;
    if let (Some(key), Some(s)) = (key, session_hold.take()) {
        let a = build_agent(BuildAgent {
            cfg: &cfg,
            conn,
            key,
            session: s,
            cwd: &cwd,
            home: &home,
            trusted,
            conn_name: conn_name.clone(),
            model: model.clone(),
            always_approve,
            ev_tx: ev_tx.clone(),
            cancel: cancel.clone(),
            user_io: user_io.clone(),
        });
        if let Err(e) = a.fire_session_start() {
            send_err(&ev_tx, e.to_string());
        }
        emit_mcp_status(&a, &ev_tx);
        refresh_live(&a, &live_status, &live_spend);
        agent = Some(a);
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
                            send_err(&ev_tx, e.to_string());
                            String::new()
                        }
                    };
                    refresh_live(a, &live_status, &live_spend);
                    if let Some(reply) = reply {
                        let _ = reply.send(out);
                    }
                } else {
                    send_err(&ev_tx, "no API key — /provider set-key".into());
                    if let Some(reply) = reply {
                        let _ = reply.send(String::new());
                    }
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
                    match Session::create(
                        &home,
                        &cwd,
                        Phase::Build,
                        a.connection.clone(),
                        a.model.clone(),
                    ) {
                        Ok(mut s) => {
                            // A new session stays in the mode the user is in.
                            let _ = s.set_mode(a.role);
                            swap_session(a, s);
                            let _ = ev_tx.send(session_event(a));
                        }
                        Err(e) => send_err(&ev_tx, e.to_string()),
                    }
                }
            }
            Ok(Work::Resume(id)) => {
                if let Some(a) = &mut agent {
                    match Session::find(&home, Some(&cwd), &id) {
                        Ok(s) => {
                            let role = s.meta.mode.unwrap_or(Role::SoloBuild);
                            swap_session(a, s);
                            a.role = role;
                            a.ctx.role = role;
                            a.model = a.session.meta.model.clone();
                            a.connection = a.session.meta.connection.clone();
                            refresh_live(a, &live_status, &live_spend);
                            let _ = ev_tx.send(session_event(a));
                        }
                        Err(e) => send_err(&ev_tx, e.to_string()),
                    }
                }
            }
            Ok(Work::Kill(id)) => {
                if let Some(a) = &agent {
                    if !a.kill_child(&id) {
                        send_err(&ev_tx, format!("no running specialist {id}"));
                    }
                }
            }
            Ok(Work::KillAll) => {
                if let Some(a) = &agent {
                    a.kill_all_children();
                }
            }
            Ok(Work::Rename(title)) => {
                if let Some(a) = &mut agent {
                    let _ = a.session.set_title(&title);
                } else if let Some(s) = &mut session_hold {
                    let _ = s.set_title(&title);
                }
            }
            Ok(Work::Context) => {
                if let Some(a) = &mut agent {
                    let _ = a.emit_context();
                }
            }
            Ok(Work::Compact) => {
                if let Some(a) = &mut agent {
                    if let Err(e) = a.compact_now() {
                        send_err(&ev_tx, e.to_string());
                    }
                }
            }
            Ok(Work::SetCrew { specialists }) => {
                if let Some(c) = agent.as_mut().and_then(|a| a.cfg.as_mut()) {
                    c.specialists = specialists;
                }
            }
            Ok(Work::SetMcp { servers }) => {
                if let Some(a) = &mut agent {
                    if let Some(c) = &mut a.cfg {
                        c.mcp_servers = servers.clone();
                    }
                    a.ctx.mcp = ryter_core::McpHub::connect(&servers)
                        .ok()
                        .map(|h| Arc::new(Mutex::new(h)));
                    emit_mcp_status(a, &ev_tx);
                } else {
                    // No agent yet: still report per-server status so `/mcp` is honest.
                    if let Ok(h) = ryter_core::McpHub::connect(&servers) {
                        let _ = ev_tx.send(AgentEvent::McpStatus {
                            servers: h
                                .status()
                                .iter()
                                .map(|(k, v)| (k.clone(), v.clone()))
                                .collect(),
                        });
                    }
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
                        Some(Arc::new(HookSet::from_config(&hooks)))
                    };
                }
            }
            Ok(Work::SetRole(role)) => {
                if let Some(a) = &mut agent {
                    a.role = role;
                    a.ctx.role = role;
                    let _ = a.session.set_mode(role);
                }
            }
            Ok(Work::Undo) => {
                let message = match &mut agent {
                    Some(a) => a.undo().unwrap_or_else(|e| format!("undo failed: {e}")),
                    None => "nothing to undo yet".into(),
                };
                let _ = ev_tx.send(AgentEvent::Notice { message });
            }
            Ok(Work::SetSettings {
                budget_usd,
                task_budget_usd,
                max_crew,
                web,
            }) => {
                // The worker's copy too: an agent rebuilt later (a provider
                // switch) starts from it, and used to lose live changes.
                let apply = |c: &mut Config| {
                    c.spend.session_budget_usd = budget_usd;
                    c.spend.task_budget_usd = task_budget_usd;
                    c.subagents.max = max_crew;
                    c.features.web = web;
                };
                apply(&mut cfg);
                if let Some(a) = &mut agent {
                    a.budget_usd = budget_usd;
                    a.max_crew = max_crew;
                    a.ctx.web = web;
                    if let Some(c) = &mut a.cfg {
                        apply(c);
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
                            enrich(&mut m, &book);
                            m.connection = Some(name.clone());
                            all.push(m);
                        }
                    }
                }
                let _ = ev_tx.send(AgentEvent::ModelsListed { models: all });
            }
            Ok(Work::ListModels) => {
                let Some(a) = &agent else {
                    let _ = ev_tx.send(AgentEvent::ModelsListed { models: vec![] });
                    continue;
                };
                match rt.block_on(a.provider.list_models()) {
                    Ok(models) => {
                        let book = PriceBook::from_config(&cfg);
                        let conn = a.connection.clone();
                        let models: Vec<_> = models
                            .into_iter()
                            .map(|mut m| {
                                enrich(&mut m, &book);
                                if m.connection.is_none() {
                                    m.connection = Some(conn.clone());
                                }
                                m
                            })
                            .collect();
                        let _ = ev_tx.send(AgentEvent::ModelsListed { models });
                    }
                    Err(e) => {
                        send_err(&ev_tx, format!("models: {e}"));
                        let _ = ev_tx.send(AgentEvent::ModelsListed { models: vec![] });
                    }
                }
            }
            Ok(Work::Reconnect {
                name,
                model: new_model,
                key: new_key,
            }) => {
                let Some(c) = cfg.connections.get(&name).cloned() else {
                    send_err(&ev_tx, format!("unknown connection {name}"));
                    continue;
                };
                if let Some(a) = &mut agent {
                    a.provider = Arc::new(http_provider(&c, new_key));
                    a.connection = name.clone();
                    a.model = new_model.clone();
                    let _ = a.session.set_route(name, new_model);
                    refresh_live(a, &live_status, &live_spend);
                } else if let Some(mut s) = session_hold.take() {
                    let _ = s.set_route(name.clone(), new_model.clone());
                    let a = build_agent(BuildAgent {
                        cfg: &cfg,
                        conn: c,
                        key: new_key,
                        session: s,
                        cwd: &cwd,
                        home: &home,
                        trusted,
                        conn_name: name,
                        model: new_model,
                        always_approve,
                        ev_tx: ev_tx.clone(),
                        cancel: cancel.clone(),
                        user_io: user_io.clone(),
                    });
                    if let Err(e) = a.fire_session_start() {
                        send_err(&ev_tx, e.to_string());
                    }
                    emit_mcp_status(&a, &ev_tx);
                    refresh_live(&a, &live_status, &live_spend);
                    agent = Some(a);
                }
            }
        }
    }
}

fn send_err(tx: &mpsc::Sender<AgentEvent>, message: String) {
    let _ = tx.send(AgentEvent::Error { message });
}

fn session_event(a: &Agent) -> AgentEvent {
    AgentEvent::Session {
        id: a.session.meta.id.to_string(),
        phase: a.session.meta.phase,
        title: a.session.meta.title.clone(),
    }
}

fn swap_session(a: &mut Agent, s: Session) {
    a.session = s;
    a.ctx.notes_dir = a.session.notes_dir();
    let q = Arc::new(Mutex::new(ryter_core::queue::TaskQueue::open(
        a.session.dir.join("tasks.json"),
    )));
    a.ctx.queue = q.clone();
    a.queue = q;
}

fn enrich(m: &mut ryter_core::ModelInfo, book: &PriceBook) {
    if m.input_per_million.is_none() {
        if let Some(r) = book.rates(&m.id) {
            m.input_per_million = Some(r.input_per_million);
            m.output_per_million = Some(r.output_per_million);
        }
    }
    if m.context_length.is_none() {
        m.context_length = Some(ryter_core::window_for(&m.id));
    }
}

fn emit_mcp_status(a: &Agent, tx: &mpsc::Sender<AgentEvent>) {
    let servers = a
        .ctx
        .mcp
        .as_ref()
        .and_then(|m| m.lock().ok())
        .map(|h| {
            h.status()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default();
    let _ = tx.send(AgentEvent::McpStatus { servers });
}

fn refresh_live(agent: &Agent, status: &Mutex<StatusSnapshot>, spend: &Mutex<String>) {
    if let Ok(mut s) = status.lock() {
        *s = StatusSnapshot {
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

struct BuildAgent<'a> {
    cfg: &'a Config,
    conn: ConnectionConfig,
    key: String,
    session: Session,
    cwd: &'a Path,
    home: &'a Path,
    trusted: bool,
    conn_name: String,
    model: String,
    always_approve: bool,
    ev_tx: mpsc::Sender<AgentEvent>,
    cancel: Arc<Cancel>,
    user_io: UserIo,
}

fn build_agent(b: BuildAgent<'_>) -> Agent {
    let provider = http_provider(&b.conn, b.key);
    let queue = Arc::new(Mutex::new(ryter_core::queue::TaskQueue::open(
        b.session.dir.join("tasks.json"),
    )));
    let notes = b.session.notes_dir();
    // Normal mode's build hat unless the session was left in another mode.
    let role = b.session.meta.mode.unwrap_or(Role::SoloBuild);
    Agent {
        provider: Arc::new(provider),
        book: PriceBook::from_config(b.cfg),
        session: b.session,
        ctx: ToolContext {
            workspace: b.cwd.to_path_buf(),
            notes_dir: notes,
            role,
            always_approve: b.always_approve,
            queue: queue.clone(),
            mcp: ryter_core::McpHub::connect(&b.cfg.mcp_servers)
                .ok()
                .map(|h| Arc::new(Mutex::new(h))),
            hooks: if b.cfg.hooks.is_empty() {
                None
            } else {
                Some(Arc::new(HookSet::from_config(&b.cfg.hooks)))
            },
            cancel: b.cancel,
            user_io: Some(b.user_io),
            sticky_approve: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            web: b.cfg.features.web,
        },
        connection: b.conn_name,
        model: b.model,
        role,
        max_turns: 40,
        budget_usd: b.cfg.spend.session_budget_usd,
        sink: Some(b.ev_tx),
        home: b.home.to_path_buf(),
        project_root: Some(b.cwd.to_path_buf()),
        trusted: b.trusted,
        queue,
        max_crew: b.cfg.subagents.max,
        max_retries: b.cfg.auditor.max_retries,
        checks: b.cfg.auditor.checks.clone(),
        check_timeout_secs: b.cfg.auditor.check_timeout_secs,
        context_window: 0,
        cfg: Some(b.cfg.clone()),
        running: Arc::new(Mutex::new(Vec::new())),
    }
}
