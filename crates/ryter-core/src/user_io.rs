//! Blocking prompts from the tool thread to a TUI (permission Ask, ask_user).

use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

use crate::cancel::Cancel;
use crate::tools::Decision;

const TIMEOUT: Duration = Duration::from_secs(300);
/// How often a waiting prompt checks whether the turn was cancelled.
const POLL: Duration = Duration::from_millis(100);

/// Reply to a destructive-tool prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// Run this call only.
    Allow,
    /// Do not run.
    Deny,
    /// Run, and treat further Ask as Allow this session.
    Always,
}

/// What the user said to a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanAnswer {
    /// Carry it out.
    Approve,
    /// Change it first; what to change, in their words.
    Adjust(String),
    /// Don't.
    Reject,
}

/// A prompt the TUI must answer.
#[derive(Debug)]
pub enum UserRequest {
    /// Destructive tool (`y` / `n` / `a`).
    Permission {
        /// Tool name.
        tool: String,
        /// One-line summary (no secrets).
        summary: String,
        /// For an edit: what it would do to the file, measured before asking.
        preview: Option<Box<crate::diff::FileDiff>>,
        /// Only `y` answers yes: destruction an undo may not reach.
        strict: bool,
        /// What `a` would allow for the session, in words; `None`: no `a`.
        scope: Option<String>,
        /// `preview` is the only view of the change there will be: show
        /// all of it, and take a yes only once its end has been shown.
        whole: bool,
        /// For a script of several commands: the one that asked, which the
        /// card names first.
        asks: Option<String>,
        /// Reply channel.
        reply: mpsc::Sender<Permission>,
    },
    /// A plan to approve before any work on it starts.
    Plan {
        /// A few words: the panel's title.
        title: String,
        /// The plan, in Markdown.
        plan: String,
        /// Reply channel.
        reply: mpsc::Sender<PlanAnswer>,
    },
    /// How the project runs, to approve before Ryter runs any of it.
    Run {
        /// `start`, `ready`, `test`, `stop`, each with its command.
        rows: Vec<(String, String)>,
        /// Why it is being asked again, when it is: the file changed.
        note: Option<String>,
        /// Reply channel.
        reply: mpsc::Sender<PlanAnswer>,
    },
    /// A question: the model's `ask_user`, or Ryter's own.
    Question {
        /// Who asks, when it is Ryter and not the model: `task budget`,
        /// `checks`. `None` is the model.
        title: Option<String>,
        /// Prompt text.
        question: String,
        /// Optional choices (empty = free text).
        options: Vec<String>,
        /// Reply channel.
        reply: mpsc::Sender<String>,
    },
    /// sudo wants the user's password, for a command they approved to run
    /// as root. What is typed goes to sudo and nowhere else.
    Password {
        /// What sudo asks, in its words.
        prompt: String,
        /// sudo is asking again at once: the last answer was not accepted.
        again: bool,
        /// Reply channel: the password and whether to keep it for a few
        /// minutes, or `None` to refuse.
        reply: mpsc::Sender<Option<(String, bool)>>,
    },
}

/// A tool call to ask about, and how the prompt may be answered.
#[derive(Debug, Clone, Default)]
pub struct ToolAsk {
    /// Tool name.
    pub tool: String,
    /// One-line summary.
    pub summary: String,
    /// For an edit: the change it would make.
    pub preview: Option<crate::diff::FileDiff>,
    /// Only `y` answers yes.
    pub strict: bool,
    /// What `a` would allow for the session; `None`: no `a`.
    pub scope: Option<String>,
    /// `preview` is the only view of the change there will be: show all of
    /// it, and take a yes only once its end has been shown.
    pub whole: bool,
    /// For a script of several commands: the one that asked, which the
    /// card names first. `None`: the summary's first line is the name.
    pub asks: Option<String>,
}

/// Handle held by [`crate::tools::ToolContext`].
#[derive(Clone)]
pub struct UserIo {
    tx: Arc<Mutex<mpsc::Sender<UserRequest>>>,
    /// The permission cards shown since the agent last took them, for the
    /// session's log.
    asked: Arc<Mutex<Vec<Asked>>>,
    /// Where sudo gets its password, started with the first root command
    /// the user approves. `Err` inside: why it could not be set up.
    #[cfg(unix)]
    askpass: Arc<std::sync::OnceLock<Result<Arc<crate::sudo::Askpass>, String>>>,
}

