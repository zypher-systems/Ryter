//! `AgentEvent` → `View` (§13.4). Terminal-free so it can be unit tested.

use ryter_core::{AgentEvent, Role};

use crate::activity::{self, Verb};
use crate::chat::{MessageKind, SystemLevel, ToolStatus, humanize, wrap};
use crate::panel::{self, Notice};
use crate::view::View;

/// Cap on tool error text shown in the chat (`R-EVT-02`).
const TOOL_ERROR_CHARS: usize = 600;

/// Apply one event to the view and notify open panels.
/// A turn's closing line on the ledger: `✓ 4 tools · 1 file (1 changed, +9
/// −1) · 1 command (1 ok) · 12s · $0.004`. Measured by Ryter, not reported by
/// the model. A folded turn shows it without the clock and the cost, which
/// `chat::layout::brief` finds by their shape: a new part shaped like either
/// (digits and `s` or `:`, or a leading `$`) would be folded away with them.
fn receipt(view: &mut View, verb: &Verb, tools: u32, duration_ms: u64) -> String {
    let mut parts = vec![match verb {
        Verb::Stopped => "⊘ stopped".to_string(),
        Verb::Failed => "✕ failed".to_string(),
        _ => "✓".to_string(),
    }];
    let mark = parts.remove(0);
    // A turn that stopped or failed before any tool didn't answer anything.
    parts.push(match (tools, verb) {
        (0, Verb::Stopped | Verb::Failed) => mark,
        (0, _) => format!("{mark} answered"),
        (n, _) => format!("{mark} {n} tool{}", if n == 1 { "" } else { "s" }),
    });
    if !view.tally.is_empty() {
        parts.push(view.tally.line());
    }
    parts.push(crate::chat::fmt_duration(duration_ms));
    if let Some(now) = view.spend {
        let cost = now - view.turn_spend_from.unwrap_or(0.0);
        parts.push(crate::chat::turn_usd(cost));
    }
    parts.join(" · ")
}

/// Apply an agent event to the chat.
pub fn apply(view: &mut View, ev: AgentEvent) {
    view.rack.apply(&ev);
    // The agent saying which hat it put on at the user's Tab. Once it has
    // caught up with every Tab, its word stands: a Tab pressed while it was
    // changing hats itself would otherwise leave the screen naming a hat
    // the agent is not in.
    if let AgentEvent::HatSet { role } = &ev {
        view.agent_hat = *role;
        view.hats_pending = view.hats_pending.saturating_sub(1);
        if view.hats_pending == 0 && view.mode != *role {
            view.set_mode(*role);
        }
        return;
    }
    match &ev {
        AgentEvent::TurnStarted { role, .. } => {
            view.turn_open = true;
            view.agent_hat = *role;
        }
        AgentEvent::ModeChanged { role } => view.agent_hat = *role,
        _ => {}
    }
    let closes = matches!(ev, AgentEvent::TurnFinished { .. });
    apply_to_shown(view, ev);
    if closes {
        view.turn_open = false;
        view.starting_product = None;
    }
}

fn apply_to_shown(view: &mut View, ev: AgentEvent) {
    // The instruments show the files as they are: read them again after
    // anything that changes them.
    if matches!(
        ev,
        AgentEvent::Reverted { .. }
            | AgentEvent::TurnFinished { .. }
            | AgentEvent::Checkpoint { .. }
            | AgentEvent::Notice { .. }
    ) {
        apply_inner(view, &ev);
        // So do the instruments, of what is uncommitted.
        view.refresh_uncommitted();
        return;
    }
    apply_inner(view, &ev);
}

