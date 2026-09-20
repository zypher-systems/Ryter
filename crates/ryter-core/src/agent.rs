//! Streaming agent loop.

use futures_util::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use crate::crew;
use crate::error::{Error, Result};
use crate::event::AgentEvent;
use crate::llm::{AssistantToolCall, CompletionRequest, Message, Provider, StreamDelta};
use crate::phase::Phase;
use crate::prompt::orchestrator_system;
use crate::queue::{TaskQueue, TaskStatus};
use crate::role::Role;
use crate::session::{Session, spend_record};
use crate::spend::{PriceBook, Usage};
use crate::tools::{ToolContext, gated_execute};

/// Why the loop stopped.
#[derive(Debug, Clone, PartialEq)]
pub enum StopReason {
    /// Model produced a final assistant message with no tool calls.
    Completed,
    /// Hit `max_turns`.
    MaxTurns,
    /// Spend cap.
    Budget,
    /// User or MCP cancelled the in-flight turn.
    Cancelled,
}

/// One user turn (may include many model/tool rounds).
#[derive(Debug, Clone, PartialEq)]
pub struct TurnResult {
    /// Stop reason.
    pub reason: StopReason,
    /// Assistant text from the last model round.
    pub text: String,
}

/// Agent loop over a session + provider.
pub struct Agent {
    /// Inference.
    pub provider: Arc<dyn Provider>,
    /// Price book.
    pub book: PriceBook,
    /// Disk session.
    pub session: Session,
    /// Tool context.
    pub ctx: ToolContext,
    /// Connection name (spend attribution).
    pub connection: String,
    /// Model id.
    pub model: String,
    /// Role for this loop (orchestrator until specialists land).
    pub role: Role,
    /// Max model rounds per user message.
    pub max_turns: u32,
    /// USD cap; `0` means none.
    pub budget_usd: f64,
    /// Optional live event subscriber (TUI / `--json`).
    pub sink: Option<Sender<AgentEvent>>,
    /// `~/.ryter` (or `RYTER_HOME`) for prompt overrides.
    pub home: PathBuf,
    /// Project root for `.ryter/prompts`.
    pub project_root: Option<PathBuf>,
    /// Whether project prompts/config are trusted.
    pub trusted: bool,
    /// Task queue (also on `ctx.queue`).
    pub queue: Arc<Mutex<TaskQueue>>,
    /// Parallelism cap.
    pub max_crew: u32,
    /// Auditor retries.
    pub max_retries: u32,
    /// Context window override. `0` uses the model default (grok-4.6 = 500k, else 200k).
    pub context_window: u64,
    /// Full config for `[specialists.*]` routing. Tests may leave this `None`.
    pub cfg: Option<crate::config::Config>,
    /// In-flight specialists (for `/agents` kill).
    pub running: Arc<Mutex<Vec<ChildHandle>>>,
}

/// One running specialist the TUI can list and kill.
#[derive(Clone)]
pub struct ChildHandle {
    /// Subagent id.
    pub id: crate::ids::SubagentId,
    /// Role.
    pub role: Role,
    /// Task title.
    pub label: String,
    /// Stop this child only.
    pub cancel: Arc<crate::cancel::Cancel>,
}

