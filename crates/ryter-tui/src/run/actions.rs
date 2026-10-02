//! Perform [`Action`]s: config writes, worker requests, panel side effects.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Instant;

use ryter_core::config::{self, resolve_secret};
use ryter_core::ids::ConnectionId;
use ryter_core::sandbox::SandboxProfile;
use ryter_core::session::Session;
use ryter_core::spend::PriceBook;
use ryter_core::{Config, HookSet, InboundHost, Permission, Provider, format_usd, load_catalog};

use super::worker::Work;
use crate::action::{Action, PanelId};
use crate::activity::Verb;
use crate::chat::MessageKind;
use crate::panel::{self, Notice, PanelEnv};
use crate::theme::{ColorMode, Theme};
use crate::view::{ConnRow, View};
use ryter_core::AgentEvent;

/// Loop-owned state the actions need besides the view.
pub struct Ctx {
    /// Request channel to the agent thread.
    pub work_tx: mpsc::Sender<Work>,
    /// Out-of-band notices produced by background threads.
    pub notice_tx: mpsc::Sender<Notice>,
    /// Shared cancel flag.
    pub cancel: Arc<ryter_core::Cancel>,
    /// `~/.ryter`.
    pub home: PathBuf,
    /// Project root.
    pub workspace: PathBuf,
    /// Trusted project.
    pub trusted: bool,
    /// Landlock profile in effect.
    pub sandbox: SandboxProfile,
    /// Live config (mutated by settings panels).
    pub cfg: Config,
    /// Inbound MCP host for TCP listeners started from `/mcp`.
    pub mcp_host: Arc<dyn InboundHost>,
    /// Pending permission reply.
    pub perm_reply: Option<mpsc::Sender<Permission>>,
    /// Pending `ask_user` reply.
    pub ask_reply: Option<mpsc::Sender<String>>,
    /// The plan popout's reply channel, while one is open.
    pub plan_reply: Option<mpsc::Sender<ryter_core::user_io::PlanAnswer>>,
    /// Mouse capture currently held. Released to let the terminal select
    /// text, since capture takes click-drag away from the user.
    pub mouse_grabbed: bool,
    /// Active theme.
    pub theme: Theme,
    /// Theme to restore on `/theme` cancel.
    pub theme_before_preview: Option<(String, Theme)>,
    /// Detected color mode.
    pub color_mode: ColorMode,
    /// Last `/doctor` report for `c` export.
    pub last_doctor: Option<ryter_core::DoctorReport>,
    /// Set when the loop should clear and repaint.
    pub want_redraw: bool,
    /// Set when the loop should exit.
    pub want_quit: bool,
    /// The worker's word on a stop asked for at quit: the loop exits once
    /// it says the product was stopped, and stays if it could not be.
    pub stop_reply: Option<mpsc::Receiver<std::result::Result<String, String>>>,
    /// A `$EDITOR` request to run with the terminal released.
    pub want_edit: Option<PathBuf>,
}

impl Ctx {
    /// Panel construction context.
    pub fn env(&self) -> PanelEnv {
        PanelEnv {
            home: self.home.clone(),
            cwd: self.workspace.clone(),
            trusted: self.trusted,
            sandbox: self.sandbox,
        }
    }

    pub(super) fn send(&self, w: Work) {
        let _ = self.work_tx.send(w);
    }

    fn notice(&self, n: Notice) {
        let _ = self.notice_tx.send(n);
    }
}