/// The TUI's password panel, as sudo's askpass reaches it.
#[cfg(unix)]
struct PasswordPanel(Arc<Mutex<mpsc::Sender<UserRequest>>>);

#[cfg(unix)]
impl crate::sudo::PasswordSource for PasswordPanel {
    fn password(&self, prompt: &str, again: bool) -> Option<crate::sudo::Typed> {
        let (reply, answer) = mpsc::channel();
        self.0
            .lock()
            .ok()?
            .send(UserRequest::Password {
                prompt: prompt.to_string(),
                again,
                reply,
            })
            .ok()?;
        // No turn to cancel it from here: a cancelled turn ends the command,
        // and the TUI closes the panel, which ends this wait.
        let (password, remember) = wait(&answer, &Cancel::new())??;
        Some(crate::sudo::Typed { password, remember })
    }
}

/// A permission card and what came of it. The agent turns each into an
/// [`crate::AgentEvent::Asked`] beside the tool call that put it up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
    /// The card's title: the tool, or the tool and why.
    pub tool: String,
    /// What was asked, as the card showed it.
    pub what: String,
    /// Only `y` would do.
    pub strict: bool,
    /// `allow`, `always`, `deny`, or `none` when nobody answered.
    pub answer: String,
    /// How long the person took.
    pub waited_ms: u64,
    /// When the card opened, milliseconds since the Unix epoch.
    pub at: u64,
}

impl std::fmt::Debug for UserIo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserIo").finish_non_exhaustive()
    }
}

impl UserIo {
    /// Pair: tool-side handle + TUI receiver.
    pub fn pair() -> (Self, mpsc::Receiver<UserRequest>) {
        let (tx, rx) = mpsc::channel();
        (
            Self {
                tx: Arc::new(Mutex::new(tx)),
                asked: Arc::default(),
                #[cfg(unix)]
                askpass: Arc::default(),
            },
            rx,
        )
    }

    /// Where sudo gets its password for a root command the user approved:
    /// set up under `home` the first time one runs. `Err`: why it could
    /// not be (no sudo here, a folder that can't be made).
    #[cfg(unix)]
    pub fn askpass(&self, home: &std::path::Path) -> Result<Arc<crate::sudo::Askpass>, String> {
        self.askpass
            .get_or_init(|| {
                crate::sudo::Askpass::start(home, Arc::new(PasswordPanel(self.tx.clone())))
                    .map(Arc::new)
                    .map_err(|e| e.to_string())
            })
            .clone()
    }

    /// Use `askpass` for root commands instead of setting one up.
    #[cfg(unix)]
    pub fn set_askpass(&self, askpass: Arc<crate::sudo::Askpass>) {
        let _ = self.askpass.set(Ok(askpass));
    }

    /// The cards shown since the last call, oldest first.
    pub fn take_asked(&self) -> Vec<Asked> {
        self.asked
            .lock()
            .map(|mut a| std::mem::take(&mut *a))
            .unwrap_or_default()
    }

    /// Ask the TUI about a tool. Disconnected, timeout, or a cancelled turn
    /// → Deny. The caller checks `cancel` to tell a cancel from a no.
    pub fn permission(&self, tool: &str, summary: &str, cancel: &Cancel) -> Permission {
        self.permission_with(tool, summary, None, cancel)
    }

    /// [`Self::permission`] for an edit, showing the change it would make.
    pub fn permission_with(
        &self,
        tool: &str,
        summary: &str,
        preview: Option<crate::diff::FileDiff>,
        cancel: &Cancel,
    ) -> Permission {
        self.ask_tool(
            ToolAsk {
                tool: tool.to_string(),
                summary: summary.to_string(),
                preview,
                strict: false,
                scope: None,
                whole: false,
                asks: None,
            },
            cancel,
        )
    }