impl Agent {
    /// Run one user message to completion (or cap).
    pub async fn turn(&mut self, user: &str) -> Result<TurnResult> {
        self.session.push_message(Message {
            role: "user".into(),
            content: user.to_string(),
            tool_call_id: None,
            tool_calls: None,
        })?;
        if self.session.meta.title.is_empty() {
            let t: String = user.chars().take(80).collect();
            let _ = self.session.set_title(&t);
        }

        let mut last_text = String::new();
        for _round in 0..self.max_turns {
            if self.ctx.cancel.is_cancelled() {
                return self.finish_cancelled(last_text).await;
            }
            if self.over_budget() {
                return Err(Error::Budget {
                    spent: self.session.meta.spend_usd_total.unwrap_or(0.0),
                    cap: self.budget_usd,
                });
            }
            self.maybe_compact()?;

            let req = CompletionRequest {
                model: self.model.clone(),
                system: Some(self.system_prompt()?),
                messages: self.session.transcript.clone(),
                tools: crate::tools::specs_for_opts(self.role, self.ctx.web),
                max_tokens: Some(8192),
            };

            let mut stream = tokio::select! {
                biased;
                () = self.ctx.cancel.cancelled() => {
                    return self.finish_cancelled(last_text).await;
                }
                s = self.provider.stream(req) => s?,
            };
            let mut text = String::new();
            let mut calls: HashMap<String, AssistantToolCall> = HashMap::new();
            let mut order: Vec<String> = Vec::new();
            let mut usage = Usage::default();
            let mut reported_cost: Option<f64> = None;
            let mut anon = 0u32;

            loop {
                if self.ctx.cancel.is_cancelled() {
                    return self.finish_cancelled(last_text).await;
                }
                let delta = tokio::select! {
                    biased;
                    () = self.ctx.cancel.cancelled() => {
                        return self.finish_cancelled(if text.is_empty() { last_text } else { text }).await;
                    }
                    d = stream.next() => d,
                };
                let Some(delta) = delta else {
                    break;
                };
                match delta? {
                    StreamDelta::Text(t) => {
                        text.push_str(&t);
                        self.emit(AgentEvent::Token { text: t })?;
                    }
                    StreamDelta::Reasoning(t) => {
                        self.emit(AgentEvent::Reasoning { text: t })?;
                    }
                    StreamDelta::ToolCall {
                        id,
                        name,
                        arguments,
                    } => {
                        let key = if id.is_empty() {
                            anon += 1;
                            format!("anon-{anon}")
                        } else {
                            id.clone()
                        };
                        let is_new = !calls.contains_key(&key);
                        let entry = calls
                            .entry(key.clone())
                            .or_insert_with(|| AssistantToolCall {
                                id: key.clone(),
                                name: name.clone(),
                                arguments: String::new(),
                            });
                        if is_new {
                            order.push(key);
                        }
                        if !name.is_empty() {
                            entry.name = name;
                        }
                        if !arguments.is_empty() {
                            entry.arguments.push_str(&arguments);
                        }
                    }
                    StreamDelta::Usage(u) => usage = u,
                    StreamDelta::ReportedCost(c) => reported_cost = Some(c),
                    StreamDelta::Done => {}
                }
            }

            let total_usd = reported_cost.or_else(|| self.book.cost(&self.model, usage));
            self.session.record_spend(spend_record(
                self.connection.clone(),
                self.model.clone(),
                self.role,
                usage,
                total_usd,
            ))?;
            self.emit(AgentEvent::Spend {
                connection: self.connection.clone(),
                model: self.model.clone(),
                role: self.role,
                subagent_id: None,
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cached_tokens: usage.cached_tokens,
                total_usd,
            })?;

            if self.over_budget() {
                return Err(Error::Budget {
                    spent: self.session.meta.spend_usd_total.unwrap_or(0.0),
                    cap: self.budget_usd,
                });
            }

            last_text = text.clone();
            let call_list: Vec<AssistantToolCall> = order
                .iter()
                .filter_map(|k| calls.get(k).cloned())
                .filter(|c| !c.name.is_empty())
                .collect();

            self.session.push_message(Message {
                role: "assistant".into(),
                content: text,
                tool_call_id: None,
                tool_calls: if call_list.is_empty() {
                    None
                } else {
                    Some(call_list.clone())
                },
            })?;

            if call_list.is_empty() {
                if self.role == Role::Orchestrator {
                    self.drain_crew().await?;
                }
                if self.ctx.cancel.is_cancelled() {
                    return self.finish_cancelled(last_text).await;
                }
                return Ok(TurnResult {
                    reason: StopReason::Completed,
                    text: last_text,
                });
            }

            for call in call_list {
                if self.ctx.cancel.is_cancelled() {
                    return self.finish_cancelled(last_text).await;
                }
                let args: Value = serde_json::from_str(&call.arguments).unwrap_or(Value::Null);
                self.emit(AgentEvent::ToolCall {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    args: args.clone(),
                    role: self.role,
                })?;
                let out = gated_execute(&call.name, &args, &self.ctx)?;
                self.emit(AgentEvent::ToolResult {
                    id: call.id.clone(),
                    output: out.text.clone(),
                    is_error: out.is_error,
                })?;
                self.session.push_message(Message {
                    role: "tool".into(),
                    content: out.text,
                    tool_call_id: Some(call.id),
                    tool_calls: None,
                })?;
            }
        }

        Ok(TurnResult {
            reason: StopReason::MaxTurns,
            text: last_text,
        })
    }