/// Run one action (recursing for `Many`).
pub fn perform(view: &mut View, cx: &mut Ctx, action: Action) {
    match action {
        Action::None => {}
        Action::Many(list) => {
            for a in list {
                perform(view, cx, a);
            }
        }
        // A product Ryter started is still up: ask before leaving it.
        Action::Quit if view.product.is_some() && cx.stop_reply.is_none() => {
            let asking = view
                .panels
                .top()
                .is_some_and(|p| p.kind() == "stop-product");
            if !asking {
                view.panels.push(Box::new(crate::panel::modal::StopModal));
            }
        }
        Action::Quit => cx.want_quit = true,
        Action::QuitAnswer { stop: false } => cx.want_quit = true,
        Action::QuitAnswer { stop: true } => {
            // A turn still running holds the worker: end it first.
            if view.busy {
                cx.cancel.cancel();
            }
            let (tx, rx) = mpsc::channel();
            cx.stop_reply = Some(rx);
            view.system("stopping the project…");
            cx.send(Work::StopProduct { reply: Some(tx) });
        }
        Action::StopProduct if view.product.is_none() => {
            view.system("Ryter has not started this project: there is nothing of its to stop");
        }
        Action::StopProduct if view.busy => {
            view.warn("wait for this turn to end, or stop it with esc, then /stop");
        }
        Action::StopProduct => {
            view.system("stopping the project…");
            cx.send(Work::StopProduct { reply: None });
        }
        Action::Redraw => cx.want_redraw = true,
        // Capture gives us wheel scroll and card clicks but takes the
        // terminal's own click-drag selection away, and per-message copy is not
        // built yet — so without this there is no way to get text out of Ryter.
        Action::ToggleMouse => {
            use crossterm::ExecutableCommand;
            use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
            let mut out = std::io::stdout();
            cx.mouse_grabbed = !cx.mouse_grabbed;
            if cx.mouse_grabbed {
                let _ = out.execute(EnableMouseCapture);
                view.system("mouse grabbed — wheel scroll and card clicks active");
            } else {
                let _ = out.execute(DisableMouseCapture);
                view.system("mouse released — select and copy with the terminal; ^g to grab");
            }
            cx.want_redraw = true;
        }
        Action::Submit(text) => cx.send(Work::Turn { text, reply: None }),
        Action::Cancel => {
            if view.busy {
                cx.cancel.cancel();
                // Answer any open prompt, so nothing waits on it.
                if let Some(tx) = cx.perm_reply.take() {
                    let _ = tx.send(Permission::Deny);
                }
                cx.ask_reply = None;
                // Dropped unanswered, a plan is rejected.
                cx.plan_reply = None;
                while view.panels.has_modal() {
                    view.panels.pop();
                }
                panel::sync_composer(view);
                view.cancelling = true;
                view.activity.verb = Verb::Cancelling;
                view.system("cancelling…");
            }
        }
        Action::New => new_session(view, cx),
        // On the ledger `/changes` is the workbench.
        Action::OpenPanel(PanelId::Changes) if !view.ui.classic() => {
            perform(view, cx, Action::OpenWorkbench);
        }
        Action::OpenPanel(id) => open_panel(view, cx, id),
        Action::Resume(id) => resume(view, cx, &id),
        Action::DeleteSession(id) => delete_session(view, cx, &id),
        Action::RenameSession(title) => {
            let title = title.trim().to_string();
            view.session_title = title.clone();
            view.system(format!("renamed · {title}"));
            cx.send(Work::Rename(title));
            cx.notice(Notice::SessionsChanged);
        }
        Action::SetBudget(usd) => set_budget(view, cx, usd),
        Action::SaveBudget { usd, warn } => save_budget(view, cx, usd, warn),
        Action::SetMode(role) => set_mode(view, cx, role),
        Action::SetModelReasoning { model, level } => {
            match &level {
                Some(l) => {
                    view.model_reasoning.insert(model.clone(), l.clone());
                }
                None => {
                    view.model_reasoning.remove(&model);
                }
            }
            cx.cfg.model_reasoning = view.model_reasoning.clone();
            if let Err(e) = config::save_model_reasoning(&cx.home, &view.model_reasoning) {
                view.error(e.to_string());
            }
            cx.send(Work::SetModelReasoning(view.model_reasoning.clone()));
        }
        Action::Undo { .. } | Action::Redo { .. } if view.busy => {
            view.warn("wait for this turn to end: it may still be changing files");
        }
        Action::Undo { force } => cx.send(Work::Undo { force }),
        Action::Redo { force } => cx.send(Work::Redo { force }),
        Action::ReviewNow if view.busy => {
            view.warn("a review reads the finished work: wait for this turn to end");
        }
        Action::ReviewNow => cx.send(Work::ReviewNow),
        Action::StopReviewOffers => {
            if let Some(tx) = cx.perm_reply.take() {
                let _ = tx.send(Permission::Deny);
            }
            view.ui.offer_audit = false;
            cx.cfg.ui.offer_audit = false;
            if let Err(e) = config::save_settings(&cx.home, &cx.cfg) {
                view.error(e.to_string());
            }
            cx.send(Work::SetOfferAudit(false));
            view.system(
                "no more review offers · /audit still runs one · /settings turns them back on",
            );
        }
        Action::Revert { base, path } => cx.send(Work::Revert { base, path }),
        Action::RevertHunk { base, path, hunk } => cx.send(Work::RevertHunk { base, path, hunk }),
        Action::OpenWorkbench => {
            view.panels.clear();
            view.palette = None;
            view.workbench = Some(crate::workbench::Workbench::open(
                view,
                cx.workspace.clone(),
            ));
        }

        Action::DraftCommit(paths) => cx.send(Work::DraftCommit(paths)),
        Action::Commit { paths, message } => cx.send(Work::Commit { paths, message }),
        Action::SetReceipts(on) => {
            view.ui.receipts = on;
            cx.cfg.ui.receipts = on;
            if let Err(e) = config::save_settings(&cx.home, &cx.cfg) {
                view.error(e.to_string());
            }
        }
        Action::SetTools { always } => {
            view.perm_mode = if always { "always" } else { "ask" }.into();
            cx.send(Work::SetTools { always });
        }
        Action::SetTheme(name) => {
            if apply_theme(view, cx, &name) {
                cx.theme_before_preview = None;
                cx.cfg.ui.theme = name.clone();
                view.ui.theme = name;
                let _ = config::save_settings(&cx.home, &cx.cfg);
            }
        }
        Action::PreviewTheme(name) => {
            if cx.theme_before_preview.is_none() {
                cx.theme_before_preview = Some((view.theme_name.clone(), cx.theme));
            }
            apply_theme(view, cx, &name);
        }
        Action::RevertTheme => {
            if let Some((name, t)) = cx.theme_before_preview.take() {
                cx.theme = t;
                view.theme_name = name;
                view.theme_generation += 1;
            }
        }
        Action::Context => cx.send(Work::Context),
        Action::Compact => cx.send(Work::Compact),
        Action::UseConnection(name) => use_connection(view, cx, &name),
        Action::SetKey { name, key } => set_key(view, cx, &name, &key),
        Action::BeginSetKey(name) => {
            view.panels.clear();
            panel::sync_composer(view);
            view.composer.begin_secret(name.clone());
            view.system(format!(
                "paste the API key for {name} · Enter saves to the keyring · Esc cancels"
            ));
        }
        Action::TestConnection(name) => test_connection(view, cx, &name),
        Action::AddConnection { name, conn } => add_connection(view, cx, &name, conn),
        Action::RemoveConnection(name) => remove_connection(view, cx, &name),
        Action::SetModel(model) => set_model(view, cx, model),
        Action::SetHatModel {
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
            save_hats(view, cx);
        }
        Action::ResetHatModel(role) => {
            view.specialists.remove(&role);
            save_hats(view, cx);
        }
        Action::ListAllModels { .. } => cx.send(Work::ListAllModels),
        Action::SaveMcp => {
            persist_mcp(view, cx);
            cx.send(Work::SetMcp {
                servers: view.mcp_servers.clone(),
            });
        }
        Action::McpListenTcp => {
            persist_mcp(view, cx);
            start_mcp_tcp(view, cx);
            cx.send(Work::SetMcp {
                servers: view.mcp_servers.clone(),
            });
        }
        Action::WriteSkill { name, description } => {
            match ryter_core::write_skill(&cx.home, &name, &description) {
                Ok(path) => {
                    reload_catalog(view, cx);
                    view.system(format!(
                        "wrote {} · edit the body, then /{name}",
                        path.display()
                    ));
                }
                Err(e) => view.error(e.to_string()),
            }
        }
        Action::WriteCommand { name } => match ryter_core::write_command(&cx.home, &name, "") {
            Ok(path) => {
                reload_catalog(view, cx);
                view.system(format!(
                    "wrote {} · $ARGUMENTS is replaced on invoke",
                    path.display()
                ));
            }
            Err(e) => view.error(e.to_string()),
        },
        Action::RemoveCatalog(path) => match ryter_core::remove_catalog_entry(&cx.home, &path) {
            Ok(()) => {
                reload_catalog(view, cx);
                view.system(format!("removed {}", path.display()));
            }
            Err(e) => view.error(e.to_string()),
        },
        Action::EditCatalog(path) => cx.want_edit = Some(path),
        Action::SaveHooks => {
            match config::save_hooks(&cx.home, &view.hooks) {
                Ok(()) => view.system(format!(
                    "hooks saved · {}",
                    HookSet::from_config(&view.hooks).summary()
                )),
                Err(e) => view.error(e.to_string()),
            }
            cx.cfg.hooks = view.hooks.clone();
            cx.send(Work::SetHooks {
                hooks: view.hooks.clone(),
            });
        }
        Action::SaveSettings => save_settings(view, cx),
        Action::PermissionReply(p) => {
            if let Some(tx) = cx.perm_reply.take() {
                let _ = tx.send(p);
            }
            if view.activity.busy() {
                view.activity.verb = Verb::Thinking;
            }
        }
        Action::PlanReply(answer) => {
            if let Some(tx) = cx.plan_reply.take() {
                let _ = tx.send(answer);
            }
            if view.activity.busy() {
                view.activity.verb = Verb::Thinking;
            }
        }
        Action::AskUserReply(s) => {
            if let Some(tx) = cx.ask_reply.take() {
                let _ = tx.send(s);
            }
            if view.activity.busy() {
                view.activity.verb = Verb::Thinking;
            }
        }
        Action::TrustProject(yes) => {
            if yes {
                match config::trust(&cx.workspace) {
                    Ok(()) => {
                        cx.trusted = true;
                        reload_catalog(view, cx);
                        view.system("trusted this project's .ryter/");
                    }
                    Err(e) => view.error(e.to_string()),
                }
            } else {
                view.system("left project untrusted · project skills and config are ignored");
            }
        }
        Action::ExportSpend => export_spend(view, cx),
        Action::SaveDoctorReport => {
            let Some(report) = cx.last_doctor.clone() else {
                view.warn("run /doctor first");
                return;
            };
            let path = cx.home.join("doctor-report.txt");
            match std::fs::write(&path, report.render()) {
                Ok(()) => {
                    let shown = path.display().to_string();
                    view.last_export = Some(shown.clone());
                    cx.notice(Notice::Exported(shown));
                }
                Err(e) => view.error(format!("{}: {e}", path.display())),
            }
        }
    }
}

