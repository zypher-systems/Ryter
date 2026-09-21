//! Live terminal loop: startup wiring, 30 fps coalesced redraw (`R-PERF-06`),
//! input routing, and the agent worker thread.

mod actions;
pub(crate) mod events;
pub(crate) mod keys;
mod worker;

use std::io::{self, Write, stdout};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossterm::ExecutableCommand;
use crossterm::cursor::{Hide, SetCursorStyle, Show};
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyEventKind, KeyboardEnhancementFlags, MouseButton, MouseEvent, MouseEventKind,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ryter_core::config::{self, resolve_secret};
use ryter_core::ids::ConnectionId;
use ryter_core::sandbox::{self, SandboxProfile};
use ryter_core::session::Session;
use ryter_core::spend::PriceBook;
use ryter_core::{
    AgentEvent, Cancel, HookSet, InboundHost, Phase, StatusSnapshot, UserIo, UserRequest,
    load_catalog,
};

use crate::action::Action;
use crate::activity::{Mode as ActivityMode, Verb};
use crate::chat::parse_tz_offset;
use crate::draw::{Hit, draw};
use crate::panel::modal::{AskModal, PermissionModal, TrustModal};
use crate::panel::{self, Notice};
use crate::theme::{ColorMode, Theme};
use crate::view::{ConnRow, View, resolve_username};
use actions::Ctx;
use worker::{Work, WorkerInit};

pub use actions::display_home_path;

/// Redraw budget: 30 fps.
const FRAME: Duration = Duration::from_millis(33);

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

struct TuiAttach {
    work: mpsc::Sender<Work>,
    cancel: Arc<Cancel>,
    status: Arc<Mutex<StatusSnapshot>>,
    spend: Arc<Mutex<String>>,
}

impl InboundHost for TuiAttach {
    fn prompt(&self, text: &str) -> ryter_core::Result<String> {
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

    fn cancel(&self) {
        self.cancel.cancel();
    }
}

fn io_err(e: impl std::fmt::Display) -> ryter_core::Error {
    ryter_core::Error::Io(e.to_string())
}

/// Run the fullscreen TUI. Restores the terminal on exit.
pub fn run(opts: TuiOpts) -> ryter_core::Result<()> {
    let cwd = std::env::current_dir().map_err(io_err)?;
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
    let phase = actions::parse_phase(opts.phase.as_deref())?.unwrap_or(Phase::Build);
    let profile: SandboxProfile = if let Some(s) = opts.sandbox.as_deref() {
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

    // -- view -----------------------------------------------------------------
    let mut view = View::new(
        phase,
        conn_name.clone(),
        model.clone(),
        display_home_path(&cwd),
    );
    populate_view(
        &mut view,
        &cfg,
        &home,
        &cwd,
        trusted,
        &session,
        key.is_some(),
        &opts,
    );
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
    if resumed {
        actions::fill_view_from_session(&mut view, &session);
    }
    for w in cfg.warnings.clone() {
        view.warn(w);
    }
    if key.is_none() {
        view.warn(format!(
            "no API key for {conn_name} — /provider set-key, or export XAI_API_KEY / OPENROUTER_API_KEY"
        ));
    }
    if cwd.join(".ryter").is_dir() && !trusted {
        view.panels.push(Box::new(TrustModal::default()));
    }

    // -- theme ----------------------------------------------------------------
    let color_mode = ColorMode::detect(&cfg.ui.colors, |k| std::env::var(k).ok());
    let theme = match Theme::load_named(&home, &cfg.ui.theme) {
        Ok(t) => {
            view.theme_name = cfg.ui.theme.clone();
            t.degrade(color_mode)
        }
        Err(e) => {
            view.warn(format!("{e} — using dark"));
            Theme::truecolor_dark().degrade(color_mode)
        }
    };

    // -- channels & worker ------------------------------------------------------
    let (user_io, prompt_rx) = UserIo::pair();
    let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>();
    let (work_tx, work_rx) = mpsc::channel::<Work>();
    let (notice_tx, notice_rx) = mpsc::channel::<Notice>();
    let cancel = Cancel::new();
    let live_status = Arc::new(Mutex::new(StatusSnapshot {
        model: model.clone(),
        connection: conn_name.clone(),
        session: session.meta.id.to_string(),
        last_error: String::new(),
    }));
    let live_spend = Arc::new(Mutex::new(String::new()));

    let init = WorkerInit {
        cfg: cfg.clone(),
        conn,
        key,
        session,
        cwd: cwd.clone(),
        home: home.clone(),
        trusted,
        conn_name: conn_name.clone(),
        model: model.clone(),
        always_approve: opts.always_approve,
        profile,
        work_rx,
        ev_tx,
        cancel: cancel.clone(),
        live_status: live_status.clone(),
        live_spend: live_spend.clone(),
        user_io,
    };
    let _ = work_tx.send(Work::ListModels);
    std::thread::spawn(move || worker::run(init));

    let attach_host: Arc<dyn InboundHost> = Arc::new(TuiAttach {
        work: work_tx.clone(),
        cancel: cancel.clone(),
        status: live_status,
        spend: live_spend,
    });
    let sock_path = start_inbound(&mut view, &cfg, &home, attach_host.clone());

    // -- terminal -------------------------------------------------------------
    let mouse = cfg.ui.mouse;
    let mut cx = Ctx {
        work_tx: work_tx.clone(),
        notice_tx,
        cancel,
        home: home.clone(),
        workspace: cwd,
        trusted,
        sandbox: profile,
        cfg,
        mcp_host: attach_host,
        perm_reply: None,
        ask_reply: None,
        mouse_grabbed: mouse,
        theme,
        theme_before_preview: None,
        color_mode,
        last_doctor: None,
        want_redraw: false,
        want_quit: false,
        want_edit: None,
    };
    enter_terminal(mouse)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout())).map_err(io_err)?;
    let result = loop_ui(
        &mut terminal,
        &mut view,
        &mut cx,
        &ev_rx,
        &notice_rx,
        &prompt_rx,
        mouse,
    );
    // The user may have released it mid-session (`Ctrl+G`).
    leave_terminal(cx.mouse_grabbed);
    if let Some(p) = sock_path {
        let _ = std::fs::remove_file(p);
    }
    let _ = config::save_last_route(
        &home,
        &config::LastRoute {
            connection: view.connection.clone(),
            model: view.model.clone(),
            context_length: view.ctx_window,
            input_per_million: view.price_in,
            output_per_million: view.price_out,
        },
    );
    let _ = work_tx.send(Work::Shutdown);
    result
}