fn apply_inner(view: &mut View, ev: &AgentEvent) {
    // The guard card's `audit.md` row reads `writing…` through an audit turn.
    match ev {
        AgentEvent::TurnStarted { role, .. } => {
            view.audit_writing = *role == ryter_core::Role::Audit;
        }
        AgentEvent::TurnFinished { .. } | AgentEvent::Audited { .. } => {
            view.audit_writing = false;
        }
        _ => {}
    }
    match ev {
        AgentEvent::Token { text } => view.on_token(text),
        AgentEvent::Reasoning { text } => {
            let turn = view.turn;
            // Thought is output too: the pulse counts it.
            let tokens = (text.len() as u64).div_ceil(4);
            view.pulse.push(view.now_ms, tokens);
            activity::push_reasoning(view, turn, text);
            if view.activity.busy() {
                view.activity.verb = Verb::Thinking;
                view.activity.tokens += tokens;
                let now = view.now_ms;
                view.activity.note_delta(now);
                view.activity.note_text(text);
            }
        }
        AgentEvent::ToolCall {
            id,
            name,
            args,
            role,
            summary,
            ..
        } => on_tool_call(view, id, name, args, *role, summary.as_deref()),
        AgentEvent::ToolResult {
            id,
            output,
            is_error,
            duration_ms,
            diff,
        } => {
            on_tool_result(view, id, output, *is_error, *duration_ms, diff.as_deref());
            view.activity.tool_in_flight = None;
            if view.activity.busy() {
                view.activity.verb = Verb::Thinking;
                view.activity.current.clear();
                let now = view.now_ms;
                view.activity.note_delta(now);
            }
        }
        AgentEvent::ToolLive { id, tail } => on_tool_live(view, id, tail),
        AgentEvent::TurnStarted { .. } => {
            view.tally = Default::default();
            view.lookups = None;
            view.turn_spend_from = view.spend;
            // `R-EVT-03`: authoritative busy signal. `submit_user` already
            // started the strip for keyboard turns; MCP-driven turns land here.
            view.busy = true;
            view.cancelling = false;
            if !view.activity.busy() {
                let turn = view.turn;
                let now = view.now_ms;
                view.activity.start(turn, now);
            }
        }
        AgentEvent::TurnFinished {
            tools, duration_ms, ..
        } => {
            // `Cancelled` and `Error` arrive first and end the strip; keep
            // what they said rather than calling the turn done.
            let verb = match view.activity.verb {
                _ if view.cancelling => Verb::Stopped,
                Verb::Stopped | Verb::Failed if !view.activity.busy() => view.activity.verb.clone(),
                _ => Verb::Done,
            };
            view.busy = false;
            view.cancelling = false;
            view.activity
                .finish(verb.clone(), Some(*tools), Some(*duration_ms));
            // Every turn closes with what it came to.
            {
                let line = receipt(view, &verb, *tools, *duration_ms);
                view.push(
                    MessageKind::System {
                        level: crate::chat::SystemLevel::Receipt,
                    },
                    line,
                );
                view.tally = Default::default();
            }
        }
        AgentEvent::Spend {
            connection,
            model,
            role,
            input_tokens,
            output_tokens,
            cached_tokens,
            cache_write_tokens,
            total_usd,
            incomplete,
        } => {
            if let Some(p) = &mut view.project_spend {
                p.add(*role, model, *total_usd);
                if *incomplete && total_usd.is_some() {
                    p.unpriced_calls += 1;
                }
            }
            on_spend(
                view,
                connection,
                *role,
                *input_tokens,
                *output_tokens,
                (*cached_tokens, *cache_write_tokens),
                *total_usd,
                *incomplete,
            );
        }
        // `/audit` adds this after the turn's `Audited`, with the tree the
        // audit covered: the commit receipt's mark. The words were said by
        // `Audited`.
        AgentEvent::Reviewed {
            model,
            verdict,
            tree,
            ..
        } => {
            view.last_review = Some((tree.clone(), model.clone(), *verdict));
        }
        AgentEvent::Audited {
            model,
            verdict,
            headline,
            restored,
            filed,
            total_usd,
            ..
        } => {
            // To the tenth of a cent, as a turn's cost is.
            let cost = total_usd.map_or_else(String::new, |usd| {
                format!(" · {}", crate::chat::turn_usd(usd))
            });
            let tree = if restored.is_empty() {
                String::new()
            } else {
                format!(
                    " · restored {} file{}",
                    restored.len(),
                    if restored.len() == 1 { "" } else { "s" }
                )
            };
            view.system(format!(
                "audit · {} · {headline}{cost}{tree}",
                crate::chat::short_model(model)
            ));
            if view.last_review.is_none() {
                view.last_review = Some((None, model.clone(), *verdict));
            }
            view.refresh_uncommitted();
            if *filed {
                let opened = view.now_ms;
                if let Some(p) = crate::panel::audit::AuditModal::from_event(ev, opened) {
                    view.panels.push(Box::new(p));
                }
            }
        }
        AgentEvent::Session { id, title } => {
            view.session_id = id.clone();
            view.session_title = title.clone();
            // The session just left is on disk now, or the one resumed
            // is open: the welcome's count follows.
            view.refresh_sessions_count();
        }
        AgentEvent::Notice { message } => view.system(message.clone()),
        AgentEvent::Product {
            running,
            at,
            address,
            stop,
        } => {
            let was = view.product.take();
            if !*running {
                return;
            }
            let now = crate::view::ProductUp {
                at: at.clone(),
                address: address.clone(),
                stop: stop.clone(),
            };
            // Said once, when it comes up (or is found up at startup).
            if was.as_ref() != Some(&now) {
                let time = at.rsplit(' ').next().unwrap_or(at);
                let at_address = address
                    .as_ref()
                    .map(|a| format!(" at {a}"))
                    .unwrap_or_default();
                let how = stop.as_ref().map(|s| format!(" ({s})")).unwrap_or_default();
                view.system(format!(
                    "the project is running{at_address}, started {time} · /stop stops it{how}"
                ));
            }
            view.product = Some(now);
        }
        // Taken in `apply`, before anything is routed.
        AgentEvent::HatSet { .. } => {}
        AgentEvent::ModeChanged { role } => {
            view.set_mode(*role);
            // A hat on a model of its own says which: the next message
            // goes to it.
            let own = if view.hat_model() == view.model {
                String::new()
            } else {
                format!(" ({})", crate::chat::short_model(view.hat_model()))
            };
            view.system(format!(
                "switched to the {role} hat{own} · Tab to change it again"
            ));
        }
        AgentEvent::Checkpoint { sha } => view.last_checkpoint = sha.clone(),
        // The rack counts it; the plan's own notice says it in the chat.
        AgentEvent::Planned { .. } => {}
        // The commit panel shows the draft.
        AgentEvent::CommitDraft { .. } => {}
        AgentEvent::Committed { summary, error } => match (summary, error) {
            (Some(s), _) => {
                view.system(format!("committed {s}"));
                view.refresh_uncommitted();
                view.last_tests = None;
                view.tests_stale = false;
                view.last_review = None;
                view.panels
                    .stack
                    .retain(|p| !matches!(p.kind(), "commit" | "changes"));
            }
            (None, Some(e)) => view.error(format!("commit failed: {e}")),
            (None, None) => {}
        },
        AgentEvent::Reverted { path, error } => match error {
            None => view.system(format!("put back {path} · /undo brings it back")),
            Some(e) => view.error(format!("couldn't put back {path}: {e}")),
        },
        AgentEvent::Error { message } => {
            let was_busy = view.busy || view.activity.busy();
            view.busy = false;
            view.cancelling = false;
            view.error(message.clone());
            if was_busy {
                view.activity.finish(Verb::Failed, None, None);
            }
        }
        AgentEvent::Cancelled => {
            view.busy = false;
            view.cancelling = false;
            view.activity.finish(Verb::Stopped, None, None);
            view.system("cancelled");
        }
        AgentEvent::Context {
            tokens,
            window,
            pct,
            messages,
            breakdown,
            ..
        } => {
            view.ctx_pct = Some(*pct);
            view.ctx_tokens = Some(*tokens);
            view.ctx_window = Some(*window);
            view.ctx_messages = Some(*messages);
            view.ctx_breakdown = breakdown.clone();
        }
        AgentEvent::Compacted {
            before,
            after,
            window,
        } => {
            let pct = if *window == 0 {
                0
            } else {
                (after.saturating_mul(100) / window).min(100) as u8
            };
            view.ctx_pct = Some(pct);
            view.ctx_tokens = Some(*after);
            view.ctx_window = Some(*window);
            view.push(
                MessageKind::System {
                    level: SystemLevel::Rule,
                },
                format!(
                    "transcript compacted · {} → {} tokens",
                    humanize(*before),
                    humanize(*after)
                ),
            );
        }
        AgentEvent::ModelsListed {
            models,
            covered,
            failed,
        } => {
            // The list may carry every keyed connection's rows, and one id
            // can be on two of them at two prices and two windows. Only the
            // session's own connection's row is the session's. Names are
            // compared as the worker knows them: a session saved under a
            // connection's old name runs, and is listed, under the new.
            let canon = |c: &str| ryter_core::canonical_connection(c).to_string();
            let on = |m: &ryter_core::ModelInfo| {
                canon(m.connection.as_deref().unwrap_or(&view.connection))
            };
            let mine = canon(&view.connection);
            let covers = |c: &str| covered.iter().any(|x| canon(x) == c);
            // A covering list is the whole of what that connection lists:
            // its earlier rows go, so a model it no longer carries (its
            // account's filter dropped it, or the list came back empty)
            // has no stale price to fall back on.
            view.catalog_rates.retain(|(c, _), _| !covers(c));
            view.catalog_windows.retain(|(c, _), _| !covers(c));
            for m in models {
                if let (Some(i), Some(o)) = (m.input_per_million, m.output_per_million) {
                    view.catalog_rates.insert((on(m), m.id.clone()), (i, o));
                }
                if let Some(w) = m.context_length {
                    view.catalog_windows.insert((on(m), m.id.clone()), w);
                }
            }
            let own = models.iter().find(|m| m.id == view.model && on(m) == mine);
            if let Some(w) = own.and_then(|m| m.context_length) {
                view.ctx_window = Some(w);
            }
            // The header's price is the book's, which charges the turns.
            // The row's own price is one input to that book (a shipped
            // cache tier can win over it), so the event loop sets the
            // header from the book built as the worker builds it, and
            // saves the route with it: a model the list no longer carries
            // is priced as the book prices it, which may be not at all.
            if own.is_some() || covers(&mine) {
                view.reprice = true;
            }
            panel::on_notice(
                view,
                &Notice::Models {
                    models: models.clone(),
                    covered: covered.clone(),
                    failed: failed.clone(),
                },
            );
        }
        AgentEvent::ModelsNote { note } => view.models_note = note.clone(),
        AgentEvent::McpStatus { servers } => {
            view.mcp_status = servers.iter().cloned().collect();
        }
        // The card was the screen's part; the event is for the log.
        AgentEvent::Asked { .. } => {}
    }
    panel::on_event(view, ev);
}