    /// Ask about a tool call.
    pub fn ask_tool(&self, ask: ToolAsk, cancel: &Cancel) -> Permission {
        let (reply_tx, reply_rx) = mpsc::channel();
        // Wrapped in the prompt; the cap keeps a pasted blob from
        // filling it. An edit's whole change rides in `preview`.
        let summary: String = ask.summary.chars().take(400).collect();
        let mut record = Asked {
            tool: ask.tool.clone(),
            what: ask.asks.clone().unwrap_or_else(|| summary.clone()),
            strict: ask.strict,
            answer: "none".into(),
            waited_ms: 0,
            at: crate::event::now_ms(),
        };
        let opened = std::time::Instant::now();
        let req = UserRequest::Permission {
            tool: ask.tool,
            summary,
            preview: ask.preview.map(Box::new),
            strict: ask.strict,
            scope: ask.scope,
            whole: ask.whole,
            asks: ask.asks,
            reply: reply_tx,
        };
        let answer = if self.send(req).is_err() {
            None
        } else {
            wait(&reply_rx, cancel)
        };
        record.waited_ms = u64::try_from(opened.elapsed().as_millis()).unwrap_or(u64::MAX);
        record.answer = match answer {
            Some(Permission::Allow) => "allow",
            Some(Permission::Always) => "always",
            Some(Permission::Deny) => "deny",
            None => "none",
        }
        .into();
        if let Ok(mut a) = self.asked.lock() {
            a.push(record);
        }
        answer.unwrap_or(Permission::Deny)
    }

    /// Ask the human. Empty string on timeout, disconnect, or cancel.
    pub fn ask(&self, question: &str, options: Vec<String>, cancel: &Cancel) -> String {
        self.ask_as(None, question, options, cancel)
    }

    /// [`Self::ask`] for a question Ryter asks, under `title`.
    pub fn ask_as(
        &self,
        title: Option<&str>,
        question: &str,
        options: Vec<String>,
        cancel: &Cancel,
    ) -> String {
        let (reply_tx, reply_rx) = mpsc::channel();
        let req = UserRequest::Question {
            title: title.map(str::to_string),
            question: question.to_string(),
            options,
            reply: reply_tx,
        };
        if self.send(req).is_err() {
            return String::new();
        }
        wait(&reply_rx, cancel).unwrap_or_default()
    }

    /// Show the user a plan and wait for their answer. A plan takes reading,
    /// so this waits as long as they take: a prompt's five-minute limit
    /// would reject a plan the user was still on. No answer (the turn was
    /// cancelled, or nobody is there) is a rejection.
    pub fn present_plan(&self, title: &str, plan: &str, cancel: &Cancel) -> PlanAnswer {
        let (reply_tx, reply_rx) = mpsc::channel();
        let req = UserRequest::Plan {
            title: title.to_string(),
            plan: plan.to_string(),
            reply: reply_tx,
        };
        if self.send(req).is_err() {
            return PlanAnswer::Reject;
        }
        loop {
            if cancel.is_cancelled() {
                return PlanAnswer::Reject;
            }
            match reply_rx.recv_timeout(POLL) {
                Ok(answer) => return answer,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return PlanAnswer::Reject,
            }
        }
    }

    /// Show the user how the project runs (its start, ready, test and stop
    /// commands) and wait for their answer, as for a plan. No answer is a
    /// rejection.
    pub fn present_run(
        &self,
        rows: Vec<(String, String)>,
        note: Option<String>,
        cancel: &Cancel,
    ) -> PlanAnswer {
        let (reply_tx, reply_rx) = mpsc::channel();
        let req = UserRequest::Run {
            rows,
            note,
            reply: reply_tx,
        };
        if self.send(req).is_err() {
            return PlanAnswer::Reject;
        }
        loop {
            if cancel.is_cancelled() {
                return PlanAnswer::Reject;
            }
            match reply_rx.recv_timeout(POLL) {
                Ok(answer) => return answer,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return PlanAnswer::Reject,
            }
        }
    }

    fn send(&self, req: UserRequest) -> Result<(), ()> {
        let tx = self.tx.lock().map_err(|_| ())?;
        tx.send(req).map_err(|_| ())
    }
}

/// Wait for a reply, giving up at the timeout or as soon as the turn is
/// cancelled. Esc used to leave the agent blocked here for five minutes
/// while the screen said "cancelling…".
fn wait<T>(rx: &mpsc::Receiver<T>, cancel: &Cancel) -> Option<T> {
    let deadline = std::time::Instant::now() + TIMEOUT;
    loop {
        if cancel.is_cancelled() {
            return None;
        }
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return None;
        }
        match rx.recv_timeout(left.min(POLL)) {
            Ok(v) => return Some(v),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return None,
        }
    }
}

