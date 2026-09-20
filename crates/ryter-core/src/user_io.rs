//! Blocking prompts from the tool thread to a TUI (permission Ask, ask_user).

use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

use crate::tools::Decision;

const TIMEOUT: Duration = Duration::from_secs(300);

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

/// A prompt the TUI must answer.
#[derive(Debug)]
pub enum UserRequest {
    /// Destructive tool (`y` / `n` / `a`).
    Permission {
        /// Tool name.
        tool: String,
        /// One-line summary (no secrets).
        summary: String,
        /// Reply channel.
        reply: mpsc::Sender<Permission>,
    },
    /// `ask_user` question.
    Question {
        /// Prompt text.
        question: String,
        /// Optional choices (empty = free text).
        options: Vec<String>,
        /// Reply channel.
        reply: mpsc::Sender<String>,
    },
}

/// Handle held by [`crate::tools::ToolContext`].
#[derive(Clone)]
pub struct UserIo {
    tx: Arc<Mutex<mpsc::Sender<UserRequest>>>,
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
            },
            rx,
        )
    }

    /// Ask the TUI about a tool. Disconnected or timeout → Deny.
    pub fn permission(&self, tool: &str, summary: &str) -> Permission {
        let (reply_tx, reply_rx) = mpsc::channel();
        let req = UserRequest::Permission {
            tool: tool.to_string(),
            summary: summary.chars().take(160).collect(),
            reply: reply_tx,
        };
        if self.send(req).is_err() {
            return Permission::Deny;
        }
        match reply_rx.recv_timeout(TIMEOUT) {
            Ok(p) => p,
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => Permission::Deny,
        }
    }

    /// Ask the human. Empty string on timeout/disconnect.
    pub fn ask(&self, question: &str, options: Vec<String>) -> String {
        let (reply_tx, reply_rx) = mpsc::channel();
        let req = UserRequest::Question {
            question: question.to_string(),
            options,
            reply: reply_tx,
        };
        if self.send(req).is_err() {
            return String::new();
        }
        reply_rx.recv_timeout(TIMEOUT).unwrap_or_default()
    }

    fn send(&self, req: UserRequest) -> Result<(), ()> {
        let tx = self.tx.lock().map_err(|_| ())?;
        tx.send(req).map_err(|_| ())
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
        assert_eq!(io.permission("bash", "rm -rf x"), Permission::Deny);
    }

    #[test]
    fn allow_reaches_the_tool_thread() {
        let (io, rx) = UserIo::pair();
        let worker = std::thread::spawn(move || io.permission("bash", "rm"));
        match rx.recv().unwrap() {
            UserRequest::Permission { reply, .. } => {
                reply.send(Permission::Always).unwrap();
            }
            UserRequest::Question { .. } => panic!("expected permission"),
        }
        assert_eq!(worker.join().unwrap(), Permission::Always);
    }
}