    /// Run pending queue items. Orchestrator only. Role follows the current phase.
    pub async fn drain_crew(&mut self) -> Result<()> {
        if self.role != Role::Orchestrator {
            return Ok(());
        }
        let phase = self.session.meta.phase;
        let role = match phase {
            Phase::Plan => Role::Planner,
            Phase::Architect => Role::Architect,
            Phase::Build => Role::Builder,
            Phase::Audit => Role::Auditor,
        };
        loop {
            if self.ctx.cancel.is_cancelled() {
                break;
            }
            let batch = {
                let mut q = self
                    .queue
                    .lock()
                    .map_err(|e| Error::Config(e.to_string()))?;
                q.take_pending(self.max_crew)
            };
            if batch.is_empty() {
                break;
            }
            let mut jobs = Vec::new();
            let mut child_cancels: Vec<Arc<crate::cancel::Cancel>> = Vec::new();
            for task in batch {
                let sub_id = crate::queue::new_sub_id();
                let child_cancel = crate::cancel::Cancel::new();
                {
                    let mut run = self
                        .running
                        .lock()
                        .map_err(|e| Error::Config(e.to_string()))?;
                    run.push(ChildHandle {
                        id: sub_id.clone(),
                        role,
                        label: task.title.clone(),
                        cancel: child_cancel.clone(),
                    });
                }
                child_cancels.push(child_cancel.clone());
                self.emit(crew::started_event(sub_id.clone(), role, &task))?;
                let (provider, model) = self.specialist_stack(role);
                let (auditor_p, auditor_m) = self.specialist_stack(Role::Auditor);
                let workspace = self.ctx.workspace.clone();
                let home = self.home.clone();
                let session_id = self.session.meta.id.as_str().to_string();
                let always = self.ctx.always_approve;
                let web = self.ctx.web;
                let auditor_on = self.session.meta.auditor_enabled;
                let max_retries = self.max_retries;
                let project_root = self.project_root.clone();
                let trusted = self.trusted;
                let hooks = self.ctx.hooks.clone();
                let notes_dir = self.session.notes_dir();
                let pass = pass_note_for(&self.session, role);
                let cancel = child_cancel;
                let running = self.running.clone();
                let task_id = task.id.clone();
                jobs.push(async move {
                    let result = if role == Role::Builder {
                        let mut task = task;
                        loop {
                            if cancel.is_cancelled() {
                                break (
                                    sub_id,
                                    task_id.clone(),
                                    task.retries,
                                    Err(Error::Cancelled),
                                );
                            }
                            let outcome = crew::run_build_task(
                                provider.clone(),
                                &workspace,
                                &home,
                                &session_id,
                                &task,
                                &model,
                                auditor_p.clone(),
                                &auditor_m,
                                always,
                                web,
                                auditor_on,
                                max_retries,
                                project_root.as_deref(),
                                trusted,
                                hooks.clone(),
                                cancel.clone(),
                            )
                            .await;
                            match outcome {
                                Ok(o) if o.status == TaskStatus::Pending => {
                                    task.retries += 1;
                                    task.findings = o.findings.clone();
                                    continue;
                                }
                                other => {
                                    break (sub_id, task_id.clone(), task.retries, other);
                                }
                            }
                        }
                    } else if cancel.is_cancelled() {
                        (sub_id, task_id, task.retries, Err(Error::Cancelled))
                    } else {
                        let outcome = crew::run_note_task(
                            provider,
                            &workspace,
                            &home,
                            &task,
                            &model,
                            role,
                            true,
                            web,
                            project_root.as_deref(),
                            trusted,
                            hooks,
                            notes_dir,
                            &pass,
                            cancel,
                        )
                        .await;
                        (sub_id, task_id, task.retries, outcome)
                    };
                    if let Ok(mut run) = running.lock() {
                        run.retain(|c| c.id.as_str() != result.0.as_str());
                    }
                    result
                });
            }
            let parent = self.ctx.cancel.clone();
            let kids = child_cancels.clone();
            let watch = tokio::spawn(async move {
                parent.cancelled().await;
                for k in kids {
                    k.cancel();
                }
            });
            let results = futures_util::future::join_all(jobs).await;
            watch.abort();
            let mut combined = String::new();
            for (sub_id, task_id, retries, outcome) in results {
                match outcome {
                    Ok(outcome) => {
                        if role != Role::Builder && !outcome.findings.trim().is_empty() {
                            if !combined.is_empty() {
                                combined.push_str("\n\n---\n\n");
                            }
                            combined.push_str(&outcome.findings);
                        }
                        {
                            let mut q = self
                                .queue
                                .lock()
                                .map_err(|e| Error::Config(e.to_string()))?;
                            q.set(&outcome.id, outcome.status, &outcome.findings);
                            if let Some(t) = q.tasks.iter_mut().find(|t| t.id == outcome.id) {
                                t.retries = retries;
                            }
                        }
                        self.emit(crew::finished_event(
                            sub_id,
                            role,
                            outcome.summary,
                            outcome.findings,
                        ))?;
                    }
                    Err(Error::Cancelled) => {
                        {
                            let mut q = self
                                .queue
                                .lock()
                                .map_err(|e| Error::Config(e.to_string()))?;
                            q.set(&task_id, TaskStatus::Blocked, "cancelled");
                        }
                        self.emit(crew::finished_event(
                            sub_id,
                            role,
                            "cancelled".into(),
                            String::new(),
                        ))?;
                    }
                    Err(e) => {
                        self.emit(AgentEvent::Error {
                            message: e.to_string(),
                        })?;
                    }
                }
            }
            if role != Role::Builder && !combined.is_empty() {
                let _ = self.session.write_note(phase, &combined);
            }
        }
        Ok(())
    }