// -- panels ---------------------------------------------------------------------

fn open_panel(view: &mut View, cx: &mut Ctx, id: PanelId) {
    let env = cx.env();
    let then = panel::open(view, id, &env);
    panel::sync_composer(view);
    if then != Action::None {
        perform(view, cx, then);
    }
    match id {
        PanelId::Models => cx.send(Work::ListModels),
        // The logs are the truth; replace the live running copy with them.
        PanelId::Spend | PanelId::SpendDrawer => {
            load_project_spend(view, &cx.home, &cx.workspace);
        }
        PanelId::Theme => {
            cx.theme_before_preview = Some((view.theme_name.clone(), cx.theme));
        }
        PanelId::Doctor => run_doctor(cx),
        _ => {}
    }
}

fn run_doctor(cx: &mut Ctx) {
    let report = ryter_core::doctor::run(ryter_core::doctor::DoctorOpts {
        home: &cx.home,
        cwd: &cx.workspace,
        trusted: cx.trusted,
        sandbox: cx.sandbox,
    });
    let rows = report
        .checks
        .iter()
        .map(|c| {
            let status = match c.status {
                ryter_core::doctor::CheckStatus::Ok => "ok",
                ryter_core::doctor::CheckStatus::Warn => "warn",
                ryter_core::doctor::CheckStatus::Fail => "fail",
            };
            (c.name.clone(), status.to_string(), c.detail.clone())
        })
        .collect();
    cx.last_doctor = Some(report);
    cx.notice(Notice::Doctor(rows));
}

