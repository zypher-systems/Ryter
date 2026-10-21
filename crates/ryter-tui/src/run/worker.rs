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
        /// The mode.
        mode: ryter_core::ToolsMode,
    },
    /// Durable `[permissions]` rules from `/permissions`.
    SetPermissions {
        /// The rules.
        permissions: ryter_core::permissions::Permissions,
    },
    /// Emit a `Context` event.
    Context,
    /// Compact now.
    Compact,
    /// `GET /models` on the active connection.
    ListModels,
    /// `GET /models` on every keyed connection (a hat's seat).
    ListAllModels,
    /// The connections changed in `/provider` (one added or removed): the
    /// worker's copy, which `ListAllModels` and `Reconnect` read.
    SetConnections(BTreeMap<String, ConnectionConfig>),
    /// The price catalog was pulled: price with it from here on.
    ReloadPrices,
    /// Replace each hat's own model.
    SetHats {
        /// Assignments.
        hats: BTreeMap<String, RoleModel>,
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
    /// `/changes`: put one file back as `base` had it.
    Revert {
        /// Commit to restore from.
        base: String,
        /// Repository-relative path.
        path: String,
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
        /// Rounds one message may use; `0` lifts the cap.
        rounds: u32,
        /// Most one review may spend.
        audit_usd: f64,
        /// Web tools.
        web: bool,
        /// Open pages the model shows in the browser.
        open_pages: bool,
    },
    /// A search provider's key was saved: read `[search]` and its key again.
    ReloadSearch,
    /// `[search]` was chosen in the TUI: the worker's copy, and the
    /// session's search, follow.
    SetSearch(ryter_core::config::SearchConfig),
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
    /// A yes to every question.
    pub yolo: bool,
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
    /// Per connection: its name, the generation of the fetch (the slot's
    /// at the time it started), and the list or the error.
    results: Vec<(String, u64, Result<Vec<ryter_core::ModelInfo>, String>)>,
}

/// Fetch each connection's model list on a thread of its own, so a slow
/// catalog never holds up a turn, and send what came back.
fn fetch_models(targets: Vec<(String, u64, Arc<dyn Provider>)>, tx: mpsc::Sender<Fetched>) {
    std::thread::spawn(move || {
        // A thread that could not report would leave its connections
        // marked as fetching for good, and every later `/models` waiting
        // on a list that never comes.
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                let results = targets
                    .into_iter()
                    .map(|(name, g, _)| (name, g, Err(format!("no runtime for the fetch: {e}"))))
                    .collect();
                let _ = tx.send(Fetched { results });
                return;
            }
        };
        let results = targets
            .into_iter()
            .map(|(name, g, p)| {
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
                (name, g, r)
            })
            .collect();
        let _ = tx.send(Fetched { results });
    });
}

/// The picker's note after a batch of lists: what stands against any
/// connection (it didn't answer, or its stale list was kept), from this
/// batch or an earlier one; cleared when nothing stands and nothing is in
/// flight, as the "refreshing" note has done its work; left as it is
/// while another connection's list is still coming.
fn note_after(standing: &BTreeMap<String, String>, in_flight: bool) -> Option<Option<String>> {
    if !standing.is_empty() {
        Some(Some(
            standing.values().cloned().collect::<Vec<_>>().join("; "),
        ))
    } else if !in_flight {
        Some(None)
    } else {
        None
    }
}