    /// Switch phase, write a pass note (may be empty), keep the transcript.
    pub fn handoff(&mut self, to: Phase, note: &str, back_reason: Option<&str>) -> Result<()> {
        if let Some(hooks) = &self.ctx.hooks {
            if let crate::hooks::HookDecision::Deny(msg) =
                hooks.handoff(self.session.meta.phase, to, note, &self.ctx.workspace)
            {
                return Err(Error::Config(format!("handoff hook denied: {msg}")));
            }
        }
        self.session.handoff(to, note, back_reason)?;
        self.emit(AgentEvent::PhaseChanged { phase: to })
    }

    /// `/context` snapshot (does not compact).
    pub fn context_report(&self) -> Result<crate::compact::ContextReport> {
        let sys = self.system_prompt().unwrap_or_default();
        Ok(crate::compact::report(
            &sys,
            &self.session.transcript,
            &self.model,
            self.context_window,
        ))
    }

    /// Emit a [`AgentEvent::Context`] for the TUI / `--json`.
    pub fn emit_context(&mut self) -> Result<()> {
        let r = self.context_report()?;
        self.emit(AgentEvent::Context {
            tokens: r.tokens,
            window: r.window,
            pct: r.pct,
            messages: r.messages,
        })
    }

    /// Deterministic compact. Always emits [`AgentEvent::Compacted`].
    pub fn compact_now(&mut self) -> Result<crate::compact::ContextReport> {
        let sys = self.system_prompt().unwrap_or_default();
        let before = crate::compact::estimate_tokens(&sys, &self.session.transcript);
        let note = self
            .session
            .read_note(self.session.meta.phase)
            .unwrap_or_default();
        let next = crate::compact::compact(
            &self.session.transcript,
            &note,
            crate::compact::KEEP_USER_TURNS,
        );
        if next.len() < self.session.transcript.len() {
            self.session.replace_transcript(next)?;
        }
        let mut rep = crate::compact::report(
            &sys,
            &self.session.transcript,
            &self.model,
            self.context_window,
        );
        rep.compacted = true;
        self.emit(AgentEvent::Compacted {
            before,
            after: rep.tokens,
            window: rep.window,
        })?;
        Ok(rep)
    }

