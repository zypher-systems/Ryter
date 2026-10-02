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
/// the model.
fn receipt(view: &mut View, verb: &Verb, tools: u32, duration_ms: u64) -> String {
    // A turn in which the tester filed its report closes on the report:
    // `✗ 2 of 5 failed · 1:40 · $0.21`.
    let report = view.turn_report.take();
    if let (Some(headline), Verb::Done) = (report, verb) {
        let mut parts = vec![headline, crate::chat::fmt_duration(duration_ms)];
        if let Some(now) = view.spend {
            parts.push(crate::chat::turn_usd(
                now - view.turn_spend_from.unwrap_or(0.0),
            ));
        }
        return parts.join(" · ");
    }
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

/// Apply an agent event to the chat it belongs to.
///
/// What a turn says goes into the conversation that turn is part of, which
/// may not be the one on screen: the user can Tab to the main chat while a
/// test runs, or to the tester's while a build does. A hat change the
/// agent makes itself (a review, a test run) is said in the main
/// conversation, and the screen follows it.
pub fn apply(view: &mut View, ev: AgentEvent) {
    use ryter_core::Thread;
    let shown = view.shown;
    let target = match &ev {
        AgentEvent::TurnStarted { role, .. } => {
            view.turn_thread = role.thread();
            view.turn_thread
        }
        // A hat change the agent makes, and a report the tester files, are
        // said in the conversation the other hats share.
        AgentEvent::ModeChanged { .. } | AgentEvent::Tested { .. } => Thread::Main,
        _ if view.busy => view.turn_thread,
        _ => shown,
    };
    view.show(target);
    // The screen follows the agent into the tester's conversation and
    // back out of it. A change of hat within one conversation leaves the
    // screen where the user put it.
    let follow = match &ev {
        AgentEvent::ModeChanged { role } if role.thread() != view.turn_thread => {
            Some(role.thread())
        }
        _ => None,
    };
    apply_to_shown(view, ev);
    match follow {
        Some(thread) => {
            view.turn_thread = thread;
            view.show(thread);
        }
        None => view.show(shown),
    }
}

fn apply_to_shown(view: &mut View, ev: AgentEvent) {
    // The workbench shows the files as they are: read them again after
    // anything that changes them.
    if matches!(
        ev,
        AgentEvent::Reverted { .. }
            | AgentEvent::TurnFinished { .. }
            | AgentEvent::Checkpoint { .. }
            | AgentEvent::Notice { .. }
    ) {
        apply_inner(view, &ev);
        if let Some(mut w) = view.workbench.take() {
            w.reload(view);
            view.workbench = Some(w);
        }
        return;
    }
    apply_inner(view, &ev);
}

fn apply_inner(view: &mut View, ev: &AgentEvent) {
    match ev {
        AgentEvent::Token { text } => view.on_token(text),
        AgentEvent::Reasoning { text } => {
            let turn = view.turn;
            activity::push_reasoning(view, turn, text);
            if view.activity.busy() {
                view.activity.verb = Verb::Thinking;
            }
        }
        AgentEvent::ToolCall {
            id,
            name,
            args,
            role,
            summary,
        } => on_tool_call(view, id, name, args, *role, summary.as_deref()),
        AgentEvent::ToolResult {
            id,
            output,
            is_error,
            duration_ms,
            diff,
        } => {
            on_tool_result(view, id, output, *is_error, *duration_ms, diff.as_deref());
            if view.activity.busy() {
                view.activity.verb = Verb::Thinking;
                view.activity.current.clear();
            }
        }
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
            if !view.ui.classic() {
                // The ledger closes every turn with what it came to.
                let line = receipt(view, &verb, *tools, *duration_ms);
                view.push(
                    MessageKind::System {
                        level: crate::chat::SystemLevel::Receipt,
                    },
                    line,
                );
                view.tally = Default::default();
            } else if !view.tally.is_empty() {
                // What the turn did, measured, whatever the model said about it.
                let line = format!(
                    "{} · {}",
                    view.tally.line(),
                    crate::chat::fmt_duration(*duration_ms)
                );
                view.push(
                    MessageKind::System {
                        level: crate::chat::SystemLevel::Rule,
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
            total_usd,
        } => {
            if let Some(p) = &mut view.project_spend {
                p.add(*role, model, *total_usd);
            }
            on_spend(
                view,
                connection,
                *role,
                *input_tokens,
                *output_tokens,
                *cached_tokens,
                *total_usd,
            );
        }
        AgentEvent::Reviewed {
            model,
            verdict,
            tree,
            total_usd,
            ..
        } => {
            let said = match verdict {
                Some(true) => "✓ no blocking problems",
                Some(false) => "✗ blocking problems",
                None => "no verdict",
            };
            // To the tenth of a cent, as a turn's cost is: a review that
            // cost $0.003 read "$0.00".
            let cost = total_usd.map_or_else(String::new, |usd| {
                format!(" · {}", crate::chat::turn_usd(usd))
            });
            view.system(format!(
                "review · {} · {said}{cost}",
                crate::chat::short_model(model)
            ));
            view.last_review = Some((tree.clone(), model.clone(), *verdict));
        }
        AgentEvent::Tested {
            model,
            headline,
            passed,
            rows,
            file,
            first_failed,
            tree,
            total_usd,
            duration_ms,
        } => {
            let mut head = format!(
                "test · {} · {headline} · {}",
                crate::chat::short_model(model),
                crate::chat::fmt_duration(*duration_ms)
            );
            if let Some(usd) = total_usd {
                head.push_str(&format!(" · {}", crate::chat::turn_usd(*usd)));
            }
            let mut body = head;
            for row in rows {
                body.push('\n');
                body.push_str(row);
            }
            body.push_str(&format!("\nfull report  {file}"));
            view.report(body, !passed);
            // Left running, so the user can look at what the tester saw.
            if let Some(p) = &view.product {
                let at = p
                    .address
                    .as_ref()
                    .map(|a| format!(" at {a}"))
                    .unwrap_or_default();
                let how = p
                    .stop
                    .as_ref()
                    .map(|s| format!(" ({s})"))
                    .unwrap_or_default();
                view.system(format!(
                    "the project is still running{at}\n/stop stops it{how}"
                ));
            }
            view.last_test = Some((tree.clone(), model.clone(), *passed));
            view.test_runs += 1;
            view.retest = *first_failed;
            // The tester's own turn closes on what it reported.
            view.turn_report = Some(headline.clone());
        }
        AgentEvent::Session { id, title } => {
            view.session_id = id.clone();
            view.session_title = title.clone();
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
        AgentEvent::ModeChanged { role } => {
            view.mode = *role;
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
        // The commit panel shows the draft.
        AgentEvent::CommitDraft { .. } => {}
        AgentEvent::Committed { summary, error } => match (summary, error) {
            (Some(s), _) => {
                view.system(format!("committed {s}"));
                view.last_tests = None;
                view.tests_stale = false;
                view.last_review = None;
                view.last_test = None;
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
        AgentEvent::ModelsListed { models } => {
            for m in models {
                if let (Some(i), Some(o)) = (m.input_per_million, m.output_per_million) {
                    view.catalog_rates.insert(m.id.clone(), (i, o));
                }
            }
            if let Some(m) = models.iter().find(|m| m.id == view.model) {
                apply_model_catalog(view, m);
            }
            panel::on_notice(view, &Notice::Models(models.clone()));
        }
        AgentEvent::ModelsNote { note } => view.models_note = note.clone(),
        AgentEvent::McpStatus { servers } => {
            view.mcp_status = servers.iter().cloned().collect();
        }
    }
    panel::on_event(view, ev);
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
    let speaker = role.is_solo();
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
    if view.activity.busy() && role.is_solo() {
        view.activity.verb = Verb::Tool(name.to_string());
        view.activity.current = wrap::truncate(&label, 48);
        view.activity.tools += 1;
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
            let joined = if m.body.is_empty() {
                body
            } else {
                format!("{}\n{body}", m.body)
            };
            m.set_body(joined);
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
    cached_tokens: u64,
    total_usd: Option<f64>,
) {
    let role_name = crate::view::role_label(&role.to_string()).to_string();
    {
        let row = view.spend_rows_role.entry(role_name.clone()).or_default();
        row.calls += 1;
        row.input += input_tokens;
        row.output += output_tokens;
        row.cached += cached_tokens;
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
        row.calls += 1;
        row.input += input_tokens;
        row.output += output_tokens;
        row.cached += cached_tokens;
        if let Some(v) = total_usd {
            row.usd += v;
        } else {
            row.unpriced = true;
        }
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
    if role.is_solo() {
        // Tokens this turn become exact once accounting lands (`R-ACT-07`).
        view.activity.tokens = output_tokens;
        view.activity.tokens_estimated = false;
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
        let used = input_tokens.saturating_add(output_tokens);
        view.ctx_tokens = Some(used.max(view.ctx_tokens.unwrap_or(0).min(used)));
        let window = view.ctx_window_or_default();
        view.ctx_window = Some(window);
        view.ctx_pct = Some(((used.min(window) * 100) / window.max(1)) as u8);
    }
}

/// Pull the picker row for the active model into the header / info cards.
pub fn apply_model_catalog(view: &mut View, m: &ryter_core::ModelInfo) {
    if let Some(w) = m.context_length {
        view.ctx_window = Some(w);
    }
    if let (Some(i), Some(o)) = (m.input_per_million, m.output_per_million) {
        view.price_in = Some(i);
        view.price_out = Some(o);
        view.price_label = ryter_core::format_rates(Some(ryter_core::Rates::per_million(i, o)));
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
                    id: id.into(),
                    name: "record_decision".into(),
                    args: serde_json::json!({"title": "No export button in this pass"}),
                    role: Role::SoloBuild,
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
        let mut v = View::new("spacexai".into(), "grok-4.6".into(), "~/p".into());
        let _ = v.submit_user("hi".into(), "hi".into());
        v
    }

    #[test]
    fn tool_error_shows_output_not_bare_label() {
        let mut v = view();
        apply(
            &mut v,
            AgentEvent::ToolCall {
                id: "t1".into(),
                name: "bash".into(),
                args: serde_json::json!({"command": "cargo test --all"}),
                role: Role::SoloBuild,
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
        let ledger = || {
            let mut v = view();
            v.ui.layout = "ledger".into();
            v
        };
        let mut v = ledger();
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
        let mut v = ledger();
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
                connection: "spacexai".into(),
                model: "grok-4.6".into(),
                role: Role::SoloBuild,
                input_tokens: 100,
                output_tokens: 20,
                cached_tokens: 0,
                total_usd: Some(0.01),
            },
        );
        assert!(v.busy, "spend alone does not end the turn");
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
                role: Role::SoloBuild,
                input_tokens: 1,
                output_tokens: 1,
                cached_tokens: 0,
                total_usd: None,
            },
        );
        assert!(v.spend_unknown);
        assert_eq!(v.unpriced_calls, 1);
        assert_eq!(v.spend_label(), "$?.??");
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
}
