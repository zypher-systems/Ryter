//! `AgentEvent` → `View` (§13.4). Terminal-free so it can be unit tested.

use ryter_core::{AgentEvent, Role};

use crate::activity::{self, Verb};
use crate::chat::{MessageKind, SystemLevel, ToolStatus, humanize, wrap};
use crate::panel::{self, Notice};
use crate::view::{CrewRow, TodoRow, View};

/// Cap on tool error text shown in the chat (`R-EVT-02`).
const TOOL_ERROR_CHARS: usize = 600;

/// Apply one event to the view and notify open panels.
pub fn apply(view: &mut View, ev: AgentEvent) {
    match &ev {
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
        } => {
            on_tool_result(view, id, output, *is_error, *duration_ms);
            if view.activity.busy() {
                view.activity.verb = Verb::Thinking;
                view.activity.current.clear();
            }
        }
        AgentEvent::TurnStarted { .. } => {
            view.tally = Default::default();
            view.lookups = None;
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
            view.activity.finish(verb, Some(*tools), Some(*duration_ms));
            // What the turn did, measured, whatever the model said about it.
            if !view.tally.is_empty() {
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
            ..
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
        AgentEvent::PhaseChanged { phase } => {
            view.phase = *phase;
        }
        AgentEvent::SubagentStarted {
            id,
            role,
            description,
        } => {
            view.crew.push(CrewRow {
                id: id.to_string(),
                role: role.to_string(),
                label: description.clone(),
                spend: None,
                status: "running".into(),
                started_ms: view.now_ms,
            });
        }
        // Progress goes on the specialist's crew row, not into the chat.
        AgentEvent::SubagentActivity { id, text, .. } => {
            if let Some(row) = view.crew.iter_mut().find(|c| c.id == id.as_str()) {
                row.status = wrap::truncate(text, 48);
            }
        }
        AgentEvent::SubagentFinished {
            id,
            role,
            summary,
            body,
        } => {
            let label = view
                .crew
                .iter()
                .find(|c| c.id == id.as_str())
                .map(|c| c.label.clone())
                .unwrap_or_else(|| role.to_string());
            if !body.trim().is_empty() {
                let model = specialist_model(view, role.as_str());
                let m = view.push(
                    MessageKind::Specialist {
                        role: role.as_str().to_string(),
                        model,
                    },
                    body.clone(),
                );
                m.meta.label = Some(label.clone());
            }
            if *role == Role::Builder {
                view.push(MessageKind::Merge, format!("{label} · {summary}"));
                if view.activity.busy() {
                    view.activity.verb = Verb::Merging;
                }
            }
            view.crew.retain(|c| c.id != id.as_str());
        }
        AgentEvent::Session { id, phase, title } => {
            view.session_id = id.clone();
            view.phase = *phase;
            view.session_title = title.clone();
        }
        AgentEvent::Notice { message } => view.system(message.clone()),
        AgentEvent::ModeChanged { role } => {
            view.mode = *role;
            view.system(format!(
                "switched to the {role} hat · Tab to change it again"
            ));
        }
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
            view.crew.clear();
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
        AgentEvent::McpStatus { servers } => {
            view.mcp_status = servers.iter().cloned().collect();
        }
    }
    panel::on_event(view, &ev);
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
    let speaker = role == Role::Orchestrator || role.is_solo();
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
        let shown = if target.is_empty() {
            label.clone()
        } else {
            target.clone()
        };
        let m = view.push(
            MessageKind::Tool {
                name: toolview::verb(name).to_string(),
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
    if name == "todo_write" {
        view.todos = parse_todos(args);
    }
    if view.activity.busy() && (role == Role::Orchestrator || role.is_solo()) {
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
    let (detail, body) = toolview::result(&tool, output, is_error);
    if let Some(m) = view
        .messages
        .iter_mut()
        .rev()
        .find(|m| m.meta.tool_id.as_deref() == Some(id))
    {
        m.meta.detail = Some(detail);
        if !body.is_empty() {
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
    view.tally.add(&tool, &target, output, is_error);
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
    // The user knows this role as the lead; logs keep `orchestrator`.
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
    if role == Role::Orchestrator || role.is_solo() {
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
    } else if let Some(c) = view
        .crew
        .iter_mut()
        .find(|c| c.role == role.to_string() && c.status == "running")
    {
        if let Some(v) = total_usd {
            c.spend = Some(c.spend.unwrap_or(0.0) + v);
        }
    }
}

fn specialist_model(view: &View, role: &str) -> String {
    view.specialists
        .get(role)
        .and_then(|rm| rm.model.clone())
        .unwrap_or_else(|| view.model.clone())
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

/// `todo_write` arguments → info-panel rows.
pub fn parse_todos(args: &serde_json::Value) -> Vec<TodoRow> {
    let Some(items) = args.get("items").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .map(|item| {
            if let Some(s) = item.as_str() {
                TodoRow {
                    title: s.to_string(),
                    status: "pending".into(),
                }
            } else {
                TodoRow {
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

#[cfg(test)]
mod tests {
    use super::*;
    use ryter_core::Phase;

    fn view() -> View {
        let mut v = View::new(
            Phase::Build,
            "spacexai".into(),
            "grok-4.6".into(),
            "~/p".into(),
        );
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
                role: Role::Orchestrator,
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
                role: Role::Orchestrator,
                subagent_id: None,
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
        assert_eq!(v.spend_rows_role["lead"].calls, 1);
    }

    #[test]
    fn unknown_price_never_prints_zero() {
        let mut v = view();
        apply(
            &mut v,
            AgentEvent::Spend {
                connection: "x".into(),
                model: "m".into(),
                role: Role::Orchestrator,
                subagent_id: None,
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

    #[test]
    fn specialist_finish_posts_body_and_merge_row() {
        let mut v = view();
        let id = ryter_core::SubagentId::new("abc");
        apply(
            &mut v,
            AgentEvent::SubagentStarted {
                id: id.clone(),
                role: Role::Builder,
                description: "wire the loop".into(),
            },
        );
        assert_eq!(v.crew.len(), 1);
        apply(
            &mut v,
            AgentEvent::SubagentFinished {
                id,
                role: Role::Builder,
                summary: "merged 3 files".into(),
                body: "done.".into(),
            },
        );
        assert!(v.crew.is_empty());
        let kinds: Vec<_> = v.messages.iter().map(|m| &m.kind).collect();
        assert!(
            kinds
                .iter()
                .any(|k| matches!(k, MessageKind::Specialist { .. }))
        );
        assert!(kinds.iter().any(|k| matches!(k, MessageKind::Merge)));
    }
}