    fn maybe_compact(&mut self) -> Result<()> {
        let rep = self.context_report()?;
        if crate::compact::should_compact(&rep) {
            self.compact_now()?;
        }
        Ok(())
    }

    /// Run SessionStart hooks. Call once after the agent is constructed.
    pub fn fire_session_start(&self) -> Result<()> {
        let Some(hooks) = &self.ctx.hooks else {
            return Ok(());
        };
        match hooks.session_start(&self.ctx.workspace, self.session.meta.phase) {
            crate::hooks::HookDecision::Allow => Ok(()),
            crate::hooks::HookDecision::Deny(msg) => {
                Err(Error::Config(format!("session start hook denied: {msg}")))
            }
        }
    }

    fn specialist_stack(&self, role: Role) -> (Arc<dyn Provider>, String) {
        let Some(cfg) = &self.cfg else {
            return (self.provider.clone(), self.model.clone());
        };
        if cfg.follows_orchestrator(role) {
            return (self.provider.clone(), self.model.clone());
        }
        let (conn, model) = cfg.route_for(role);
        if conn == self.connection && model == self.model {
            return (self.provider.clone(), model);
        }
        if let Ok(key) = crate::config::resolve_secret(cfg, &crate::ids::ConnectionId::new(&conn)) {
            if let Some(c) = cfg.connections.get(&conn) {
                return (Arc::new(crate::llm::http_provider(c, key)), model);
            }
        }
        (self.provider.clone(), self.model.clone())
    }

    fn system_prompt(&self) -> Result<String> {
        orchestrator_system(
            &self.home,
            self.project_root.as_deref(),
            self.trusted,
            &self.session,
        )
    }

    async fn finish_cancelled(&mut self, text: String) -> Result<TurnResult> {
        self.kill_all_children();
        self.emit(AgentEvent::Cancelled)?;
        Ok(TurnResult {
            reason: StopReason::Cancelled,
            text,
        })
    }

    /// Stop every in-flight specialist.
    pub fn kill_all_children(&self) {
        if let Ok(run) = self.running.lock() {
            for c in run.iter() {
                c.cancel.cancel();
            }
        }
    }

    /// Stop one specialist by id. Returns false if it was not running.
    pub fn kill_child(&self, id: &str) -> bool {
        let Ok(run) = self.running.lock() else {
            return false;
        };
        if let Some(c) = run.iter().find(|c| c.id.as_str() == id) {
            c.cancel.cancel();
            true
        } else {
            false
        }
    }

    fn over_budget(&self) -> bool {
        if self.budget_usd <= 0.0 {
            return false;
        }
        self.session
            .meta
            .spend_usd_total
            .is_some_and(|s| s >= self.budget_usd)
    }

    fn emit(&mut self, ev: AgentEvent) -> Result<()> {
        self.session.emit(&ev)?;
        if let Some(s) = &self.sink {
            let _ = s.send(ev);
        }
        Ok(())
    }
}