#[allow(clippy::too_many_arguments)]
fn populate_view(
    view: &mut View,
    cfg: &ryter_core::Config,
    home: &Path,
    cwd: &Path,
    trusted: bool,
    session: &Session,
    has_key: bool,
    opts: &TuiOpts,
) {
    view.ui = cfg.ui.clone();
    view.panel_visible = cfg.ui.panel;
    view.activity = crate::activity::Activity::new(ActivityMode::parse(&cfg.ui.reasoning));
    view.username = resolve_username(
        &cfg.ui.username,
        ryter_core::git::git(cwd, &["config", "user.name"]).ok(),
        std::env::var("USER").ok(),
    );
    view.tz_offset = local_tz_offset();
    view.git_branch = ryter_core::git::branch(cwd).ok();
    view.perm_mode = if opts.always_approve {
        "always".into()
    } else {
        "ask".into()
    };
    view.catalog = load_catalog(home, Some(cwd), trusted);
    view.hooks = cfg.hooks.clone();
    view.theme_names = Theme::list(home);
    view.has_key = has_key;
    view.auditor_on = cfg.auditor.enabled;
    view.specialists = cfg.specialists.clone();
    view.budget_usd = cfg.spend.session_budget_usd;
    if view.budget_usd > 0.0 {
        view.budget_last = view.budget_usd;
    }
    view.task_budget_usd = cfg.spend.task_budget_usd;
    view.warn_usd = cfg.spend.warn_usd;
    view.max_crew = cfg.subagents.max;
    view.sandbox_profile = cfg.sandbox.profile.clone();
    view.web = cfg.features.web;
    view.mcp_inbound = cfg.mcp.inbound;
    view.mcp_bind = cfg.mcp.bind.clone();
    view.mcp_servers = cfg.mcp_servers.clone();
    view.mcp_tokens = config::list_mcp_tokens(home).into_iter().collect();
    view.session_id = session.meta.id.to_string();
    view.session_title = session.meta.title.clone();
    view.ctx_tokens = Some(0);
    view.ctx_pct = Some(0);
    view.ctx_window = Some(ryter_core::window_for(&view.model));
    view.price_label = PriceBook::from_config(cfg).format_model_rates(&view.model);
    if let Some(r) = PriceBook::from_config(cfg).rates(&view.model) {
        view.price_in = Some(r.input_per_million);
        view.price_out = Some(r.output_per_million);
    }
    view.connections = cfg
        .connections
        .iter()
        .map(|(n, c)| ConnRow {
            name: n.clone(),
            kind: c.kind.clone(),
            model: c.default_model.clone().unwrap_or_default(),
            has_key: config::has_secret(cfg, n),
        })
        .collect();
    let _ = HookSet::from_config(&cfg.hooks);
}

/// Local UTC offset via `date +%z` (no chrono dependency); UTC on failure.
fn local_tz_offset() -> i32 {
    std::process::Command::new("date")
        .arg("+%z")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| parse_tz_offset(&s))
        .unwrap_or(0)
}