// -- sessions -------------------------------------------------------------------

fn new_session(view: &mut View, cx: &mut Ctx) {
    view.panels.clear();
    panel::sync_composer(view);
    view.reset_transcript();
    view.spend = None;
    view.spend_unknown = false;
    view.unpriced_calls = 0;
    view.spend_by_role.clear();
    view.spend_by_conn.clear();
    view.spend_rows_role.clear();
    view.spend_rows_conn.clear();
    view.ctx_tokens = Some(0);
    view.ctx_pct = Some(0);
    view.ctx_messages = None;
    view.ctx_breakdown.clear();
    view.session_title.clear();
    cx.send(Work::New);
    view.system("new session");
}

fn resume(view: &mut View, cx: &mut Ctx, id: &str) {
    if view.busy {
        view.warn("cancel the running turn first");
        return;
    }
    match Session::find(&cx.home, Some(&cx.workspace), id) {
        Ok(s) => {
            view.panels.clear();
            panel::sync_composer(view);
            fill_view_from_session(view, &s);
            cx.send(Work::Resume(view.session_id.clone()));
            view.system(format!(
                "resumed {} · {}",
                short_id(&s),
                if s.meta.title.is_empty() {
                    "untitled"
                } else {
                    s.meta.title.as_str()
                }
            ));
        }
        Err(e) => view.error(e.to_string()),
    }
}

fn delete_session(view: &mut View, cx: &mut Ctx, id: &str) {
    match Session::find(&cx.home, Some(&cx.workspace), id) {
        Ok(s) => {
            let short = short_id(&s);
            let current = s.meta.id.as_str() == view.session_id;
            match Session::remove(&s.dir) {
                Ok(()) => {
                    if current {
                        new_session(view, cx);
                    }
                    view.system(format!("deleted session {short}"));
                    cx.notice(Notice::SessionsChanged);
                }
                Err(e) => view.error(e.to_string()),
            }
        }
        Err(e) => view.error(e.to_string()),
    }
}

/// Rebuild the transcript from a saved session (`/resume`, startup).
/// The hat note Ryter puts in front of a solo message is for the model; the
/// chat shows what the user typed.
fn strip_hat_note(content: &str) -> String {
    match content.strip_prefix("[hat: ") {
        Some(rest) => match rest.split_once("]\n\n") {
            Some((_, typed)) => typed.to_string(),
            None => content.to_string(),
        },
        None => content.to_string(),
    }
}