/// Compact args for a permission line (never dump file bodies).
pub fn summary_args(name: &str, args: &Value) -> String {
    match name {
        "bash" => args
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("bash")
            .chars()
            .take(120)
            .collect(),
        "write" | "search_replace" | "read_file" => args
            .get("path")
            .or_else(|| args.get("target_file"))
            .and_then(Value::as_str)
            .unwrap_or(name)
            .to_string(),
        // The fast path shows the whole change: the person approving it is
        // the sign-off, so they must see exactly what they are approving.
        "propose_edit" => {
            let path = args.get("path").and_then(Value::as_str).unwrap_or("?");
            let why = args.get("reason").and_then(Value::as_str).unwrap_or("");
            let mut s = format!("--- {path}  {why}\n");
            for l in args
                .get("old_string")
                .and_then(Value::as_str)
                .unwrap_or("")
                .lines()
            {
                s.push_str(&format!("-{l}\n"));
            }
            for l in args
                .get("new_string")
                .and_then(Value::as_str)
                .unwrap_or("")
                .lines()
            {
                s.push_str(&format!("+{l}\n"));
            }
            s
        }
        _ => name.to_string(),
    }
}

impl From<Permission> for Decision {
    fn from(p: Permission) -> Self {
        match p {
            Permission::Allow | Permission::Always => Decision::Allow,
            Permission::Deny => Decision::Deny,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_without_tui_is_deny() {
        let (io, _rx) = UserIo::pair();
        drop(_rx);
        assert_eq!(
            io.permission("bash", "rm -rf x", &Cancel::new()),
            Permission::Deny
        );
    }

    /// Esc while a prompt is open: the tool thread stops waiting at once.
    #[test]
    fn a_cancel_ends_the_wait() {
        let (io, _rx) = UserIo::pair();
        let cancel = Cancel::new();
        let c = cancel.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            c.cancel();
        });
        let started = std::time::Instant::now();
        assert_eq!(io.ask("which?", vec![], &cancel), "");
        assert!(started.elapsed() < Duration::from_secs(2));
        t.join().unwrap();
    }

    #[test]
    fn allow_reaches_the_tool_thread() {
        let (io, rx) = UserIo::pair();
        let worker = std::thread::spawn(move || io.permission("bash", "rm", &Cancel::new()));
        match rx.recv().unwrap() {
            UserRequest::Permission { reply, .. } => {
                reply.send(Permission::Always).unwrap();
            }
            other => panic!("expected permission, got {other:?}"),
        }
        assert_eq!(worker.join().unwrap(), Permission::Always);
    }

    /// A plan is waited on for as long as the user reads it, and no answer
    /// is a rejection: the turn cancelled, or nobody there.
    #[test]
    fn a_plan_waits_for_its_answer() {
        let (io, rx) = UserIo::pair();
        let worker =
            std::thread::spawn(move || io.present_plan("Title", "## Goal", &Cancel::new()));
        match rx.recv().unwrap() {
            UserRequest::Plan { title, plan, reply } => {
                assert_eq!((title.as_str(), plan.as_str()), ("Title", "## Goal"));
                reply.send(PlanAnswer::Adjust("more".into())).unwrap();
            }
            other => panic!("expected a plan, got {other:?}"),
        }
        assert_eq!(worker.join().unwrap(), PlanAnswer::Adjust("more".into()));
        // The panel closed without an answer.
        let (io, rx) = UserIo::pair();
        let worker = std::thread::spawn(move || io.present_plan("T", "p", &Cancel::new()));
        drop(rx.recv().unwrap());
        assert_eq!(worker.join().unwrap(), PlanAnswer::Reject);
        // The turn was cancelled while it was up.
        let (io, rx) = UserIo::pair();
        let cancel = Cancel::new();
        let c = cancel.clone();
        let worker = std::thread::spawn(move || io.present_plan("T", "p", &c));
        let held = rx.recv().unwrap();
        cancel.cancel();
        assert_eq!(worker.join().unwrap(), PlanAnswer::Reject);
        drop(held);
    }
}