/// Start inbound MCP listeners (unix socket, optional TCP) if enabled.
fn start_inbound(
    view: &mut View,
    cfg: &ryter_core::Config,
    home: &Path,
    host: Arc<dyn InboundHost>,
) -> Option<PathBuf> {
    if !cfg.mcp.inbound {
        return None;
    }
    let mut sock_path = None;
    #[cfg(unix)]
    {
        let path = cfg
            .mcp
            .socket
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| ryter_core::default_socket_path(home));
        match ryter_core::bind_unix(&path) {
            Ok(listener) => {
                view.mcp_listen = Some(path.display().to_string());
                sock_path = Some(path);
                let host = host.clone();
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
            Err(e) => view.warn(format!("inbound MCP socket: {e}")),
        }
    }
    #[cfg(not(unix))]
    let _ = home;
    if let Some(bind) = cfg.mcp.bind.as_deref() {
        if let Ok(addr) = bind.parse::<std::net::SocketAddr>() {
            let tokens: Vec<String> = view.mcp_tokens.iter().map(|(_, t)| t.clone()).collect();
            if !tokens.is_empty() && ryter_core::check_tcp(addr, &tokens[0], false).is_ok() {
                view.mcp_tcp_listen = Some(bind.to_string());
                std::thread::spawn(move || {
                    let _ = ryter_core::serve_tcp(addr, host, tokens);
                });
            }
        }
    }
    sock_path
}

fn enter_terminal(mouse: bool) -> ryter_core::Result<()> {
    enable_raw_mode().map_err(io_err)?;
    let mut out = stdout();
    out.execute(EnterAlternateScreen)
        .and_then(|s| s.execute(Hide))
        .and_then(|s| s.execute(SetCursorStyle::SteadyBlock))
        .and_then(|s| s.execute(EnableBracketedPaste))
        .map_err(io_err)?;
    if mouse {
        let _ = out.execute(EnableMouseCapture);
    }
    // Best effort: lets Shift+Enter / Alt+Enter reach us on kitty-protocol terminals.
    let _ = out.execute(PushKeyboardEnhancementFlags(
        KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES,
    ));
    Ok(())
}

fn leave_terminal(mouse: bool) {
    let mut out = stdout();
    let _ = out.execute(PopKeyboardEnhancementFlags);
    if mouse {
        let _ = out.execute(DisableMouseCapture);
    }
    let _ = out.execute(DisableBracketedPaste);
    let _ = out.execute(SetCursorStyle::DefaultUserShape);
    let _ = out.execute(Show);
    let _ = disable_raw_mode();
    let _ = out.execute(LeaveAlternateScreen);
    let _ = out.flush();
}

fn loop_ui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    view: &mut View,
    cx: &mut Ctx,
    ev_rx: &mpsc::Receiver<AgentEvent>,
    notice_rx: &mpsc::Receiver<Notice>,
    prompt_rx: &mpsc::Receiver<UserRequest>,
    mouse: bool,
) -> ryter_core::Result<()> {
    let epoch = Instant::now();
    let mut last_draw = epoch.checked_sub(FRAME).unwrap_or(epoch);
    let mut hit = Hit::default();
    let mut dirty = true;
    loop {
        // Drain everything that arrived since the last frame (`R-EVT-05`).
        while let Ok(ev) = ev_rx.try_recv() {
            events::apply(view, ev);
            dirty = true;
        }
        while let Ok(n) = notice_rx.try_recv() {
            panel::on_notice(view, &n);
            dirty = true;
        }
        if drain_user_prompts(view, cx, prompt_rx) {
            dirty = true;
        }
        if !view.busy {
            if let Some(q) = view.queued_prompt.take() {
                let a = view.submit_user(q.clone(), q);
                actions::perform(view, cx, a);
                dirty = true;
            }
        }
        if cx.want_quit {
            return Ok(());
        }
        if let Some(path) = cx.want_edit.take() {
            edit_with_editor(terminal, view, mouse, &path);
            dirty = true;
        }
        if cx.want_redraw {
            cx.want_redraw = false;
            terminal.clear().map_err(io_err)?;
            dirty = true;
        }

        let now = Instant::now();
        let animating = view.busy || view.quit_armed_until.is_some();
        let since = now.duration_since(last_draw);
        // Draw when something changed or a spinner is running, but never more
        // than once per FRAME; idle screens still repaint every 500 ms for the clock.
        let due = if animating {
            since >= FRAME
        } else {
            dirty || since >= Duration::from_millis(500)
        };
        if due {
            view.tick(now.duration_since(epoch).as_millis() as u64);
            let theme = cx.theme;
            let mut painted = Hit::default();
            terminal
                .draw(|f| painted = draw(f, view, theme))
                .map_err(io_err)?;
            hit = painted;
            last_draw = Instant::now();
            dirty = false;
        }

        // Sleep until the next frame is due or input arrives.
        let until_frame = FRAME.saturating_sub(Instant::now().duration_since(last_draw));
        let wait = if animating || dirty {
            until_frame.max(Duration::from_millis(1))
        } else {
            Duration::from_millis(250)
        };
        if !event::poll(wait).map_err(io_err)? {
            continue;
        }
        // Coalesce all pending terminal events before the next draw.
        loop {
            let ev = event::read().map_err(io_err)?;
            dirty = true;
            let action = match ev {
                Event::Key(key)
                    if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat =>
                {
                    keys::handle(view, key)
                }
                Event::Key(_) => Action::None,
                Event::Paste(text) => {
                    on_paste(view, &text);
                    Action::None
                }
                Event::Mouse(m) => on_mouse(view, m, &hit),
                Event::Resize(_, _) => Action::None,
                Event::FocusGained | Event::FocusLost => Action::None,
            };
            actions::perform(view, cx, action);
            if cx.want_quit {
                return Ok(());
            }
            if !event::poll(Duration::ZERO).map_err(io_err)? {
                break;
            }
        }
    }
}