pub fn fill_view_from_session(view: &mut View, session: &Session) {
    view.reset_transcript();
    view.session_id = session.meta.id.to_string();
    view.session_title = session.meta.title.clone();
    // A session left in crew mode, before it was removed, opens in build.
    view.mode = session
        .meta
        .mode
        .map_or(ryter_core::Role::SoloBuild, ryter_core::Role::hat);
    view.spend = session.meta.spend_usd_total;
    view.spend_unknown = session.meta.spend_unknown;
    view.connection = session.meta.connection.clone();
    view.model = session.meta.model.clone();
    // Both conversations: the one the hats share, then the tester's.
    use ryter_core::Thread;
    for thread in [Thread::Main, Thread::Test] {
        view.show(thread);
        // The tester's answers are named for the test hat's model. In the
        // shared conversation the saved messages don't say which hat's
        // model wrote each, so they carry the one every hat uses.
        let model = match thread {
            Thread::Test => view
                .specialists
                .get("test")
                .filter(|r| r.is_override())
                .and_then(|r| r.model.clone())
                .unwrap_or_else(|| view.model.clone()),
            Thread::Main => view.model.clone(),
        };
        fill_chat(view, session.messages_of(thread), &model);
        view.turn = view.turn.max(1);
        view.scroll.to_bottom();
    }
    view.show(view.mode.thread());
    view.spend_by_role.clear();
    view.spend_by_conn.clear();
    view.spend_rows_role.clear();
    view.spend_rows_conn.clear();
    view.unpriced_calls = 0;
    if let Ok(recs) = session.spend_log() {
        for r in recs {
            let role = r.role.to_string();
            for (key, map) in [
                (role.clone(), &mut view.spend_rows_role),
                (r.connection.clone(), &mut view.spend_rows_conn),
            ] {
                let row = map.entry(key).or_default();
                row.calls += 1;
                row.input += r.input_tokens;
                row.output += r.output_tokens;
                row.cached += r.cached_tokens;
                match r.total_usd {
                    Some(v) => row.usd += v,
                    None => row.unpriced = true,
                }
            }
            match r.total_usd {
                Some(v) => {
                    *view.spend_by_role.entry(role).or_insert(0.0) += v;
                    *view.spend_by_conn.entry(r.connection).or_insert(0.0) += v;
                }
                None => view.unpriced_calls += 1,
            }
        }
    }
}

/// Rebuild one conversation's chat from its saved messages, through the
/// same path a live turn takes, so a resumed session reads as it did.
fn fill_chat(view: &mut View, messages: &[ryter_core::Message], model: &str) {
    for m in messages {
        match m.role.as_str() {
            "user" if !m.content.trim().is_empty() => {
                view.turn += 1;
                view.push(MessageKind::User, strip_hat_note(&m.content));
            }
            "assistant" => {
                if !m.content.trim().is_empty() {
                    view.push(
                        MessageKind::Assistant {
                            model: model.to_string(),
                        },
                        m.content.clone(),
                    );
                }
                // Replay through the live path, so a resumed session reads
                // the same as it did.
                if let Some(calls) = &m.tool_calls {
                    for c in calls {
                        let args: serde_json::Value =
                            serde_json::from_str(&c.arguments).unwrap_or(serde_json::Value::Null);
                        crate::run::events::apply(
                            view,
                            AgentEvent::ToolCall {
                                id: c.id.clone(),
                                name: c.name.clone(),
                                summary: Some(ryter_core::tool_summary(&c.name, &args)),
                                args,
                                role: view.mode,
                            },
                        );
                    }
                }
            }
            "tool" => {
                if let Some(id) = &m.tool_call_id {
                    let c = m.content.trim_start();
                    let is_err = c.starts_with("error")
                        || c.starts_with("denied")
                        || m.content.contains("[exit ")
                        || m.content.contains("\"error\"");
                    crate::run::events::apply(
                        view,
                        AgentEvent::ToolResult {
                            id: id.clone(),
                            output: m.content.clone(),
                            is_error: is_err,
                            duration_ms: None,
                            diff: None,
                        },
                    );
                }
            }
            _ => {}
        }
    }
}

// -- connections & models -------------------------------------------------------

fn route_from_view(view: &View) -> config::LastRoute {
    config::LastRoute {
        connection: view.connection.clone(),
        model: view.model.clone(),
        context_length: view.ctx_window,
        input_per_million: view.price_in,
        output_per_million: view.price_out,
    }
}