fn pass_note_for(session: &crate::session::Session, role: Role) -> String {
    match role {
        Role::Architect => session.read_note(Phase::Plan).unwrap_or_default(),
        Role::Planner => session.read_note(Phase::Plan).unwrap_or_default(),
        Role::Auditor => {
            let mut s = session.read_note(Phase::Build).unwrap_or_default();
            let a = session.read_note(Phase::Architect).unwrap_or_default();
            if !a.is_empty() {
                if !s.is_empty() {
                    s.push_str("\n\n");
                }
                s.push_str(&a);
            }
            s
        }
        Role::Builder | Role::Orchestrator => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ReplayProvider;
    use crate::phase::Phase;
    use crate::tools::ToolContext;
    use tempfile::TempDir;

    fn setup(provider: ReplayProvider) -> (TempDir, TempDir, Agent) {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        std::fs::write(cwd.path().join("hello.txt"), "hi there").unwrap();
        let session = Session::create(
            home.path(),
            cwd.path(),
            Phase::Build,
            "spacexai".into(),
            "grok-4.6".into(),
        )
        .unwrap();
        let notes = session.notes_dir();
        let queue = Arc::new(Mutex::new(crate::queue::TaskQueue::open(
            session.dir.join("tasks.json"),
        )));
        let ctx = ToolContext {
            workspace: cwd.path().to_path_buf(),
            notes_dir: notes,
            role: Role::Orchestrator,
            always_approve: true,
            queue: queue.clone(),
            mcp: None,
            hooks: None,
            cancel: crate::cancel::Cancel::new(),
            user_io: None,
            sticky_approve: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            web: false,
        };
        let agent = Agent {
            provider: Arc::new(provider),
            book: PriceBook::new(),
            session,
            ctx,
            connection: "spacexai".into(),
            model: "grok-4.6".into(),
            role: Role::Orchestrator,
            max_turns: 8,
            budget_usd: 0.0,
            sink: None,
            home: home.path().to_path_buf(),
            project_root: Some(cwd.path().to_path_buf()),
            trusted: false,
            queue,
            max_crew: 2,
            max_retries: 2,
            context_window: 0,
            cfg: None,
            running: Arc::new(Mutex::new(Vec::new())),
        };
        (home, cwd, agent)
    }

    #[tokio::test]
    async fn text_only_turn() {
        let p = ReplayProvider::new(vec![
            StreamDelta::Text("hello".into()),
            StreamDelta::Usage(Usage {
                input_tokens: 10,
                output_tokens: 2,
                cached_tokens: 0,
            }),
            StreamDelta::Done,
        ]);
        let (_home, _cwd, mut agent) = setup(p);
        let r = agent.turn("say hi").await.unwrap();
        assert_eq!(r.reason, StopReason::Completed);
        assert_eq!(r.text, "hello");
        assert!(agent.session.meta.spend_usd_total.is_some());
        assert!(!agent.session.spend_log().unwrap().is_empty());
    }

    #[tokio::test]
    async fn tool_then_answer() {
        let args = serde_json::json!({"path":"hello.txt"}).to_string();
        let p = ReplayProvider::scripted(vec![
            vec![
                StreamDelta::ToolCall {
                    id: "c1".into(),
                    name: "read_file".into(),
                    arguments: args,
                },
                StreamDelta::Usage(Usage {
                    input_tokens: 8,
                    output_tokens: 4,
                    cached_tokens: 0,
                }),
                StreamDelta::Done,
            ],
            vec![
                StreamDelta::Text("the file says hi there".into()),
                StreamDelta::Usage(Usage {
                    input_tokens: 20,
                    output_tokens: 6,
                    cached_tokens: 0,
                }),
                StreamDelta::Done,
            ],
        ]);
        let (_home, _cwd, mut agent) = setup(p);
        let r = agent.turn("read hello").await.unwrap();
        assert_eq!(r.reason, StopReason::Completed);
        assert!(r.text.contains("hi there"));
        assert!(
            agent
                .session
                .transcript
                .iter()
                .any(|m| m.role == "tool" && m.content.contains("hi there"))
        );
    }

    struct DelayProvider {
        delay: std::time::Duration,
        inner: ReplayProvider,
    }

    #[async_trait::async_trait]
    impl Provider for DelayProvider {
        async fn stream(&self, req: CompletionRequest) -> Result<crate::llm::DeltaStream> {
            tokio::time::sleep(self.delay).await;
            self.inner.stream(req).await
        }
        async fn list_models(&self) -> Result<Vec<crate::llm::ModelInfo>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn cancel_stops_a_turn() {
        let inner = ReplayProvider::new(vec![StreamDelta::Text("late".into()), StreamDelta::Done]);
        let (_home, _cwd, mut agent) = setup(ReplayProvider::new(vec![StreamDelta::Done]));
        agent.provider = Arc::new(DelayProvider {
            delay: std::time::Duration::from_secs(8),
            inner,
        });
        let cancel = agent.ctx.cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            cancel.cancel();
        });
        let start = std::time::Instant::now();
        let r = agent.turn("hi").await.unwrap();
        assert_eq!(r.reason, StopReason::Cancelled);
        assert!(start.elapsed() < std::time::Duration::from_secs(3));
    }

    #[tokio::test]
    async fn budget_stops_the_loop() {
        let p = ReplayProvider::new(vec![
            StreamDelta::Text("x".into()),
            StreamDelta::Usage(Usage {
                input_tokens: 1_000_000,
                output_tokens: 1_000_000,
                cached_tokens: 0,
            }),
            StreamDelta::Done,
        ]);
        let (_home, _cwd, mut agent) = setup(p);
        agent.budget_usd = 0.01; // grok-4.6 1M/1M is long-context $16
        let err = agent.turn("go").await.unwrap_err();
        assert!(matches!(err, Error::Budget { .. }), "{err}");
    }

    #[tokio::test]
    async fn handoff_keeps_transcript_and_updates_phase() {
        let p = ReplayProvider::new(vec![StreamDelta::Text("ok".into()), StreamDelta::Done]);
        let (_home, _cwd, mut agent) = setup(p);
        agent.turn("hello").await.unwrap();
        let n = agent.session.transcript.len();
        assert!(n >= 2);
        agent
            .handoff(Phase::Architect, "empty is fine", None)
            .unwrap();
        assert_eq!(agent.session.meta.phase, Phase::Architect);
        assert_eq!(agent.session.transcript.len(), n);
        let sys = agent.system_prompt().unwrap();
        assert!(sys.contains("architect"));
        assert!(sys.contains("empty is fine"));
        assert!(!Phase::Architect.allows(Role::Builder));
    }

    #[tokio::test]
    async fn unknown_model_does_not_record_zero_dollars() {
        let p = ReplayProvider::new(vec![
            StreamDelta::Text("ok".into()),
            StreamDelta::Usage(Usage {
                input_tokens: 100,
                output_tokens: 10,
                cached_tokens: 0,
            }),
            StreamDelta::Done,
        ]);
        let (_home, _cwd, mut agent) = setup(p);
        agent.model = "mystery-model".into();
        let _ = agent.turn("x").await.unwrap();
        assert!(agent.session.meta.spend_unknown);
        assert_eq!(agent.session.meta.spend_usd_total, None);
        let rec = &agent.session.spend_log().unwrap()[0];
        assert_eq!(rec.total_usd, None);
    }

    #[tokio::test]
    async fn handoff_hook_can_block() {
        let p = ReplayProvider::new(vec![StreamDelta::Text("ok".into()), StreamDelta::Done]);
        let (_home, cwd, mut agent) = setup(p);
        let script = cwd.path().join("deny.sh");
        std::fs::write(&script, "#!/bin/sh\necho stay\nexit 2\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        agent.ctx.hooks = Some(Arc::new(crate::hooks::HookSet::from_config(&[
            crate::config::HookConfig {
                event: "Handoff".into(),
                command: Some(script.to_string_lossy().into_owned()),
                url: None,
                matcher: None,
            },
        ])));
        let err = agent.handoff(Phase::Architect, "x", None).unwrap_err();
        assert!(err.to_string().contains("handoff hook denied"), "{err}");
        assert_eq!(agent.session.meta.phase, Phase::Build);
    }

    #[tokio::test]
    async fn auto_compact_shrinks_long_transcript() {
        let p = ReplayProvider::new(vec![StreamDelta::Text("done".into()), StreamDelta::Done]);
        let (_home, _cwd, mut agent) = setup(p);
        agent.context_window = 200;
        for i in 0..10 {
            agent
                .session
                .push_message(Message {
                    role: "user".into(),
                    content: format!("turn {i} {}", "blob ".repeat(40)),
                    tool_call_id: None,
                    tool_calls: None,
                })
                .unwrap();
            agent
                .session
                .push_message(Message {
                    role: "assistant".into(),
                    content: "ok".into(),
                    tool_call_id: None,
                    tool_calls: None,
                })
                .unwrap();
        }
        let before = agent.session.transcript.len();
        agent.turn("latest please").await.unwrap();
        assert!(
            agent.session.transcript.len() < before + 4,
            "len {} vs before {before}",
            agent.session.transcript.len()
        );
        assert!(
            agent
                .session
                .transcript
                .iter()
                .any(|m| m.content.contains("[compacted")),
            "{:?}",
            agent.session.transcript[0].content
        );
        let reopened = Session::open(&agent.session.dir).unwrap();
        assert_eq!(reopened.transcript.len(), agent.session.transcript.len());
    }

    #[tokio::test]
    async fn plan_phase_spawns_planner_and_writes_notes() {
        let p = ReplayProvider::scripted(vec![vec![
            StreamDelta::Text("## Plan\nShip a CLI flag.\n".into()),
            StreamDelta::Done,
        ]]);
        let (_home, cwd, mut agent) = setup(p);
        agent
            .session
            .handoff(Phase::Plan, "need a flag", None)
            .unwrap();
        {
            let mut q = agent.queue.lock().unwrap();
            q.apply_todo(&serde_json::json!({"items": ["draft the plan"]}))
                .unwrap();
        }
        agent.drain_crew().await.unwrap();
        let note = agent.session.read_note(Phase::Plan).unwrap();
        assert!(note.contains("CLI flag"), "{note}");
        let disk = std::fs::read_to_string(cwd.path().join("notes/plan.md")).unwrap();
        assert!(disk.contains("CLI flag"), "{disk}");
        let q = agent.queue.lock().unwrap();
        assert_eq!(q.tasks[0].status, crate::queue::TaskStatus::Done);
    }

    #[tokio::test]
    async fn plan_phase_runs_two_tasks() {
        let p = ReplayProvider::new(vec![StreamDelta::Text("planned".into()), StreamDelta::Done]);
        let (_home, cwd, mut agent) = setup(p);
        agent.session.handoff(Phase::Plan, "", None).unwrap();
        {
            let mut q = agent.queue.lock().unwrap();
            q.apply_todo(&serde_json::json!({"items": ["one", "two"]}))
                .unwrap();
        }
        agent.drain_crew().await.unwrap();
        let q = agent.queue.lock().unwrap();
        assert_eq!(q.tasks.len(), 2);
        assert!(
            q.tasks
                .iter()
                .all(|t| t.status == crate::queue::TaskStatus::Done)
        );
        let disk = std::fs::read_to_string(cwd.path().join("notes/plan.md")).unwrap();
        assert!(disk.contains("planned"), "{disk}");
    }
}