/// Newest lines from a running command: under the tool row, and the quiet
/// clock resets so a long `cargo test` is not "waiting for the model".
fn on_tool_live(view: &mut View, id: &str, tail: &[String]) {
    let body = tail.join("\n");
    if let Some(m) = view.messages.iter_mut().rev().find(|m| {
        m.meta.tool_id.as_deref() == Some(id)
            && matches!(
                m.kind,
                MessageKind::Tool {
                    status: ToolStatus::Running,
                    ..
                }
            )
    }) {
        m.set_body(body);
    }
    if view.activity.busy() {
        let now = view.now_ms;
        view.activity.note_delta(now);
        if let Some(line) = tail.last().filter(|l| !l.is_empty()) {
            view.activity.note_text(line);
        }
    }
}

fn on_tool_call(
    view: &mut View,
    id: &str,
    name: &str,
    args: &serde_json::Value,
    role: Role,
    summary: Option<&str>,
) {
    use crate::chat::toolview;
    let label = summary
        .map(str::to_string)
        .unwrap_or_else(|| guess_summary(name, args));
    let target = toolview::target(name, args);
    view.tool_calls
        .insert(id.to_string(), (name.to_string(), target.clone()));
    if name == "run_project" && args.get("action").and_then(|a| a.as_str()) == Some("start") {
        view.starting_product = Some(id.to_string());
    }
    let speaker = role.is_hat();
    // Reads and searches fold into one row while they keep coming; the
    // chat is for the work.
    if speaker && toolview::is_lookup(name) {
        let last_id = view.messages.last().map(|m| m.id);
        match &mut view.lookups {
            Some((mid, l)) if Some(*mid) == last_id => {
                l.add(name, &target);
                let label = l.label();
                let mid = *mid;
                if let Some(m) = view.messages.iter_mut().rev().find(|m| m.id == mid) {
                    m.meta.label = Some(label);
                    if let MessageKind::Tool { name, .. } = &mut m.kind {
                        *name = "look".into();
                    }
                    m.touch();
                }
            }
            _ => {
                let mut l = toolview::Lookups::default();
                l.add(name, &target);
                let label = l.label();
                let m = view.push(
                    MessageKind::Tool {
                        name: toolview::verb(name).to_string(),
                        status: ToolStatus::Ok,
                    },
                    String::new(),
                );
                m.meta.label = Some(label);
                let mid = m.id;
                view.lookups = Some((mid, l));
            }
        }
    } else {
        let shown = if name == "propose_run" {
            "how this project runs".to_string()
        } else if target.is_empty() {
            label.clone()
        } else {
            target.clone()
        };
        let m = view.push(
            MessageKind::Tool {
                // The project's own commands read as what they do:
                // `start  docker compose up -d --wait`.
                name: match (name, args.get("action").and_then(|a| a.as_str())) {
                    ("run_project", Some(action)) => action.to_string(),
                    _ => toolview::verb(name).to_string(),
                },
                status: ToolStatus::Running,
            },
            toolview::edit_preview(name, args),
        );
        m.meta.tool_id = Some(id.to_string());
        if !shown.is_empty() {
            m.meta.label = Some(if speaker {
                shown
            } else {
                format!("{role} · {shown}")
            });
        }
        view.lookups = None;
    }
    if view.activity.busy() && role.is_hat() {
        // `running cargo test`, `running write app.py`: the tool row's
        // own words, so the status row and the sidebar name the step.
        let step = match (name, target.trim()) {
            ("bash", cmd) if !cmd.is_empty() => cmd.to_string(),
            (_, "") => toolview::verb(name).to_string(),
            (_, t) => format!("{} {t}", toolview::verb(name)),
        };
        let step = wrap::truncate(&step, 40);
        view.activity.verb = Verb::Tool(step.clone());
        view.activity.tool_in_flight = Some(step);
        view.activity.note_tool(&label);
        view.activity.tools += 1;
        let now = view.now_ms;
        view.activity.note_delta(now);
    }
}