fn apply_pricing(view: &mut View, cfg: &Config, model: &str) {
    let mut book = PriceBook::from_config(cfg);
    // The picker showed a price from the provider's list; use it when the
    // built-in book has none, rather than saying "price unknown".
    if book.rates(model).is_none() {
        if let Some(&(i, o)) = view.catalog_rates.get(model) {
            book.ingest_model_info(&[ryter_core::ModelInfo {
                id: model.to_string(),
                context_length: None,
                input_per_million: Some(i),
                output_per_million: Some(o),
                connection: None,
                created: None,
                tools: None,
            }]);
        }
    }
    view.price_label = book.format_model_rates(model);
    match book.rates(model) {
        Some(r) => {
            view.price_in = Some(r.input_per_million);
            view.price_out = Some(r.output_per_million);
        }
        None => {
            view.price_in = None;
            view.price_out = None;
        }
    }
}

fn use_connection(view: &mut View, cx: &mut Ctx, name: &str) {
    let Some(conn) = cx.cfg.connections.get(name).cloned() else {
        view.error(format!("unknown connection {name}"));
        return;
    };
    match resolve_secret(&cx.cfg, &ConnectionId::new(name)) {
        Ok(key) => {
            let model = conn
                .default_model
                .clone()
                .or_else(|| cx.cfg.orchestrator.model.clone())
                .unwrap_or_else(|| "grok-4.6".into());
            view.connection = name.to_string();
            view.model = model.clone();
            view.has_key = true;
            view.ctx_window = Some(ryter_core::window_for(&model));
            apply_pricing(view, &cx.cfg, &model);
            let _ = config::save_last_route(&cx.home, &route_from_view(view));
            view.system(format!("using {name} · {model}"));
            cx.send(Work::Reconnect {
                name: name.to_string(),
                model,
                key,
            });
            cx.send(Work::ListModels);
        }
        Err(_) => perform(view, cx, Action::BeginSetKey(name.to_string())),
    }
}

fn set_key(view: &mut View, cx: &mut Ctx, name: &str, key: &str) {
    match config::store_secret_at(&cx.home, name, key) {
        // Say where it landed: a keyring failure used to be silent, leaving the
        // user to assume the key was not on disk in plaintext.
        Ok(store) => {
            if let Some(c) = view.connections.iter_mut().find(|c| c.name == name) {
                c.has_key = true;
            }
            view.system(format!("key saved for {name} in {store}"));
            use_connection(view, cx, name);
        }
        Err(e) => view.error(e.to_string()),
    }
}

fn set_model(view: &mut View, cx: &mut Ctx, model: String) {
    match resolve_secret(&cx.cfg, &ConnectionId::new(&view.connection)) {
        Ok(key) => {
            view.model = model.clone();
            view.ctx_window = Some(ryter_core::window_for(&model));
            apply_pricing(view, &cx.cfg, &model);
            let _ = config::save_last_route(&cx.home, &route_from_view(view));
            view.system(format!("model · {model}"));
            cx.send(Work::Reconnect {
                name: view.connection.clone(),
                model,
                key,
            });
        }
        Err(_) => perform(view, cx, Action::BeginSetKey(view.connection.clone())),
    }
}

/// Switch hats. A switch while a turn runs applies to the next message.
fn set_mode(view: &mut View, cx: &mut Ctx, role: ryter_core::Role) {
    view.mode = role;
    // The tester has a conversation of its own: its hat shows that one,
    // and any other hat shows the one they share.
    view.show(role.thread());
    cx.send(Work::SetRole(role));
}

fn test_connection(view: &mut View, cx: &mut Ctx, name: &str) {
    let Some(conn) = cx.cfg.connections.get(name).cloned() else {
        view.error(format!("unknown connection {name}"));
        return;
    };
    let Ok(key) = resolve_secret(&cx.cfg, &ConnectionId::new(name)) else {
        cx.notice(Notice::ConnTest {
            name: name.to_string(),
            result: Err("no key".into()),
        });
        return;
    };
    view.conn_tests.insert(name.to_string(), "testing…".into());
    let tx = cx.notice_tx.clone();
    let name = name.to_string();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        let result = match rt {
            Ok(rt) => {
                let provider = ryter_core::http_provider(&conn, key);
                let started = Instant::now();
                rt.block_on(provider.list_models())
                    .map(|_| started.elapsed().as_millis() as u64)
                    .map_err(|e| e.to_string())
            }
            Err(e) => Err(e.to_string()),
        };
        let _ = tx.send(Notice::ConnTest { name, result });
    });
}

