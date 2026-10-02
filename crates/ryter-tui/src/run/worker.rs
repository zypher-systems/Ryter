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
    Agent, AgentEvent, Cancel, Config, ConnectionConfig, HookSet, McpServerConfig, Role, RoleModel,
    StatusSnapshot, UserIo,
};

/// Requests from the UI thread.
pub enum Work {
    /// Stop the product Ryter started for a test. `reply` carries how it
    /// went back to a quit that is waiting on it.
    StopProduct {
        /// Optional reply channel.
        reply: Option<mpsc::Sender<std::result::Result<String, String>>>,
    },
    /// Run a user turn. `reply` carries the final text back to an MCP caller.
    Turn {
        /// Prompt.
        text: String,
        /// Optional reply channel.
        reply: Option<mpsc::Sender<String>>,
        /// Per-request cancellation when this turn came from MCP.
        inbound: Option<Arc<super::InboundTurn>>,
    },
    /// Fresh session.
    New,
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
    /// `GET /models` on every keyed connection (a hat's seat).
    ListAllModels,
    /// Replace each hat's own model.
    SetHats {
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
    /// Switch hats.
    SetRole(ryter_core::Role),
    /// The user's reasoning levels per model changed.
    SetModelReasoning(std::collections::BTreeMap<String, String>),
    /// `/undo [force]`.
    Undo {
        /// Even over the user's later edits.
        force: bool,
    },
    /// `/redo [force]`.
    Redo {
        /// Even over the user's edits since the undo.
        force: bool,
    },
    /// `/audit`: the review hat reviews the uncommitted work.
    ReviewNow,
    /// Offer a review after build turns, or not.
    SetOfferAudit(bool),
    /// Offer a test after a review that passed, or not.
    SetOfferTest(bool),
    /// `/test`: the test hat tests the work now.
    TestNow,
    /// `/changes`: put one file back as `base` had it.
    Revert {
        /// Commit to restore from.
        base: String,
        /// Repository-relative path.
        path: String,
    },
    /// Put one change of a file back.
    RevertHunk {
        /// Commit to restore from.
        base: String,
        /// Repository-relative path.
        path: String,
        /// Which change.
        hunk: usize,
    },
    /// `/commit`: draft a message for these paths.
    DraftCommit(Vec<String>),
    /// `/commit`: commit these paths.
    Commit {
        /// Repository-relative paths.
        paths: Vec<String>,
        /// Full message, receipt included.
        message: String,
    },
    /// Live settings knobs.
    SetSettings {
        /// Budget cap.
        budget_usd: f64,
        /// Most one review may spend.
        review_usd: f64,
        /// Web tools.
        web: bool,
        /// Open pages the model shows in the browser.
        open_pages: bool,
    },
    /// Load a saved session.
    Resume(String),
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
    /// Bounded active conversation for inbound MCP.
    pub live_transcript: Arc<Mutex<String>>,
    /// Permission / question channel to the UI.
    pub user_io: UserIo,
}

/// How long a model list may take before the picker says the provider is
/// slow. The download goes on: it is what fills the cache for next time.
const MODELS_SLOW: Duration = Duration::from_secs(30);
/// How long a model list may take at all. OpenRouter's catalog has taken
/// over a minute to trickle in.
const MODELS_TIMEOUT: Duration = Duration::from_secs(300);

/// Model lists fetched off the worker's thread, one per connection.
struct Fetched {
    /// From every connection (a hat's seat), not the active one alone.
    all: bool,
    results: Vec<(String, Result<Vec<ryter_core::ModelInfo>, String>)>,
}

/// Fetch each connection's model list on a thread of its own, so a slow
/// catalog never holds up a turn, and send what came back.
fn fetch_models(targets: Vec<(String, Arc<dyn Provider>)>, all: bool, tx: mpsc::Sender<Fetched>) {
    std::thread::spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        let results = targets
            .into_iter()
            .map(|(name, p)| {
                // The timer is made inside the runtime: made outside it, it
                // panicked and took the thread down with no word.
                let got = rt.block_on(async {
                    tokio::time::timeout(MODELS_TIMEOUT, p.list_models()).await
                });
                let r = match got {
                    Ok(Ok(m)) => Ok(m),
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(_) => Err(format!("no full answer in {}s", MODELS_TIMEOUT.as_secs())),
                };
                (name, r)
            })
            .collect();
        let _ = tx.send(Fetched { all, results });
    });
}