/// A tool finished: its row says what came of it (`new · 48 lines`,
/// `✓ 12 passed`, `✗ exit 1` and the last lines of output), and the turn's
/// tally counts it. A failed lookup, folded away, still shows its error.
fn on_tool_result(
    view: &mut View,
    id: &str,
    output: &str,
    is_error: bool,
    duration_ms: Option<u64>,
    diff: Option<&ryter_core::diff::FileDiff>,
) {
    use crate::chat::toolview;
    let (tool, target) = view.tool_calls.remove(id).unwrap_or_default();
    if view.starting_product.as_deref() == Some(id) {
        view.starting_product = None;
    }
    if toolview::is_lookup(&tool) {
        if is_error {
            let body = wrap::truncate(output.trim(), TOOL_ERROR_CHARS);
            view.error(if body.is_empty() {
                "tool error".into()
            } else {
                body
            });
        }
        return;
    }
    view.finish_tool(id, is_error, duration_ms);
    // A decision that was recorded is said once, by the line Ryter adds
    // ("decision recorded: …"). Only one that wasn't keeps its step, with
    // the reason.
    if tool == "record_decision" && !is_error {
        view.messages
            .retain(|m| m.meta.tool_id.as_deref() != Some(id));
        return;
    }
    let (detail, body) = toolview::result(&tool, output, is_error);
    if let Some(m) = view
        .messages
        .iter_mut()
        .rev()
        .find(|m| m.meta.tool_id.as_deref() == Some(id))
    {
        m.meta.detail = Some(detail);
        // The measured diff replaces the preview drawn from the model's
        // arguments: same lines, now with the file's numbers and context.
        if let Some(d) = diff.filter(|d| !is_error && !d.is_empty()) {
            m.meta.detail = Some(if d.created {
                format!("new · {} lines", d.added)
            } else {
                format!("+{} −{}", d.added, d.removed)
            });
            m.meta.diff = Some(Box::new(d.clone()));
            m.set_body(String::new());
        } else if !body.is_empty() {
            // Live bash lines were only for the screen while it ran; the
            // finished output replaces them. An edit keeps its preview.
            if tool == "bash" || tool == "run_project" {
                m.set_body(body);
            } else {
                let joined = if m.body.is_empty() {
                    body
                } else {
                    format!("{}\n{body}", m.body)
                };
                m.set_body(joined);
            }
        } else if tool == "bash" || tool == "run_project" {
            // A quiet success: drop the live tail so the row is the header.
            m.set_body(String::new());
        } else {
            m.touch();
        }
    }
    match diff.filter(|_| !is_error) {
        Some(d) => view.tally.add_diff(&target, d),
        None => view.tally.add(&tool, &target, output, is_error),
    }
    // For a commit receipt: the latest test result, and whether the model
    // edited files after it.
    match tool.as_str() {
        "bash" => {
            if let Some(s) = toolview::test_summary(output) {
                let mark = if is_error { '✗' } else { '✓' };
                view.last_tests = Some(format!("{mark} {s}"));
                view.tests_stale = false;
            }
        }
        "write" | "search_replace" if !is_error && view.last_tests.is_some() => {
            view.tests_stale = true;
        }
        _ => {}
    }
}