fn add_connection(view: &mut View, cx: &mut Ctx, name: &str, conn: ryter_core::ConnectionConfig) {
    let mut user = config::load_user_connections(&cx.home);
    user.insert(name.to_string(), conn.clone());
    if let Err(e) = config::save_user_connections(&cx.home, &user) {
        view.error(e.to_string());
        return;
    }
    cx.cfg.connections.insert(name.to_string(), conn.clone());
    view.connections.retain(|c| c.name != name);
    view.connections.push(ConnRow {
        name: name.to_string(),
        kind: conn.kind.clone(),
        model: conn.default_model.clone().unwrap_or_default(),
        has_key: config::has_secret(&cx.cfg, name),
    });
    view.connections.sort_by(|a, b| a.name.cmp(&b.name));
    view.system(format!("added connection {name}"));
    if !config::has_secret(&cx.cfg, name) {
        perform(view, cx, Action::BeginSetKey(name.to_string()));
    }
}

fn remove_connection(view: &mut View, cx: &mut Ctx, name: &str) {
    if !config::user_connection_names(&cx.home)
        .iter()
        .any(|n| n == name)
    {
        view.warn(format!(
            "{name} is defined in config.toml — remove it there"
        ));
        return;
    }
    if name == view.connection {
        view.warn("switch to another connection first");
        return;
    }
    let mut user = config::load_user_connections(&cx.home);
    user.remove(name);
    match config::save_user_connections(&cx.home, &user) {
        Ok(()) => {
            cx.cfg.connections.remove(name);
            view.connections.retain(|c| c.name != name);
            view.system(format!("removed connection {name}"));
        }
        Err(e) => view.error(e.to_string()),
    }
}

// -- hats -----------------------------------------------------------------------

/// Save each hat's own model, and tell the worker.
fn save_hats(view: &mut View, cx: &mut Ctx) {
    if let Err(e) = config::save_hats(&cx.home, &view.specialists) {
        view.error(e.to_string());
    }
    cx.cfg.specialists = view.specialists.clone();
    cx.send(Work::SetHats {
        specialists: view.specialists.clone(),
    });
}

// -- settings / theme -----------------------------------------------------------

fn save_settings(view: &mut View, cx: &mut Ctx) {
    cx.cfg.spend.session_budget_usd = view.budget_usd;
    cx.cfg.spend.warn_usd = view.warn_usd;
    cx.cfg.spend.review_usd = view.review_usd;
    cx.cfg.sandbox.profile = view.sandbox_profile.clone();
    cx.cfg.mcp.inbound = view.mcp_inbound;
    cx.cfg.features.web = view.web;
    cx.cfg.ui = view.ui.clone();
    cx.cfg.update.mode = view.update_mode;
    match config::save_settings(&cx.home, &cx.cfg) {
        Ok(()) => view.system("settings saved"),
        Err(e) => view.error(e.to_string()),
    }
    // Live `[ui]` knobs that do not need a restart.
    view.panel_visible = view.ui.panel;
    view.activity.mode = crate::activity::Mode::parse(&view.ui.reasoning);
    if view.ui.theme != view.theme_name {
        let name = view.ui.theme.clone();
        apply_theme(view, cx, &name);
    }
    let mode = ColorMode::detect(&view.ui.colors, |k| std::env::var(k).ok());
    if mode != cx.color_mode {
        cx.color_mode = mode;
        let name = view.theme_name.clone();
        apply_theme(view, cx, &name);
    }
    cx.send(Work::SetSettings {
        budget_usd: view.budget_usd,
        review_usd: view.review_usd,
        web: view.web,
        open_pages: view.ui.open_pages,
    });
    cx.send(Work::SetOfferAudit(view.ui.offer_audit));
}

fn set_budget(view: &mut View, cx: &mut Ctx, usd: f64) {
    let warn = view.warn_usd;
    save_budget(view, cx, usd, warn);
}

/// Apply spend limits to the running session and save them as the default.
fn save_budget(view: &mut View, cx: &mut Ctx, usd: f64, warn: f64) {
    view.budget_usd = usd;
    if usd > 0.0 {
        view.budget_last = usd;
    }
    view.warn_usd = warn;
    cx.cfg.spend.session_budget_usd = usd;
    cx.cfg.spend.warn_usd = warn;
    if let Err(e) = config::save_settings(&cx.home, &cx.cfg) {
        view.error(e.to_string());
    }
    cx.send(Work::SetSettings {
        budget_usd: usd,
        review_usd: view.review_usd,
        web: view.web,
        open_pages: view.ui.open_pages,
    });
    if usd > 0.0 {
        let over = view.spend.is_some_and(|s| s >= usd);
        view.system(format!(
            "budget {} · spent {}{}",
            format_usd(Some(usd)),
            format_usd(view.spend),
            if over {
                " · already reached: raise it to keep working"
            } else {
                ""
            }
        ));
    } else {
        view.system(format!(
            "budget off · spent {} · nothing stops on cost now",
            format_usd(view.spend)
        ));
    }
}