/// Rows ready for the picker: priced from the book, tagged with their
/// connection.
fn prepared(
    models: Vec<ryter_core::ModelInfo>,
    connection: &str,
    book: &PriceBook,
) -> Vec<ryter_core::ModelInfo> {
    models
        .into_iter()
        .map(|mut m| {
            enrich(&mut m, book);
            if m.connection.is_none() {
                m.connection = Some(connection.to_string());
            }
            m
        })
        .collect()
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
        live_transcript,
        user_io,
    } = init;
    if let Some(scope) = sandbox::Scope::for_profile(profile, &home) {
        if let Err(e) = scope.check(&cwd, &session.notes_dir()) {
            send_err(&ev_tx, e.to_string());
            return;
        }
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
            profile,
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
        refresh_live(&a, &live_status, &live_spend, &live_transcript);
        let _ = ev_tx.send(a.checkpoint_event());
        let mut a = a;
        // A product an earlier session left running is still Ryter's to stop.
        let _ = a.announce_product();
        agent = Some(a);
    }
    let (fetch_tx, fetch_rx) = mpsc::channel::<Fetched>();
    // Fetches in flight, and when each began: a second `/models` waits on
    // the first, and a slow one is said to be slow.
    let mut fetching: BTreeMap<(bool, String), (std::time::Instant, bool)> = BTreeMap::new();
    loop {
        for ((_, name), (since, told)) in fetching.iter_mut() {
            if !*told && since.elapsed() >= MODELS_SLOW {
                *told = true;
                let note = match ryter_core::llm::model_cache::load(&home, name) {
                    Some((_, secs)) => format!(
                        "{name} slow · list from {} ago, still downloading",
                        ryter_core::llm::model_cache::age(secs)
                    ),
                    None => format!("{name} slow · still downloading"),
                };
                let _ = ev_tx.send(AgentEvent::ModelsNote { note: Some(note) });
            }
        }
        while let Ok(done) = fetch_rx.try_recv() {
            let book = PriceBook::from_config(&cfg);
            let mut all = Vec::new();
            let mut notes = Vec::new();
            for (name, result) in done.results {
                fetching.remove(&(done.all, name.clone()));
                match result {
                    Ok(models) => {
                        ryter_core::llm::model_cache::save(&home, &name, &models);
                        all.extend(prepared(models, &name, &book));
                    }
                    Err(e) => {
                        // Keep what the picker already shows: the cached list.
                        match ryter_core::llm::model_cache::load(&home, &name) {
                            Some((models, secs)) => {
                                notes.push(format!(
                                    "{name} slow · list from {} ago",
                                    ryter_core::llm::model_cache::age(secs)
                                ));
                                all.extend(prepared(models, &name, &book));
                            }
                            None => {
                                notes.push(format!("{name} didn't answer · try again"));
                                send_err(
                                    &ev_tx,
                                    format!(
                                        "{name}'s model list didn't come ({e}). Its catalog \
                                         endpoint is slow right now; /models again in a moment."
                                    ),
                                );
                            }
                        }
                    }
                }
            }
            if let Some(a) = &mut agent {
                a.book.ingest_model_info(&all);
            }
            let _ = ev_tx.send(AgentEvent::ModelsListed { models: all });
            let _ = ev_tx.send(AgentEvent::ModelsNote {
                note: (!notes.is_empty()).then(|| notes.join("; ")),
            });
        }
        match work_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Work::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => continue,
            Ok(Work::Turn {
                text,
                reply,
                inbound,
            }) => {
                if let Some(a) = &mut agent {
                    if let Some(ticket) = &inbound {
                        if !ticket.start(&a.ctx.cancel) {
                            continue;
                        }
                    } else {
                        a.ctx.cancel.reset();
                    }
                    let before = a.session.changed_turns;
                    let out = match rt.block_on(a.turn(&text)) {
                        Ok(r) => {
                            // A build turn that finished and changed files:
                            // offer a review. Not for a turn another program
                            // asked for over MCP.
                            let changed = a.session.changed_turns > before;
                            if changed
                                && reply.is_none()
                                && r.reason == ryter_core::StopReason::Completed
                            {
                                if let Err(e) = rt.block_on(a.offer_review()) {
                                    send_err(&ev_tx, e.to_string());
                                }
                            }
                            r.text
                        }
                        // The agent reported it, before closing the turn.
                        Err(_) => String::new(),
                    };
                    if let Some(ticket) = &inbound {
                        ticket.finish();
                    }
                    refresh_live(a, &live_status, &live_spend, &live_transcript);
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
            Ok(Work::SetTools { always }) => {
                always_approve = always;
                if let Some(a) = &mut agent {
                    a.ctx.always_approve = always;
                }
            }
            Ok(Work::New) => {
                if let Some(a) = &mut agent {
                    match Session::create(&home, &cwd, a.connection.clone(), a.model.clone()) {
                        Ok(mut s) => {
                            // A new session stays in the mode the user is in.
                            let _ = s.set_mode(a.role);
                            swap_session(a, s);
                            let _ = ev_tx.send(session_event(a));
                            let _ = ev_tx.send(a.checkpoint_event());
                        }
                        Err(e) => send_err(&ev_tx, e.to_string()),
                    }
                }
            }
            Ok(Work::Resume(id)) => {
                if let Some(a) = &mut agent {
                    match Session::find(&home, Some(&cwd), &id) {
                        Ok(s) => {
                            // A session left in crew mode, before it was
                            // removed, opens in build.
                            let role = s.meta.mode.map_or(Role::SoloBuild, Role::hat);
                            swap_session(a, s);
                            let _ = a.put_on(role);
                            a.model = a.session.meta.model.clone();
                            a.connection = a.session.meta.connection.clone();
                            refresh_live(a, &live_status, &live_spend, &live_transcript);
                            let _ = ev_tx.send(session_event(a));
                            let _ = ev_tx.send(a.checkpoint_event());
                        }
                        Err(e) => send_err(&ev_tx, e.to_string()),
                    }
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
            Ok(Work::SetHats { specialists }) => {
                // The worker's copy too: an agent rebuilt later (a provider
                // switch) starts from it.
                cfg.specialists = specialists.clone();
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
            Ok(Work::SetModelReasoning(levels)) => {
                cfg.model_reasoning = levels.clone();
                if let Some(c) = agent.as_mut().and_then(|a| a.cfg.as_mut()) {
                    c.model_reasoning = levels;
                }
            }
            Ok(Work::SetRole(role)) => {
                if let Some(a) = &mut agent {
                    let _ = a.put_on(role);
                }
                let _ = ev_tx.send(AgentEvent::HatSet { role });
            }
            Ok(Work::SetOfferAudit(on)) => {
                cfg.ui.offer_audit = on;
                if let Some(c) = agent.as_mut().and_then(|a| a.cfg.as_mut()) {
                    c.ui.offer_audit = on;
                }
            }
            Ok(Work::SetOfferTest(on)) => {
                cfg.ui.offer_test = on;
                if let Some(c) = agent.as_mut().and_then(|a| a.cfg.as_mut()) {
                    c.ui.offer_test = on;
                }
            }
            Ok(Work::TestNow) => {
                if let Some(a) = &mut agent {
                    a.ctx.cancel.reset();
                    if let Err(e) = rt.block_on(a.test_now()) {
                        send_err(&ev_tx, e.to_string());
                    }
                    refresh_live(a, &live_status, &live_spend, &live_transcript);
                } else {
                    send_err(&ev_tx, "no API key — /provider set-key".into());
                }
            }
            Ok(Work::ReviewNow) => {
                if let Some(a) = &mut agent {
                    a.ctx.cancel.reset();
                    if let Err(e) = rt.block_on(a.review_now()) {
                        send_err(&ev_tx, e.to_string());
                    }
                    refresh_live(a, &live_status, &live_spend, &live_transcript);
                } else {
                    send_err(&ev_tx, "no API key — /provider set-key".into());
                }
            }
            Ok(Work::StopProduct { reply }) => {
                let out = match &mut agent {
                    Some(a) => {
                        a.ctx.cancel.reset();
                        a.stop_product().unwrap_or_else(|e| Err(e.to_string()))
                    }
                    None => Err("Ryter has not started this project".to_string()),
                };
                match (&out, &reply) {
                    (Ok(did), _) => {
                        let _ = ev_tx.send(AgentEvent::Notice {
                            message: format!("the project was stopped ({did})"),
                        });
                    }
                    (Err(why), _) => {
                        send_err(&ev_tx, format!("the project was not stopped: {why}"))
                    }
                }
                if let Some(reply) = reply {
                    let _ = reply.send(out);
                }
            }
            Ok(Work::Undo { force }) => {
                let message = match &mut agent {
                    Some(a) => {
                        let r = if force { a.undo_force() } else { a.undo() };
                        r.unwrap_or_else(|e| format!("undo failed: {e}"))
                    }
                    None => "nothing to undo yet".into(),
                };
                let _ = ev_tx.send(AgentEvent::Notice { message });
            }
            Ok(Work::Redo { force }) => {
                let message = match &mut agent {
                    Some(a) => {
                        let r = if force { a.redo_force() } else { a.redo() };
                        r.unwrap_or_else(|e| format!("redo failed: {e}"))
                    }
                    None => "nothing to redo".into(),
                };
                let _ = ev_tx.send(AgentEvent::Notice { message });
            }
            Ok(Work::RevertHunk { base, path, hunk }) => {
                let result = match &mut agent {
                    Some(a) => a.revert_hunk(&base, &path, hunk),
                    None => ryter_core::review::revert_hunk(&cwd, &base, &path, hunk),
                };
                let _ = ev_tx.send(AgentEvent::Reverted {
                    path: format!("{path} (change {})", hunk + 1),
                    error: result.err().map(|e| e.to_string()),
                });
            }
            Ok(Work::Revert { base, path }) => {
                let result = match &mut agent {
                    Some(a) => a.revert_file(&base, &path),
                    None => ryter_core::review::revert_file(&cwd, &base, &path),
                };
                let _ = ev_tx.send(AgentEvent::Reverted {
                    path,
                    error: result.err().map(|e| e.to_string()),
                });
            }
            Ok(Work::DraftCommit(paths)) => {
                let (message, error) = match &mut agent {
                    Some(a) => match rt.block_on(a.draft_commit(&paths)) {
                        Ok(m) => (Some(m), None),
                        Err(e) => (None, Some(e.to_string())),
                    },
                    None => (
                        None,
                        Some("no model connected; write the message yourself".into()),
                    ),
                };
                let _ = ev_tx.send(AgentEvent::CommitDraft { message, error });
            }
            Ok(Work::Commit { paths, message }) => {
                let (summary, error) = match ryter_core::review::commit(&cwd, &paths, &message) {
                    Ok(s) => (Some(s), None),
                    Err(e) => (None, Some(e.to_string())),
                };
                let _ = ev_tx.send(AgentEvent::Committed { summary, error });
            }
            Ok(Work::SetSettings {
                budget_usd,
                review_usd,
                web,
                open_pages,
            }) => {
                // The worker's copy too: an agent rebuilt later (a provider
                // switch) starts from it, and used to lose live changes.
                let apply = |c: &mut Config| {
                    c.spend.session_budget_usd = budget_usd;
                    c.spend.review_usd = review_usd;
                    c.features.web = web;
                    c.ui.open_pages = open_pages;
                };
                apply(&mut cfg);
                if let Some(a) = &mut agent {
                    a.budget_usd = budget_usd;
                    a.ctx.web = web;
                    if let Some(c) = &mut a.cfg {
                        apply(c);
                    }
                }
            }
            Ok(Work::ListAllModels) => {
                let book = PriceBook::from_config(&cfg);
                let mut shown = Vec::new();
                let mut oldest = 0u64;
                let mut targets: Vec<(String, Arc<dyn Provider>)> = Vec::new();
                for (name, conn) in &cfg.connections {
                    let Ok(key) = resolve_secret(&cfg, &ConnectionId::new(name)) else {
                        continue;
                    };
                    if let Some((models, secs)) = ryter_core::llm::model_cache::load(&home, name) {
                        oldest = oldest.max(secs);
                        shown.extend(prepared(models, name, &book));
                    }
                    if let std::collections::btree_map::Entry::Vacant(slot) =
                        fetching.entry((true, name.clone()))
                    {
                        slot.insert((std::time::Instant::now(), false));
                        targets.push((name.clone(), Arc::new(http_provider(conn, key))));
                    }
                }
                if !shown.is_empty() {
                    let _ = ev_tx.send(AgentEvent::ModelsListed { models: shown });
                    let _ = ev_tx.send(AgentEvent::ModelsNote {
                        note: Some(format!(
                            "from {} ago · refreshing",
                            ryter_core::llm::model_cache::age(oldest)
                        )),
                    });
                }
                if !targets.is_empty() {
                    fetch_models(targets, true, fetch_tx.clone());
                }
            }
            Ok(Work::ListModels) => {
                let Some(a) = &agent else {
                    let _ = ev_tx.send(AgentEvent::ModelsListed { models: vec![] });
                    continue;
                };
                let name = a.connection.clone();
                // The last list at once; the fresh one behind it.
                if let Some((models, secs)) = ryter_core::llm::model_cache::load(&home, &name) {
                    let book = PriceBook::from_config(&cfg);
                    let _ = ev_tx.send(AgentEvent::ModelsListed {
                        models: prepared(models, &name, &book),
                    });
                    let _ = ev_tx.send(AgentEvent::ModelsNote {
                        note: Some(format!(
                            "from {} ago · refreshing",
                            ryter_core::llm::model_cache::age(secs)
                        )),
                    });
                }
                if let std::collections::btree_map::Entry::Vacant(slot) =
                    fetching.entry((false, name.clone()))
                {
                    slot.insert((std::time::Instant::now(), false));
                    // A client of its own, made on the thread that uses it. The
                    // agent's client keeps its connections on the worker's
                    // runtime, which only runs while the worker is busy; a
                    // fetch from another thread stalled on them for good.
                    let provider: Arc<dyn Provider> = match cfg.connections.get(&name) {
                        Some(c) => {
                            let key =
                                resolve_secret(&cfg, &ConnectionId::new(&name)).unwrap_or_default();
                            Arc::new(http_provider(c, key))
                        }
                        None => a.provider.clone(),
                    };
                    fetch_models(vec![(name, provider)], false, fetch_tx.clone());
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
                    refresh_live(a, &live_status, &live_spend, &live_transcript);
                } else if let Some(mut s) = session_hold.take() {
                    let _ = s.set_route(name.clone(), new_model.clone());
                    let a = build_agent(BuildAgent {
                        cfg: &cfg,
                        profile,
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
                    refresh_live(&a, &live_status, &live_spend, &live_transcript);
                    let _ = ev_tx.send(a.checkpoint_event());
                    let mut a = a;
                    // As at startup: a product an earlier session left
                    // running is still this one's to stop.
                    let _ = a.announce_product();
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
        title: a.session.meta.title.clone(),
    }
}

fn swap_session(a: &mut Agent, s: Session) {
    a.session = s;
    a.announce_recovery();
    a.ctx.notes_dir = a.session.notes_dir();
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

fn refresh_live(
    agent: &Agent,
    status: &Mutex<StatusSnapshot>,
    spend: &Mutex<String>,
    transcript: &Mutex<String>,
) {
    let text = ryter_core::mcp::transcript_snapshot(&agent.session.transcript);
    if let Ok(mut saved) = transcript.lock() {
        *saved = text;
    }
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
    profile: sandbox::SandboxProfile,
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
    let notes = b.session.notes_dir();
    // The build hat, unless the session was left in another. A session
    // left in crew mode, before it was removed, opens in build too.
    let role = b.session.meta.mode.map_or(Role::SoloBuild, Role::hat);
    Agent {
        provider: Arc::new(provider),
        book: PriceBook::from_config(b.cfg),
        session: b.session,
        ctx: ToolContext {
            sandbox: sandbox::Scope::for_profile(b.profile, b.home),
            live: None,
            workspace: b.cwd.to_path_buf(),
            notes_dir: notes,
            role,
            always_approve: b.always_approve,
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
            allowed: Default::default(),
            web: b.cfg.features.web,
            cwd: Default::default(),
            vars: Default::default(),
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
        context_window: 0,
        cfg: Some(b.cfg.clone()),
        machine: ryter_core::prompt::machine_for(b.profile),
        product: None,
        filed: Default::default(),
    }
}