/// The price book for a session on `connection`: the shipped and pulled
/// tables, with that connection's own model list (as last cached) over
/// them for the models they don't know. One connection's list, not every
/// connection's: the book keeps the first rate it sees for an id, and one
/// id can be on two connections at two prices.
pub(super) fn book_for(cfg: &Config, home: &Path, connection: &str) -> PriceBook {
    let mut book = PriceBook::load(cfg, home);
    if let Some((models, _)) = ryter_core::llm::model_cache::load(home, connection) {
        book.ingest_model_info(&models);
    }
    book
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
        mut yolo,
        profile,
        work_rx,
        ev_tx,
        cancel,
        live_status,
        live_spend,
        live_transcript,
        user_io,
    } = init;
    // Before a provider is connected there is no agent, and the panels can
    // still revert and commit: that Git work gets this scope.
    let idle_scope = sandbox::Scope::for_profile(profile, &home);
    let idle_notes = session.notes_dir();
    if let Some(scope) = &idle_scope {
        if let Err(e) = scope.check(&cwd, &idle_notes) {
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
            yolo,
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
    // the first, and a slow one is said to be slow. One fetch per
    // connection at a time, whichever opening asked (startup's, or a hat's
    // seat): a second answer would overwrite the first's cache, book and
    // note, and an older one could land last. Each fetch has a generation,
    // and only the one the slot holds is current: a connection removed and
    // added again has a new fetch out, and the old thread's answer is
    // nobody's.
    let mut fetching: BTreeMap<String, (std::time::Instant, bool, u64)> = BTreeMap::new();
    let mut generation = 0u64;
    // What the picker's note says of each connection whose last answer was
    // no list or a stale one, until that connection answers again: a later
    // batch that did not include it does not clear it.
    let mut standing: BTreeMap<String, String> = BTreeMap::new();
    loop {
        for (name, (since, told, _)) in fetching.iter_mut() {
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
        match work_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Work::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
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
                    let out = match rt.block_on(a.turn(&text)) {
                        Ok(r) => r.text,
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
            Ok(Work::ReloadSearch) => {
                if let Some(a) = &mut agent {
                    a.ctx.search = ryter_core::tools::Search::from_config(&cfg.search);
                }
            }
            Ok(Work::SetSearch(search)) => {
                cfg.search = search.clone();
                if let Some(a) = &mut agent {
                    a.ctx.search = ryter_core::tools::Search::from_config(&cfg.search);
                    if let Some(c) = &mut a.cfg {
                        c.search = search;
                    }
                }
            }
            Ok(Work::SetTools { mode }) => {
                always_approve = mode != ryter_core::ToolsMode::Ask;
                yolo = mode == ryter_core::ToolsMode::Yolo;
                if let Some(a) = &mut agent {
                    a.ctx.always_approve = always_approve;
                    a.ctx.yolo = yolo;
                }
            }
            Ok(Work::SetPermissions { permissions }) => {
                cfg.permissions = permissions.clone();
                if let Some(a) = &mut agent {
                    a.ctx.permissions = Arc::new(permissions.clone());
                    if let Some(c) = &mut a.cfg {
                        c.permissions = permissions;
                    }
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
                            let role = s.meta.mode.map_or(Role::Build, Role::hat);
                            swap_session(a, s);
                            let _ = a.put_on(role);
                            a.model = a.session.meta.model.clone();
                            // A session saved under a connection's old
                            // name runs on the new one, as at startup.
                            a.connection =
                                ryter_core::canonical_connection(&a.session.meta.connection)
                                    .to_string();
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
            Ok(Work::SetHats { hats }) => {
                // The worker's copy too: an agent rebuilt later (a provider
                // switch) starts from it.
                cfg.hats = hats.clone();
                if let Some(c) = agent.as_mut().and_then(|a| a.cfg.as_mut()) {
                    c.hats = hats;
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
            Ok(Work::Revert { base, path }) => {
                let result = match &mut agent {
                    Some(a) => a.revert_file(&base, &path),
                    None => scoped(&idle_scope, &cwd, &idle_notes, || {
                        ryter_core::review::revert_file(&cwd, &base, &path)
                    }),
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
                // The repository's hooks run in a commit: scoped like a tool.
                let result = match &agent {
                    Some(a) => a.commit(&paths, &message),
                    None => scoped(&idle_scope, &cwd, &idle_notes, || {
                        ryter_core::review::commit(&cwd, &paths, &message)
                    }),
                };
                let (summary, error) = match result {
                    Ok(s) => (Some(s), None),
                    Err(e) => (None, Some(e.to_string())),
                };
                let _ = ev_tx.send(AgentEvent::Committed { summary, error });
            }
            Ok(Work::SetSettings {
                budget_usd,
                rounds,
                audit_usd,
                web,
                open_pages,
            }) => {
                // The worker's copy too: an agent rebuilt later (a provider
                // switch) starts from it, and used to lose live changes.
                let apply = |c: &mut Config| {
                    c.spend.session_budget_usd = budget_usd;
                    c.limits.rounds = rounds;
                    c.spend.audit_usd = audit_usd;
                    c.features.web = web;
                    c.ui.open_pages = open_pages;
                };
                apply(&mut cfg);
                if let Some(a) = &mut agent {
                    a.budget_usd = budget_usd;
                    a.max_turns = ryter_core::config::rounds_cap(rounds);
                    a.ctx.web = web;
                    if let Some(c) = &mut a.cfg {
                        apply(c);
                    }
                }
            }
            Ok(Work::ListAllModels) => {
                let book = PriceBook::load(&cfg, &home);
                let mut shown = Vec::new();
                let mut oldest = 0u64;
                let mut targets: Vec<(String, u64, Arc<dyn Provider>)> = Vec::new();
                let mut keyed = 0usize;
                let mut cached = Vec::new();
                for (name, conn) in &cfg.connections {
                    let Ok(key) = resolve_secret(&cfg, &ConnectionId::new(name)) else {
                        continue;
                    };
                    keyed += 1;
                    if let Some((models, secs)) = ryter_core::llm::model_cache::load(&home, name) {
                        oldest = oldest.max(secs);
                        cached.push(name.clone());
                        shown.extend(prepared(models, name, &book));
                    }
                    if let std::collections::btree_map::Entry::Vacant(slot) =
                        fetching.entry(name.clone())
                    {
                        generation += 1;
                        slot.insert((std::time::Instant::now(), false, generation));
                        targets.push((
                            name.clone(),
                            generation,
                            Arc::new(http_provider(conn, key)),
                        ));
                    }
                }
                if !shown.is_empty() {
                    let _ = ev_tx.send(AgentEvent::ModelsListed {
                        models: shown,
                        covered: cached,
                        failed: vec![],
                    });
                    let _ = ev_tx.send(AgentEvent::ModelsNote {
                        note: Some(format!(
                            "from {} ago · refreshing",
                            ryter_core::llm::model_cache::age(oldest)
                        )),
                    });
                } else if keyed == 0 {
                    // No connection with a key: the panel keeps its
                    // built-in rows and stops saying "loading". With keys
                    // and no cache, the lists are on their way (this
                    // opening's, or an earlier one's still in flight), and
                    // the panel keeps saying so until they come.
                    let _ = ev_tx.send(AgentEvent::ModelsListed {
                        models: vec![],
                        covered: vec![],
                        failed: cfg.connections.keys().cloned().collect(),
                    });
                }
                if !targets.is_empty() {
                    fetch_models(targets, fetch_tx.clone());
                }
                // A connection that has no key now is not asked again: a
                // note about its last answer would stand for good.
                standing.retain(|n, _| {
                    cfg.connections.contains_key(n)
                        && resolve_secret(&cfg, &ConnectionId::new(n)).is_ok()
                });
            }
            Ok(Work::SetConnections(connections)) => {
                standing.retain(|n, _| connections.contains_key(n));
                fetching.retain(|n, _| connections.contains_key(n));
                cfg.connections = connections;
            }
            Ok(Work::ReloadPrices) => {
                if let Some(a) = &mut agent {
                    a.book = book_for(&cfg, &home, &a.connection);
                }
            }
            Ok(Work::ListModels) => {
                let Some(a) = &agent else {
                    // No agent: no connection has a key yet, so no list is
                    // coming for any of them.
                    let _ = ev_tx.send(AgentEvent::ModelsListed {
                        models: vec![],
                        covered: vec![],
                        failed: cfg.connections.keys().cloned().collect(),
                    });
                    continue;
                };
                let name = a.connection.clone();
                // The last list at once; the fresh one behind it.
                if let Some((models, secs)) = ryter_core::llm::model_cache::load(&home, &name) {
                    let book = PriceBook::load(&cfg, &home);
                    let _ = ev_tx.send(AgentEvent::ModelsListed {
                        models: prepared(models, &name, &book),
                        covered: vec![name.clone()],
                        failed: vec![],
                    });
                    let _ = ev_tx.send(AgentEvent::ModelsNote {
                        note: Some(format!(
                            "from {} ago · refreshing",
                            ryter_core::llm::model_cache::age(secs)
                        )),
                    });
                }
                if let std::collections::btree_map::Entry::Vacant(slot) =
                    fetching.entry(name.clone())
                {
                    generation += 1;
                    slot.insert((std::time::Instant::now(), false, generation));
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
                    fetch_models(vec![(name, generation, provider)], fetch_tx.clone());
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
                    // The session's turns are priced on the connection
                    // they run on: the book takes that connection's list.
                    a.book = book_for(&cfg, &home, &name);
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
                        yolo,
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
        // The lists that came, after the work: a `SetConnections` sent
        // before a result arrived is applied first, so the result of a
        // fetch for a connection removed meanwhile is nobody's.
        while let Ok(done) = fetch_rx.try_recv() {
            let book = PriceBook::load(&cfg, &home);
            let mut all = Vec::new();
            let mut covered = Vec::new();
            let mut failed = Vec::new();
            for (name, fetched, result) in done.results {
                // Only the fetch the slot holds is current. One whose slot
                // is gone (removed in `/provider` while its list was out)
                // or taken by a newer fetch (added again since) is
                // nobody's: its cache is not written, and it is neither
                // covered nor failed.
                match fetching.get(&name) {
                    Some((_, _, g)) if *g == fetched => {
                        fetching.remove(&name);
                    }
                    _ => continue,
                }
                if !cfg.connections.contains_key(&name) {
                    continue;
                }
                standing.remove(&name);
                match result {
                    Ok(models) => {
                        // An empty list is an answer (an account with no
                        // model that takes tools), not a failure: the last
                        // list goes with it, or the next opening shows it.
                        if models.is_empty() {
                            ryter_core::llm::model_cache::forget(&home, &name);
                        } else {
                            ryter_core::llm::model_cache::save(&home, &name, &models);
                        }
                        covered.push(name.clone());
                        all.extend(prepared(models, &name, &book));
                    }
                    Err(e) => {
                        // Keep what the picker already shows: the cached list.
                        match ryter_core::llm::model_cache::load(&home, &name) {
                            Some((models, secs)) => {
                                standing.insert(
                                    name.clone(),
                                    format!(
                                        "{name} slow · list from {} ago",
                                        ryter_core::llm::model_cache::age(secs)
                                    ),
                                );
                                covered.push(name.clone());
                                all.extend(prepared(models, &name, &book));
                            }
                            None => {
                                failed.push(name.clone());
                                standing.insert(
                                    name.clone(),
                                    format!("{name} didn't answer · try again"),
                                );
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
                // The book prices the session's turns. It was built from
                // the last list at startup, and it keeps the first rate it
                // sees for an id, so the fresh list is laid in by building
                // it again from the file just written, from the provider's
                // own rows: ingesting the picker's rows over it would have
                // kept every old rate.
                if covered.contains(&a.connection) {
                    a.book = book_for(&cfg, &home, &a.connection);
                }
            }
            let _ = ev_tx.send(AgentEvent::ModelsListed {
                models: all,
                covered,
                failed,
            });
            if let Some(note) = note_after(&standing, !fetching.is_empty()) {
                let _ = ev_tx.send(AgentEvent::ModelsNote { note });
            }
        }
    }
}

fn send_err(tx: &mpsc::Sender<AgentEvent>, message: String) {
    let _ = tx.send(AgentEvent::Error { message });
}

/// Git work with no agent to scope it: under the profile, when there is one.
fn scoped<T: Send>(
    scope: &Option<sandbox::Scope>,
    cwd: &Path,
    notes: &Path,
    run: impl FnOnce() -> ryter_core::Result<T> + Send,
) -> ryter_core::Result<T> {
    match scope {
        Some(scope) => scope.run(cwd, notes, run),
        None => run(),
    }
}

fn session_event(a: &Agent) -> AgentEvent {
    AgentEvent::Session {
        id: a.session.meta.id.to_string(),
        title: a.session.meta.title.clone(),
    }
}

fn swap_session(a: &mut Agent, s: Session) {
    a.session = s;
    a.seed_prompt_tokens();
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
    yolo: bool,
    ev_tx: mpsc::Sender<AgentEvent>,
    cancel: Arc<Cancel>,
    user_io: UserIo,
}

fn build_agent(b: BuildAgent<'_>) -> Agent {
    let provider = http_provider(&b.conn, b.key);
    let notes = b.session.notes_dir();
    // The build hat, unless the session was left in another. A session
    // left in crew mode, before it was removed, opens in build too.
    let role = b.session.meta.mode.map_or(Role::Build, Role::hat);
    let last_input_tokens = b.session.last_input_tokens();
    Agent {
        provider: Arc::new(provider),
        book: book_for(b.cfg, b.home, &b.conn_name),
        session: b.session,
        ctx: ToolContext {
            sandbox: sandbox::Scope::for_profile(b.profile, b.home),
            live: None,
            workspace: b.cwd.to_path_buf(),
            notes_dir: notes,
            role,
            always_approve: b.always_approve,
            yolo: b.yolo,
            permissions: Arc::new(b.cfg.permissions.clone()),
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
            read_only: false,
            created: Vec::new(),
            kept: Vec::new(),
            sessions: Vec::new(),
            search: ryter_core::tools::Search::from_config(&b.cfg.search),
        },
        connection: b.conn_name,
        model: b.model,
        role,
        max_turns: ryter_core::config::rounds_cap(b.cfg.limits.rounds),
        budget_usd: b.cfg.spend.session_budget_usd,
        sink: Some(b.ev_tx),
        home: b.home.to_path_buf(),
        project_root: Some(b.cwd.to_path_buf()),
        trusted: b.trusted,
        context_window: 0,
        cfg: Some(b.cfg.clone()),
        machine: ryter_core::prompt::machine_for(b.profile),
        product: None,
        audit_pending: None,
        audit_live: None,
        last_audit_verdict: None,
        turn: 0,
        last_input_tokens,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A note about a connection that didn't answer stands until that
    /// connection answers again: a batch that did not include it leaves it,
    /// and the note is cleared only when nothing stands and nothing is in
    /// flight.
    #[test]
    fn a_failure_note_outlives_another_connections_batch() {
        let mut standing = BTreeMap::new();
        standing.insert(
            "beta".to_string(),
            "beta didn't answer · try again".to_string(),
        );
        // Alpha's list came while beta's earlier failure stands.
        assert_eq!(
            note_after(&standing, false),
            Some(Some("beta didn't answer · try again".into()))
        );
        assert_eq!(
            note_after(&standing, true),
            Some(Some("beta didn't answer · try again".into()))
        );
        // Beta answered: nothing stands. Cleared only once nothing is in
        // flight; the "refreshing" note holds meanwhile.
        standing.remove("beta");
        assert_eq!(note_after(&standing, true), None);
        assert_eq!(note_after(&standing, false), Some(None));
    }
}