/// Load and degrade a theme; bump the render generation. Returns success.
fn apply_theme(view: &mut View, cx: &mut Ctx, name: &str) -> bool {
    match Theme::load_named(&cx.home, name) {
        Ok(t) => {
            cx.theme = t.degrade(cx.color_mode);
            view.theme_name = name.to_string();
            view.theme_generation += 1;
            true
        }
        Err(e) => {
            view.error(e);
            false
        }
    }
}

// -- mcp ------------------------------------------------------------------------

fn persist_mcp(view: &mut View, cx: &mut Ctx) {
    cx.cfg.mcp.inbound = view.mcp_inbound;
    cx.cfg.mcp.bind = view.mcp_bind.clone();
    if let Some(p) = &view.mcp_listen {
        cx.cfg.mcp.socket = Some(p.clone());
    }
    cx.cfg.mcp_servers = view.mcp_servers.clone();
    if let Err(e) = config::save_mcp(&cx.home, &cx.cfg) {
        view.error(e.to_string());
    }
    let tokens: std::collections::BTreeMap<String, String> =
        view.mcp_tokens.iter().cloned().collect();
    if let Err(e) = config::save_mcp_tokens(&cx.home, &tokens) {
        view.error(e.to_string());
    }
}

fn start_mcp_tcp(view: &mut View, cx: &mut Ctx) {
    if view.mcp_tcp_listen.is_some() {
        return;
    }
    let Some(bind) = view.mcp_bind.clone() else {
        view.warn("set a TCP bind address first");
        return;
    };
    let Ok(addr) = bind.parse::<std::net::SocketAddr>() else {
        view.error(format!("bad MCP bind {bind}"));
        return;
    };
    let tokens: Vec<String> = view.mcp_tokens.iter().map(|(_, t)| t.clone()).collect();
    if tokens.is_empty() {
        view.warn("create an inbound token first");
        return;
    }
    if let Err(e) = ryter_core::check_tcp(addr, &tokens[0], false) {
        view.error(e.to_string());
        return;
    }
    view.mcp_tcp_listen = Some(bind.clone());
    let host = cx.mcp_host.clone();
    std::thread::spawn(move || {
        let _ = ryter_core::serve_tcp(addr, host, tokens);
    });
    view.system(format!("MCP tcp listening on {bind}"));
}

// -- catalog --------------------------------------------------------------------

fn reload_catalog(view: &mut View, cx: &Ctx) {
    view.catalog = load_catalog(&cx.home, Some(&cx.workspace), cx.trusted);
}

// -- spend export ---------------------------------------------------------------

fn export_spend(view: &mut View, cx: &mut Ctx) {
    let short: String = view.session_id.chars().take(8).collect();
    let short = if short.is_empty() {
        "session".to_string()
    } else {
        short
    };
    let path = cx.home.join(format!("spend-{short}.csv"));
    match std::fs::write(&path, panel::spend::csv(view)) {
        Ok(()) => {
            let shown = path.display().to_string();
            view.last_export = Some(shown.clone());
            cx.notice(Notice::Exported(shown));
        }
        Err(e) => view.error(format!("{}: {e}", path.display())),
    }
}

fn short_id(s: &Session) -> String {
    s.meta.id.as_str().chars().take(8).collect()
}

/// `~`-relative display path for the header.
/// Read what the project has cost, and note its root when that isn't `cwd`:
/// a repository around the folder counts the other folders in it too.
pub fn load_project_spend(view: &mut View, home: &Path, cwd: &Path) {
    let Ok(p) = ryter_core::project::project_spend(home, cwd) else {
        return;
    };
    let here = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    view.project_root = (p.root != here).then(|| display_home_path(&p.root));
    view.project_spend = Some(p);
}

pub fn display_home_path(cwd: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        if let Ok(rel) = cwd.strip_prefix(&home) {
            if rel.as_os_str().is_empty() {
                return "~".into();
            }
            return format!("~/{}", rel.display());
        }
    }
    cwd.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The model picker shows a provider's price; the sidebar must use it
    /// when the built-in book doesn't know the model.
    #[test]
    fn the_sidebar_uses_the_catalog_price() {
        let mut v = View::new(
            "openrouter".into(),
            "vendor/new-model".into(),
            "/tmp".into(),
        );
        let cfg = Config::default();
        apply_pricing(&mut v, &cfg, "vendor/new-model");
        assert_eq!(v.price_in, None, "unknown to the book and no catalog yet");
        v.catalog_rates
            .insert("vendor/new-model".into(), (0.15, 0.6));
        apply_pricing(&mut v, &cfg, "vendor/new-model");
        assert_eq!(v.price_in, Some(0.15));
        assert_eq!(v.price_out, Some(0.6));
        assert!(!v.price_label.contains('?'), "{}", v.price_label);
    }
}