/// Bracketed paste → composer (`R-COMP-10`). Panels that own the composer get it too.
fn on_paste(view: &mut View, text: &str) {
    if view.panels.has_modal() && view.panels.wants_input(view).is_none() {
        return;
    }
    view.composer.paste(text);
    crate::palette::refresh(view);
}

/// Wheel scrolls the chat; clicks open info cards or toggle the activity strip
/// (`R-SCROLL-11`). Nothing else is captured.
fn on_mouse(view: &mut View, m: MouseEvent, hit: &Hit) -> Action {
    let inside = |r: Rect| -> bool {
        m.column >= r.x && m.column < r.x + r.width && m.row >= r.y && m.row < r.y + r.height
    };
    // Wheel anywhere over the chat column (pane or its gutter) scrolls the chat;
    // over the composer it is ignored so a wheel flick never moves the transcript
    // out from under the cursor while typing.
    let over_chat = inside(hit.chat)
        || (m.row >= hit.chat.y
            && m.row < hit.chat.y + hit.chat.height
            && m.column < hit.chat.x + hit.chat.width + 1);
    let over_composer = inside(hit.composer);
    match m.kind {
        MouseEventKind::ScrollUp => {
            if view.panels.is_empty() && over_chat && !over_composer {
                view.scroll.wheel(true, view.busy);
            }
            Action::None
        }
        MouseEventKind::ScrollDown => {
            if view.panels.is_empty() && over_chat && !over_composer {
                view.scroll.wheel(false, view.busy);
            }
            Action::None
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if !view.panels.is_empty() {
                return Action::None;
            }
            if inside(hit.activity) && hit.activity.height > 0 {
                view.activity.toggle();
                return Action::None;
            }
            for (card, rect) in &hit.cards {
                if inside(*rect) {
                    if let Some(id) = card.opens() {
                        return Action::OpenPanel(id);
                    }
                }
            }
            Action::None
        }
        _ => Action::None,
    }
}

/// Surface one pending permission / question as a modal (`R-POP-75..`).
fn drain_user_prompts(
    view: &mut View,
    cx: &mut Ctx,
    prompt_rx: &mpsc::Receiver<UserRequest>,
) -> bool {
    if view.panels.has_modal() {
        return false;
    }
    let Ok(req) = prompt_rx.try_recv() else {
        return false;
    };
    match req {
        UserRequest::Permission {
            tool,
            summary,
            reply,
        } => {
            cx.perm_reply = Some(reply);
            view.panels
                .push(Box::new(PermissionModal::new(tool, summary)));
        }
        UserRequest::Question {
            question,
            options,
            reply,
        } => {
            cx.ask_reply = Some(reply);
            view.panels.push(Box::new(AskModal::new(question, options)));
        }
    }
    panel::sync_composer(view);
    if view.activity.busy() {
        view.activity.verb = Verb::Waiting;
    }
    true
}

/// Release the terminal, run `$EDITOR <path>`, and take it back.
fn edit_with_editor(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    view: &mut View,
    mouse: bool,
    path: &Path,
) {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".into());
    leave_terminal(mouse);
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("ryter")
        .arg(path)
        .status();
    let _ = enter_terminal(mouse);
    let _ = terminal.clear();
    match status {
        Ok(s) if s.success() => view.system(format!("edited {}", path.display())),
        Ok(s) => view.warn(format!("{editor} exited with {s}")),
        Err(e) => view.error(format!("{editor}: {e}")),
    }
}