/// `R-ACT-05`: the most identifying argument when the core sent no summary.
fn guess_summary(name: &str, args: &serde_json::Value) -> String {
    let pick = |keys: &[&str]| -> Option<String> {
        keys.iter()
            .find_map(|k| args.get(*k).and_then(|v| v.as_str()).map(str::to_string))
    };
    match name {
        "bash" | "shell" | "run" => pick(&["command", "cmd"])
            .map(|c| c.lines().next().unwrap_or("").chars().take(40).collect())
            .unwrap_or_default(),
        "web_search" => pick(&["query", "q"]).unwrap_or_default(),
        _ => pick(&["path", "file", "file_path", "url", "pattern", "name"]).unwrap_or_default(),
    }
}

#[allow(clippy::too_many_arguments)]
fn on_spend(
    view: &mut View,
    connection: &str,
    role: Role,
    input_tokens: u64,
    output_tokens: u64,
    (cached_tokens, cache_write_tokens): (u64, u64),
    total_usd: Option<f64>,
    incomplete: bool,
) {
    let role_name = crate::view::role_label(&role.to_string()).to_string();
    {
        let row = view.spend_rows_role.entry(role_name.clone()).or_default();
        row.unpriced |= incomplete;
        row.calls += 1;
        row.input += input_tokens;
        row.output += output_tokens;
        row.cached += cached_tokens;
        row.written += cache_write_tokens;
        if let Some(v) = total_usd {
            row.usd += v;
        } else {
            row.unpriced = true;
        }
    }
    {
        let row = view
            .spend_rows_conn
            .entry(connection.to_string())
            .or_default();
        row.unpriced |= incomplete;
        row.calls += 1;
        row.input += input_tokens;
        row.output += output_tokens;
        row.cached += cached_tokens;
        row.written += cache_write_tokens;
        if let Some(v) = total_usd {
            row.usd += v;
        } else {
            row.unpriced = true;
        }
    }
    if incomplete {
        view.spend_unknown = true;
        if total_usd.is_some() {
            view.unpriced_calls += 1;
        }
        view.activity.cost_unknown = true;
    }
    match total_usd {
        Some(v) => {
            view.spend = Some(view.spend.unwrap_or(0.0) + v);
            *view.spend_by_role.entry(role_name).or_insert(0.0) += v;
            *view
                .spend_by_conn
                .entry(connection.to_string())
                .or_insert(0.0) += v;
        }
        None => {
            view.spend_unknown = true;
            view.unpriced_calls += 1;
        }
    }
    if role.is_hat() {
        // Tokens this turn become exact once accounting lands (`R-ACT-07`).
        view.activity.tokens = output_tokens;
        view.activity.tokens_estimated = incomplete;
        match total_usd {
            Some(v) => view.activity.cost = Some(view.activity.cost.unwrap_or(0.0) + v),
            None => view.activity.cost_unknown = true,
        }
        // Attach the cost to the assistant message this call produced.
        if let Some(m) = view
            .messages
            .iter_mut()
            .rev()
            .find(|m| matches!(m.kind, MessageKind::Assistant { .. }))
        {
            if let Some(v) = total_usd {
                m.meta.cost = Some(m.meta.cost.unwrap_or(0.0) + v);
                m.touch();
            }
        }
        // Context fill is the prompt the provider counted, not input+output
        // and not the bytes/4 estimate.
        let used = input_tokens;
        view.ctx_tokens = Some(used);
        let window = view.ctx_window_or_default();
        view.ctx_window = Some(window);
        view.ctx_pct = Some(((used.min(window) * 100) / window.max(1)) as u8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A decision that was recorded is one line in the chat: Ryter's
    /// "decision recorded". One that wasn't keeps its step and its reason.
    #[test]
    fn a_recorded_decision_is_said_once() {
        let call = |v: &mut View, id: &str| {
            apply(
                v,
                AgentEvent::ToolCall {
                    turn: 0,
                    at: 0,
                    id: id.into(),
                    name: "record_decision".into(),
                    args: serde_json::json!({"title": "No export button in this pass"}),
                    role: Role::Build,
                    summary: None,
                },
            );
        };
        let result = |v: &mut View, id: &str, output: &str, is_error: bool| {
            apply(
                v,
                AgentEvent::ToolResult {
                    id: id.into(),
                    output: output.into(),
                    is_error,
                    duration_ms: Some(2),
                    diff: None,
                },
            );
        };
        let lines = |v: &View| -> Vec<String> {
            v.messages
                .iter()
                .filter(|m| !matches!(m.kind, MessageKind::User))
                .map(|m| {
                    format!("{} {}", m.meta.label.clone().unwrap_or_default(), m.body)
                        .trim()
                        .to_string()
                })
                .collect()
        };
        let mut v = view();
        call(&mut v, "d1");
        apply(
            &mut v,
            AgentEvent::Notice {
                message: "decision recorded: No export button in this pass".into(),
            },
        );
        result(&mut v, "d1", "Recorded in `.ryter/decisions.md`", false);
        assert_eq!(
            lines(&v),
            ["decision recorded: No export button in this pass"]
        );
        // Refused: the step stays, named in plain words, with why.
        let mut v = view();
        call(&mut v, "d2");
        result(
            &mut v,
            "d2",
            "no plan has been approved in this session",
            true,
        );
        let tool = v
            .messages
            .iter()
            .find(|m| matches!(m.kind, MessageKind::Tool { .. }))
            .expect("the step");
        assert!(
            matches!(&tool.kind, MessageKind::Tool { name, status: ToolStatus::Error } if name == "decide"),
            "{:?}",
            tool.kind
        );
        assert_eq!(
            tool.meta.label.as_deref(),
            Some("No export button in this pass")
        );
    }

    fn view() -> View {
        let mut v = View::new("spacexsi".into(), "grok-4.6".into(), "~/p".into());
        let _ = v.submit_user("hi".into(), "hi".into());
        v
    }

    #[test]
    fn tool_error_shows_output_not_bare_label() {
        let mut v = view();
        apply(
            &mut v,
            AgentEvent::ToolCall {
                turn: 0,
                at: 0,
                id: "t1".into(),
                name: "bash".into(),
                args: serde_json::json!({"command": "cargo test --all"}),
                role: Role::Build,
                summary: None,
            },
        );
        assert_eq!(v.activity.current, "cargo test --all");
        apply(
            &mut v,
            AgentEvent::ToolResult {
                id: "t1".into(),
                output: "error[E0308]: mismatched types".into(),
                is_error: true,
                duration_ms: Some(1200),
                diff: None,
            },
        );
        let tool = v
            .messages
            .iter()
            .find(|m| matches!(m.kind, MessageKind::Tool { .. }))
            .unwrap();
        assert!(matches!(
            tool.kind,
            MessageKind::Tool {
                status: ToolStatus::Error,
                ..
            }
        ));
        assert_eq!(tool.meta.duration_ms, Some(1200));
        assert!(v.messages.last().unwrap().body.contains("E0308"));
    }

    /// A running bash shows its newest lines under the tool row, and those
    /// lines are replaced by the finished output (not kept beside it).
    #[test]
    fn a_running_bash_shows_its_newest_lines() {
        let mut v = view();
        apply(
            &mut v,
            AgentEvent::TurnStarted {
                turn: 1,
                role: Role::Build,
                at: 0,
            },
        );
        apply(
            &mut v,
            AgentEvent::ToolCall {
                turn: 1,
                at: 0,
                id: "b1".into(),
                name: "bash".into(),
                args: serde_json::json!({"command": "cargo test"}),
                role: Role::Build,
                summary: Some("cargo test".into()),
            },
        );
        v.now_ms = 1_000;
        apply(
            &mut v,
            AgentEvent::ToolLive {
                id: "b1".into(),
                tail: vec![
                    "test a ... ok".into(),
                    "test b ... ok".into(),
                    "test c ... ok".into(),
                ],
            },
        );
        let tool = v
            .messages
            .iter()
            .find(|m| m.meta.tool_id.as_deref() == Some("b1"))
            .expect("running bash row");
        assert!(
            matches!(
                &tool.kind,
                MessageKind::Tool {
                    status: ToolStatus::Running,
                    ..
                }
            ),
            "{:?}",
            tool.kind
        );
        assert_eq!(tool.body, "test a ... ok\ntest b ... ok\ntest c ... ok");
        assert_eq!(v.activity.last_delta_ms, Some(1_000));
        // Still running: quiet clock must not flip to waiting for the model.
        assert!(
            matches!(v.activity.verb, Verb::Tool(_)),
            "{:?}",
            v.activity.verb
        );
        apply(
            &mut v,
            AgentEvent::ToolLive {
                id: "b1".into(),
                tail: vec!["test c ... ok".into(), "test d ... ok".into()],
            },
        );
        let tool = v
            .messages
            .iter()
            .find(|m| m.meta.tool_id.as_deref() == Some("b1"))
            .unwrap();
        assert_eq!(tool.body, "test c ... ok\ntest d ... ok");
        apply(
            &mut v,
            AgentEvent::ToolResult {
                id: "b1".into(),
                output: "test result: ok. 4 passed\n".into(),
                is_error: false,
                duration_ms: Some(4_000),
                diff: None,
            },
        );
        let tool = v
            .messages
            .iter()
            .find(|m| m.meta.tool_id.as_deref() == Some("b1"))
            .unwrap();
        assert!(
            !tool.body.contains("test a ... ok"),
            "live tail must not stay: {}",
            tool.body
        );
        assert!(
            tool.body.is_empty(),
            "live tail cleared for a test summary: {}",
            tool.body
        );
        assert!(
            tool.meta
                .detail
                .as_deref()
                .is_some_and(|d| d.starts_with('✓') && d.contains("4 passed")),
            "finished detail: {:?}",
            tool.meta.detail
        );
    }

    #[test]
    fn a_cancelled_turn_ends_stopped_not_done() {
        let mut v = view();
        apply(&mut v, AgentEvent::Cancelled);
        apply(
            &mut v,
            AgentEvent::TurnFinished {
                turn: 1,
                tools: 1,
                duration_ms: 8000,
            },
        );
        assert_eq!(v.activity.verb, Verb::Stopped);
        assert!(!v.busy);
    }

    /// A turn that failed before any tool closes as failed: the agent
    /// reports the error first. It used to read "✓ answered", then
    /// "✕ failed answered".
    #[test]
    fn a_failed_turn_closes_as_failed() {
        let receipt = |v: &View| {
            v.messages
                .iter()
                .rev()
                .find(|m| {
                    matches!(
                        m.kind,
                        MessageKind::System {
                            level: crate::chat::SystemLevel::Receipt
                        }
                    )
                })
                .map(|m| m.body.clone())
                .unwrap_or_default()
        };
        let fresh = || view();
        let mut v = fresh();
        apply(
            &mut v,
            AgentEvent::Error {
                message: "The crew can't work in ~/workspace".into(),
            },
        );
        let done = AgentEvent::TurnFinished {
            turn: 1,
            tools: 0,
            duration_ms: 10,
        };
        apply(&mut v, done.clone());
        assert!(receipt(&v).starts_with("✕ failed · "), "{}", receipt(&v));
        let mut v = fresh();
        apply(&mut v, done);
        assert!(receipt(&v).starts_with("✓ answered · "), "{}", receipt(&v));
    }

    #[test]
    fn turn_lifecycle_drives_busy_and_summary() {
        let mut v = view();
        assert!(v.busy);
        apply(&mut v, AgentEvent::Reasoning { text: "hmm".into() });
        assert_eq!(v.activity.verb, Verb::Thinking);
        apply(&mut v, AgentEvent::Token { text: "ok".into() });
        apply(
            &mut v,
            AgentEvent::Spend {
                connection: "spacexsi".into(),
                model: "grok-4.6".into(),
                role: Role::Build,
                input_tokens: 100,
                output_tokens: 20,
                cached_tokens: 0,
                cache_write_tokens: 0,
                total_usd: Some(0.01),
                incomplete: false,
            },
        );
        assert!(v.busy, "spend alone does not end the turn");
        assert_eq!(v.ctx_tokens, Some(100), "gauge uses provider input_tokens");
        assert_ne!(v.ctx_tokens, Some(120), "not input+output");
        apply(
            &mut v,
            AgentEvent::TurnFinished {
                turn: 1,
                tools: 3,
                duration_ms: 41_000,
            },
        );
        assert!(!v.busy);
        assert_eq!(v.activity.verb, Verb::Done);
        assert_eq!(v.activity.tools, 3);
        assert_eq!(v.spend, Some(0.01));
        assert_eq!(v.spend_rows_role["build"].calls, 1);
    }

    #[test]
    fn unknown_price_never_prints_zero() {
        let mut v = view();
        apply(
            &mut v,
            AgentEvent::Spend {
                connection: "x".into(),
                model: "m".into(),
                role: Role::Build,
                input_tokens: 1,
                output_tokens: 1,
                cached_tokens: 0,
                cache_write_tokens: 0,
                total_usd: None,
                incomplete: false,
            },
        );
        assert!(v.spend_unknown);
        assert_eq!(v.unpriced_calls, 1);
        assert_eq!(v.spend_label(), "$?.??");
    }

    #[test]
    fn interrupted_accounting_keeps_known_cost_and_marks_it_incomplete() {
        let mut v = view();
        apply(
            &mut v,
            AgentEvent::Spend {
                connection: "fixture".into(),
                model: "fixture".into(),
                role: Role::Build,
                input_tokens: 1_000,
                output_tokens: 0,
                cached_tokens: 0,
                cache_write_tokens: 0,
                total_usd: Some(0.002),
                incomplete: true,
            },
        );
        assert_eq!(v.spend, Some(0.002));
        assert!(v.spend_unknown && v.activity.cost_unknown && v.activity.tokens_estimated);
        assert!(v.spend_rows_role["build"].unpriced);
        assert!(v.spend_rows_conn["fixture"].unpriced);
        assert_eq!(v.unpriced_calls, 1);
    }

    #[test]
    fn compacted_leaves_a_rule_marker() {
        let mut v = view();
        apply(
            &mut v,
            AgentEvent::Compacted {
                before: 180_000,
                after: 42_000,
                window: 256_000,
            },
        );
        let last = v.messages.last().unwrap();
        assert!(matches!(
            last.kind,
            MessageKind::System {
                level: SystemLevel::Rule
            }
        ));
        assert_eq!(last.body, "transcript compacted · 180k → 42k tokens");
    }

    /// One model id on two connections, at two prices and two windows: the
    /// session's connection's row is the one the header and the price take,
    /// whichever comes first in the list, and each row's rate is kept under
    /// its own connection.
    #[test]
    fn the_sessions_row_is_its_own_connections() {
        let row = |conn: &str, ctx: u64, i: f64, o: f64| ryter_core::ModelInfo {
            id: "shared-model".into(),
            context_length: Some(ctx),
            input_per_million: Some(i),
            output_per_million: Some(o),
            cached_per_million: None,
            cache_write_per_million: None,
            connection: Some(conn.into()),
            created: None,
            tools: None,
        };
        for other_first in [true, false] {
            let mut v = View::new("openrouter".into(), "shared-model".into(), "~/p".into());
            let mut models = vec![
                row("anthropic", 100_000, 1.0, 2.0),
                row("openrouter", 200_000, 3.0, 4.0),
            ];
            if !other_first {
                models.reverse();
            }
            apply(
                &mut v,
                AgentEvent::ModelsListed {
                    models,
                    covered: vec![],
                    failed: vec![],
                },
            );
            assert_eq!(v.ctx_window, Some(200_000), "other first: {other_first}");
            assert!(v.reprice, "the header is priced from the book");
            assert_eq!(
                v.catalog_rates
                    .get(&("anthropic".to_string(), "shared-model".to_string())),
                Some(&(1.0, 2.0))
            );
            assert_eq!(
                v.catalog_rates
                    .get(&("openrouter".to_string(), "shared-model".to_string())),
                Some(&(3.0, 4.0))
            );
        }
    }

    /// The session's connection answers `/models` without the session's
    /// model: the turns are priced from that list, so the catalog price the
    /// header showed goes, and the header is to be set from the book again.
    /// Another connection's list leaves it.
    #[test]
    fn a_list_without_the_sessions_model_drops_its_catalog_price() {
        let row = |id: &str, conn: &str| ryter_core::ModelInfo {
            id: id.into(),
            context_length: Some(128_000),
            input_per_million: Some(1.0),
            output_per_million: Some(2.0),
            cached_per_million: None,
            cache_write_per_million: None,
            connection: Some(conn.into()),
            created: None,
            tools: None,
        };
        let listed = |v: &mut View, models: Vec<ryter_core::ModelInfo>, covered: &[&str]| {
            apply(
                v,
                AgentEvent::ModelsListed {
                    models,
                    covered: covered.iter().map(|c| c.to_string()).collect(),
                    failed: vec![],
                },
            );
        };
        let key = ("openrouter".to_string(), "vendor/gone".to_string());
        let mut v = View::new("openrouter".into(), "vendor/gone".into(), "~/p".into());
        listed(
            &mut v,
            vec![row("vendor/gone", "openrouter")],
            &["openrouter"],
        );
        assert_eq!(v.catalog_rates.get(&key), Some(&(1.0, 2.0)));
        assert_eq!(v.catalog_windows.get(&key), Some(&128_000));
        assert!(v.reprice, "the session's own row: priced from the book");
        v.reprice = false;
        // Another connection's list is not this one's business.
        listed(&mut v, vec![row("other", "anthropic")], &["anthropic"]);
        assert_eq!(v.catalog_rates.get(&key), Some(&(1.0, 2.0)));
        assert!(!v.reprice);
        // The session's connection's list, without the model.
        listed(
            &mut v,
            vec![row("vendor/other", "openrouter")],
            &["openrouter"],
        );
        assert_eq!(v.catalog_rates.get(&key), None);
        assert_eq!(v.catalog_windows.get(&key), None);
        assert!(v.reprice);
        // An empty answer from it, the same.
        v.reprice = false;
        listed(
            &mut v,
            vec![row("vendor/gone", "openrouter")],
            &["openrouter"],
        );
        assert_eq!(v.catalog_rates.get(&key), Some(&(1.0, 2.0)));
        v.reprice = false;
        listed(&mut v, vec![], &["openrouter"]);
        assert_eq!(v.catalog_rates.get(&key), None);
        assert!(v.reprice);
    }

    /// A model's window is its list row's; an id the lists gave no length
    /// for gets Ryter's own guess, 200k for one it does not know by name.
    #[test]
    fn the_window_of_a_model_is_its_rows() {
        let mut v = View::new("openrouter".into(), "x-ai/grok-4.7".into(), "~/p".into());
        assert_eq!(v.window_of("openrouter", "x-ai/grok-4.7"), 200_000);
        assert_eq!(v.window_of("spacexsi", "grok-4.7"), 500_000);
        apply(
            &mut v,
            AgentEvent::ModelsListed {
                models: vec![ryter_core::ModelInfo {
                    id: "x-ai/grok-4.7".into(),
                    context_length: Some(131_072),
                    input_per_million: None,
                    output_per_million: None,
                    cached_per_million: None,
                    cache_write_per_million: None,
                    connection: Some("openrouter".into()),
                    created: None,
                    tools: None,
                }],
                covered: vec![],
                failed: vec![],
            },
        );
        assert_eq!(v.window_of("openrouter", "x-ai/grok-4.7"), 131_072);
        assert_eq!(v.window_of("spacexsi", "grok-4.7"), 500_000);
    }

    /// A session saved under a connection's old name (`spacexai`) is
    /// listed, tagged and covered under the new (`spacexsi`): the row is
    /// still the session's own, and its window and price reach the view.
    #[test]
    fn a_row_under_the_connections_new_name_is_the_sessions() {
        let mut v = View::new("spacexai".into(), "grok-4.6".into(), "~/p".into());
        apply(
            &mut v,
            AgentEvent::ModelsListed {
                models: vec![ryter_core::ModelInfo {
                    id: "grok-4.6".into(),
                    context_length: Some(512_000),
                    input_per_million: Some(2.0),
                    output_per_million: Some(6.0),
                    cached_per_million: None,
                    cache_write_per_million: None,
                    connection: Some("spacexsi".into()),
                    created: None,
                    tools: None,
                }],
                covered: vec!["spacexsi".into()],
                failed: vec![],
            },
        );
        assert_eq!(v.ctx_window, Some(512_000));
        assert!(v.reprice);
        assert_eq!(
            v.catalog_rates
                .get(&("spacexsi".to_string(), "grok-4.6".to_string())),
            Some(&(2.0, 6.0))
        );
    }
}
