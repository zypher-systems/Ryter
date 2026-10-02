//! How a project runs: `.ryter/run.toml`, and the product a test started.
//!
//! The tester has to start the product, know when it is up, run its tests,
//! and stop it. Those commands are the project's own, so they are kept in
//! the project, drafted by the model and approved by the user. Ryter runs
//! what was approved itself: a server that stays in the foreground can't be
//! started through a shell tool that waits for its command to end.
//!
//! Approval is of the file's content, and is kept in Ryter's home folder,
//! not in the project: a run file that arrives with a clone, or is edited
//! afterwards, has to be approved again before anything in it runs.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::error::{Error, Result};

/// The file, under the project's top.
pub const FILE: &str = ".ryter/run.toml";

/// How long a start command, and the wait for the product to answer, may
/// take together.
pub const START_TIMEOUT: Duration = Duration::from_secs(300);

/// How long one test command or the stop command may take.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(600);

/// With no address to ask, how long a start command that stays in the
/// foreground is watched before the product is taken to be up.
const SETTLE: Duration = Duration::from_secs(3);

/// The project's own commands.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunFile {
    /// Starts the product. It may return (`docker compose up -d`) or stay
    /// in the foreground (`npm run dev`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    /// An address that answers once the product is up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<String>,
    /// The project's tests and checks, in order.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "one_or_many"
    )]
    pub test: Vec<String>,
    /// Stops what `start` started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<String>,
}

/// `test = "pytest"` as well as `test = ["pytest", "ruff check ."]`.
fn one_or_many<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(s) => vec![s],
        OneOrMany::Many(v) => v,
    })
}

/// A string as TOML writes it.
fn quoted(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

impl RunFile {
    /// One line each, trimmed; nothing that is empty.
    pub fn tidy(mut self) -> Self {
        let one = |s: String| {
            let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
            (!s.is_empty()).then_some(s)
        };
        self.start = self.start.and_then(one);
        self.ready = self.ready.and_then(one);
        self.stop = self.stop.and_then(one);
        self.test = self.test.into_iter().filter_map(one).collect();
        self
    }

    /// Whether it says nothing at all.
    pub fn is_empty(&self) -> bool {
        self.start.is_none() && self.ready.is_none() && self.test.is_empty() && self.stop.is_none()
    }

    /// Every command in it, in the order they would run.
    pub fn commands(&self) -> Vec<&str> {
        self.start
            .iter()
            .chain(self.test.iter())
            .chain(self.stop.iter())
            .map(String::as_str)
            .collect()
    }

    /// The rows a person reads: `start`, `ready`, `test` (one row a
    /// command, the label on the first), `stop`.
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        let mut rows = Vec::new();
        if let Some(s) = &self.start {
            rows.push(("start", s.clone()));
        }
        if let Some(s) = &self.ready {
            rows.push(("ready", s.clone()));
        }
        for (i, t) in self.test.iter().enumerate() {
            rows.push((if i == 0 { "test" } else { "" }, t.clone()));
        }
        if let Some(s) = &self.stop {
            rows.push(("stop", s.clone()));
        }
        rows
    }

    /// The file's text.
    pub fn text(&self) -> String {
        let mut s = String::from(
            "# How this project runs. Ryter's test hat starts it, waits for `ready` to\n\
             # answer, runs `test`, and stops it with `stop`. Change it here or ask the\n\
             # tester to propose a new one; Ryter asks you to approve it again.\n",
        );
        if let Some(v) = &self.start {
            s.push_str(&format!("start = {}\n", quoted(v)));
        }
        if let Some(v) = &self.ready {
            s.push_str(&format!("ready = {}\n", quoted(v)));
        }
        match self.test.as_slice() {
            [] => {}
            [one] => s.push_str(&format!("test  = [{}]\n", quoted(one))),
            many => {
                s.push_str("test  = [\n");
                for t in many {
                    s.push_str(&format!("    {},\n", quoted(t)));
                }
                s.push_str("]\n");
            }
        }
        if let Some(v) = &self.stop {
            s.push_str(&format!("stop  = {}\n", quoted(v)));
        }
        s
    }
}

/// What a project's run file comes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// There is none.
    None,
    /// It can't be read: why.
    Broken(String),
    /// The user approved exactly this.
    Approved(RunFile),
    /// It is there, and the user hasn't approved what it says now. With
    /// the text it was read from: a yes is to that text, and to no other
    /// the file may hold by the time the yes comes.
    Unapproved(RunFile, String),
}

fn digest(text: &str) -> String {
    let mut h = Sha256::new();
    h.update(text.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn approvals_path(home: &Path) -> PathBuf {
    home.join("run-approved.toml")
}

fn approvals(home: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(approvals_path(home))
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default()
}

/// The name a project's approval is kept under: its real path.
fn key(root: &Path) -> String {
    std::fs::canonicalize(root)
        .unwrap_or_else(|_| root.to_path_buf())
        .display()
        .to_string()
}

/// Read the project's run file, and whether what it says is approved.
pub fn find(root: &Path, home: &Path) -> Found {
    let text = match std::fs::read_to_string(root.join(FILE)) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Found::None,
        Err(e) => return Found::Broken(e.to_string()),
    };
    let run: RunFile = match toml::from_str(&text) {
        Ok(r) => r,
        Err(e) => return Found::Broken(e.message().to_string()),
    };
    let run = run.tidy();
    if approvals(home).get(&key(root)) == Some(&digest(&text)) {
        Found::Approved(run)
    } else {
        Found::Unapproved(run, text)
    }
}

/// Save `run` as the project's run file and record that the user approved
/// it. Returns the file.
pub fn save_approved(root: &Path, home: &Path, run: &RunFile) -> Result<PathBuf> {
    let dir = crate::plan::own_folder(root)?;
    let path = root.join(FILE);
    let io = |e: std::io::Error| Error::Io(format!("{}: {e}", path.display()));
    let text = run.text();
    // Written beside it and moved into place: a file there that is a link
    // is replaced, not written through to what it points at.
    let tmp = dir.join("run.toml.tmp");
    std::fs::write(&tmp, &text).map_err(io)?;
    std::fs::rename(&tmp, &path).map_err(io)?;
    approve(root, home, &text)?;
    Ok(path)
}

/// Record that the user approved a run file holding exactly `text`: what
/// [`find`] read and they were shown. The file is not read again, so one
/// rewritten while they were reading is not what they approved.
pub fn approve(root: &Path, home: &Path, text: &str) -> Result<()> {
    let mut all = approvals(home);
    all.insert(key(root), digest(text));
    let out = toml::to_string(&all).map_err(|e| Error::Io(e.to_string()))?;
    let path = approvals_path(home);
    std::fs::create_dir_all(home).map_err(|e| Error::Io(e.to_string()))?;
    std::fs::write(&path, out).map_err(|e| Error::Io(format!("{}: {e}", path.display())))
}

// ---------------------------------------------------------------------------
// The product, once started
// ---------------------------------------------------------------------------

/// A product Ryter started for a test, and how to stop it.
#[derive(Debug)]
pub struct Started {
    /// When, as the user's clock reads (`YYYY-MM-DD HH:MM`).
    pub at: String,
    /// Where it answers, if the run file says.
    pub address: Option<String>,
    /// The command that stops it, if the run file has one.
    pub stop: Option<String>,
    /// The start command, when it stayed in the foreground: Ryter holds it,
    /// and stops it by ending its process group.
    child: Option<std::process::Child>,
    /// The start command's process group, when the command itself returned
    /// and left something running in it (`./server &`). Ended on stop, by
    /// this process only: it is the group this process started.
    group: Option<u32>,
    /// Where the start command's output goes.
    pub log: PathBuf,
}

impl Started {
    /// A product an earlier session left running: only its stop command
    /// is Ryter's to use now.
    pub fn left(left: Left, log: PathBuf) -> Self {
        Self {
            at: left.at,
            address: left.address,
            stop: left.stop,
            child: None,
            group: None,
            log,
        }
    }

    /// The process group of a start command that is still running, or of
    /// what it left running.
    pub fn pgid(&self) -> Option<u32> {
        self.child
            .as_ref()
            .map(std::process::Child::id)
            .or(self.group)
    }

    /// What a later session needs to know, to offer to stop it.
    pub fn note(&self) -> Left {
        Left {
            at: self.at.clone(),
            address: self.address.clone(),
            stop: self.stop.clone(),
            pid: self.pgid(),
        }
    }
}

/// A product left running by a session that has ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Left {
    /// When it was started.
    pub at: String,
    /// Where it answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// The command that stops it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<String>,
    /// The start command's process, when it stayed in the foreground. A
    /// later session doesn't end it: a number is not proof the process is
    /// still the one that was started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
}

fn left_path(home: &Path, root: &Path) -> PathBuf {
    let key = key(root);
    // The folder's name to read by, and a digest of its whole path to tell
    // it from every other: `my-app` and `my_app` shared one note.
    let name: String = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    home.join("running")
        .join(format!("{name}-{}.toml", &digest(&key)[..16]))
}

/// Remember that this project's product was left running.
pub fn remember(home: &Path, root: &Path, left: &Left) -> Result<()> {
    let path = left_path(home, root);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::Io(e.to_string()))?;
    }
    let text = toml::to_string(left).map_err(|e| Error::Io(e.to_string()))?;
    std::fs::write(&path, text).map_err(|e| Error::Io(format!("{}: {e}", path.display())))
}

/// The product an earlier turn or session started here, if any.
pub fn remembered(home: &Path, root: &Path) -> Option<Left> {
    toml::from_str(&std::fs::read_to_string(left_path(home, root)).ok()?).ok()
}

/// Forget it: it was stopped.
pub fn forget(home: &Path, root: &Path) {
    let _ = std::fs::remove_file(left_path(home, root));
}

/// How asking `url` went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// It answered with this HTTP status.
    Status(u16),
    /// Something is listening (an `https` address, which isn't spoken to).
    Listening,
    /// Nothing there yet: why.
    Nothing(String),
}

impl Answer {
    /// Whether the product is up: an answer that isn't a server error.
    pub fn up(&self) -> bool {
        match self {
            Self::Status(s) => *s < 500,
            Self::Listening => true,
            Self::Nothing(_) => false,
        }
    }
}

/// `host`, `port` and `path` of an `http://` or `https://` address.
fn parts(url: &str) -> Option<(bool, String, u16, String)> {
    let (tls, rest) = match url.split_once("://") {
        Some(("http", r)) => (false, r),
        Some(("https", r)) => (true, r),
        Some(_) => return None,
        None => (false, url),
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (h, after) = v6.split_once(']')?;
        (h.to_string(), after.strip_prefix(':'))
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), Some(p)),
            None => (authority.to_string(), None),
        }
    };
    let port = match port {
        Some(p) => p.parse().ok()?,
        None if tls => 443,
        None => 80,
    };
    Some((tls, host, port, path.to_string()))
}

/// Ask `url` once whether the product is up.
pub fn ask(url: &str, timeout: Duration) -> Answer {
    use std::io::{Read, Write};
    use std::net::{TcpStream, ToSocketAddrs};
    let Some((tls, host, port, path)) = parts(url) else {
        return Answer::Nothing(format!("{url} is not an http address"));
    };
    let addrs = match (host.as_str(), port).to_socket_addrs() {
        Ok(a) => a.collect::<Vec<_>>(),
        Err(e) => return Answer::Nothing(e.to_string()),
    };
    let mut last = "no address".to_string();
    for addr in addrs {
        let mut stream = match TcpStream::connect_timeout(&addr, timeout) {
            Ok(s) => s,
            Err(e) => {
                last = e.to_string();
                continue;
            }
        };
        if tls {
            return Answer::Listening;
        }
        let _ = stream.set_read_timeout(Some(timeout));
        let _ = stream.set_write_timeout(Some(timeout));
        let req = format!(
            "GET {path} HTTP/1.0\r\nHost: {host}:{port}\r\nUser-Agent: ryter\r\nConnection: close\r\n\r\n"
        );
        if let Err(e) = stream.write_all(req.as_bytes()) {
            last = e.to_string();
            continue;
        }
        let mut head = [0u8; 64];
        let n = match stream.read(&mut head) {
            Ok(n) => n,
            Err(e) => {
                last = e.to_string();
                continue;
            }
        };
        // `HTTP/1.1 200 OK`
        let line = String::from_utf8_lossy(&head[..n]);
        return match line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse::<u16>().ok())
        {
            Some(status) if line.starts_with("HTTP/") => Answer::Status(status),
            _ => Answer::Nothing("it answered, but not in HTTP".into()),
        };
    }
    Answer::Nothing(last)
}

/// How starting the product went.
#[derive(Debug)]
pub enum Start {
    /// It is up.
    Up {
        /// What was started.
        started: Started,
        /// How it was known to be up, in words.
        how: String,
    },
    /// The start command failed, or the product never answered.
    Failed(String),
    /// The turn was cancelled; what was started was stopped.
    Cancelled,
}

fn tail(path: &Path, lines: usize) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// Start the product: run `start` in `root`, and wait until it is up.
///
/// A start command that returns (a stack brought up in the background) has
/// to succeed, and then `ready` has to answer. One that stays in the
/// foreground is left running, held by Ryter, once `ready` answers, or
/// after a few seconds when there is no address to ask.
pub fn start(
    run: &RunFile,
    root: &Path,
    log: &Path,
    timeout: Duration,
    cancel: &crate::cancel::Cancel,
) -> Result<Start> {
    start_settling(run, root, log, timeout, SETTLE, cancel)
}

/// [`start`], with how long a foreground start command with no address is
/// watched before the product is taken to be up.
fn start_settling(
    run: &RunFile,
    root: &Path,
    log: &Path,
    timeout: Duration,
    settle: Duration,
    cancel: &crate::cancel::Cancel,
) -> Result<Start> {
    let Some(cmd) = run.start.as_deref() else {
        return Ok(Start::Failed(
            "the run file has no start command".to_string(),
        ));
    };
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::Io(e.to_string()))?;
    }
    let out = std::fs::File::create(log).map_err(|e| Error::Io(e.to_string()))?;
    let err = out.try_clone().map_err(|e| Error::Io(e.to_string()))?;
    let mut command = crate::tools::shell::command(cmd, root);
    command.stdout(out).stderr(err);
    let mut child = command.spawn().map_err(|e| Error::Config(e.to_string()))?;
    let pgid = child.id();
    let began = Instant::now();
    let mut exited = false;
    let how = loop {
        if cancel.is_cancelled() {
            take_down(run, root, &mut child, exited);
            return Ok(Start::Cancelled);
        }
        if !exited {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => exited = true,
                Ok(Some(status)) => {
                    let how = status
                        .code()
                        .map_or("was killed".to_string(), |c| format!("exited {c}"));
                    return Ok(Start::Failed(format!("`{cmd}` {how}:\n{}", tail(log, 30))));
                }
                Ok(None) => {}
                Err(e) => return Err(Error::Config(e.to_string())),
            }
        }
        match run.ready.as_deref() {
            Some(url) => {
                let answer = ask(url, Duration::from_millis(800));
                if answer.up() {
                    break match answer {
                        Answer::Status(s) => format!("{url} answered {s}"),
                        _ => format!("{url} is listening"),
                    };
                }
                if began.elapsed() > timeout {
                    let why = match answer {
                        Answer::Status(s) => format!("answered {s}"),
                        Answer::Nothing(e) => format!("did not answer ({e})"),
                        Answer::Listening => "is listening".into(),
                    };
                    let undone = take_down(run, root, &mut child, exited);
                    return Ok(Start::Failed(format!(
                        "`{cmd}` ran, but after {}s {url} {why}. {undone}.\n{}",
                        timeout.as_secs(),
                        tail(log, 30)
                    )));
                }
            }
            // No address to ask: a command that returned has started it; one
            // still running after a moment is the product itself.
            None if exited => break "the start command finished".to_string(),
            None if began.elapsed() > settle => {
                break "the start command is still running, and there is no `ready` address \
                       to ask"
                    .to_string();
            }
            None => {}
        }
        std::thread::sleep(Duration::from_millis(250));
    };
    Ok(Start::Up {
        started: Started {
            at: crate::clock::stamp(),
            address: run.ready.clone(),
            stop: run.stop.clone(),
            // What a command that returned left running in its group
            // (`./server &`) is Ryter's to end too.
            group: (exited && crate::tools::shell::group_alive(pgid)).then_some(pgid),
            child: (!exited).then_some(child),
            log: log.to_path_buf(),
        },
        how,
    })
}

/// A start that didn't come up, or was cancelled, is taken down again: what
/// it started is nobody's otherwise. Nothing records it, so `/stop` would
/// say Ryter started nothing while a stack whose health check failed stayed
/// up. Returns what was done, in words.
fn take_down(run: &RunFile, root: &Path, child: &mut std::process::Child, exited: bool) -> String {
    let pgid = child.id();
    if !exited {
        crate::tools::shell::end_child(child);
        return "Ryter ended the start command".to_string();
    }
    let mut did = Vec::new();
    if let Some(stop) = run.stop.as_deref() {
        // Its own cancel: the turn's may already be set, and this has to run.
        let fresh = crate::cancel::Cancel::new();
        did.push(
            match crate::tools::shell::run_command_live(stop, root, COMMAND_TIMEOUT, &fresh, None) {
                Ok(crate::tools::shell::Run::Ok(_)) => {
                    format!("Ryter ran `{stop}` to take it down")
                }
                _ => format!("`{stop}` did not take it down: it may still be running"),
            },
        );
    }
    if crate::tools::shell::group_alive(pgid) {
        crate::tools::shell::end_group(pgid);
        did.push("Ryter ended what the start command left running".to_string());
    }
    if did.is_empty() {
        "The run file has no stop command, so what it started may still be running".to_string()
    } else {
        did.join(", and ")
    }
}

/// Stop a product Ryter started: its stop command, then the start command
/// itself if Ryter still holds it. Returns what was done, in words.
pub fn stop(
    mut started: Started,
    root: &Path,
    cancel: &crate::cancel::Cancel,
) -> std::result::Result<String, String> {
    let mut did = Vec::new();
    let mut failed = None;
    if let Some(cmd) = started.stop.clone() {
        match crate::tools::shell::run_command_live(
            cmd.as_str(),
            root,
            COMMAND_TIMEOUT,
            cancel,
            None,
        ) {
            Ok(crate::tools::shell::Run::Ok(_)) => did.push(cmd),
            Ok(crate::tools::shell::Run::Failed(out)) => {
                failed = Some(format!("`{cmd}` failed:\n{}", last_lines(&out, 20)));
            }
            Ok(crate::tools::shell::Run::TimedOut) => {
                failed = Some(format!("`{cmd}` did not finish"));
            }
            Ok(crate::tools::shell::Run::Cancelled) => failed = Some("cancelled".into()),
            Err(e) => failed = Some(e.to_string()),
        }
    }
    if let Some(mut child) = started.child.take() {
        crate::tools::shell::end_child(&mut child);
        did.push("ended the start command".to_string());
    } else if let Some(pgid) = started.group.take() {
        if crate::tools::shell::group_alive(pgid) {
            crate::tools::shell::end_group(pgid);
            did.push("ended what the start command left running".to_string());
        }
    }
    match failed {
        Some(why) => Err(why),
        None if did.is_empty() => Err(
            "the run file has no stop command, and the start command is not Ryter's to end"
                .to_string(),
        ),
        None => Ok(did.join(", then ")),
    }
}

/// The last `n` lines of `text`.
pub fn last_lines(text: &str, n: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(n)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn cms() -> RunFile {
        RunFile {
            start: Some("docker compose up -d --wait".into()),
            ready: Some("http://localhost:8000/healthz".into()),
            test: vec![
                "docker compose run --rm web pytest -q".into(),
                "docker compose run --rm web ruff check .".into(),
            ],
            stop: Some("docker compose down".into()),
        }
    }

    /// The file reads as it was approved, and reads back the same.
    #[test]
    fn the_run_file_is_written_as_it_is_read() {
        let text = cms().text();
        assert!(
            text.ends_with(
                "start = \"docker compose up -d --wait\"\n\
                 ready = \"http://localhost:8000/healthz\"\n\
                 test  = [\n    \"docker compose run --rm web pytest -q\",\n    \
                 \"docker compose run --rm web ruff check .\",\n]\n\
                 stop  = \"docker compose down\"\n"
            ),
            "{text}"
        );
        assert_eq!(toml::from_str::<RunFile>(&text).unwrap(), cms());
        // One test command can be a plain string; quotes survive.
        let one: RunFile = toml::from_str("test = \"pytest -k 'a and b'\"").unwrap();
        assert_eq!(one.test, ["pytest -k 'a and b'"]);
        let odd = RunFile {
            start: Some("sh -c \"echo \\\"hi\\\"\"".into()),
            ..RunFile::default()
        };
        assert_eq!(toml::from_str::<RunFile>(&odd.text()).unwrap(), odd);
        assert_eq!(
            cms().rows(),
            [
                ("start", "docker compose up -d --wait".to_string()),
                ("ready", "http://localhost:8000/healthz".to_string()),
                ("test", "docker compose run --rm web pytest -q".to_string()),
                ("", "docker compose run --rm web ruff check .".to_string()),
                ("stop", "docker compose down".to_string()),
            ]
        );
        let messy = RunFile {
            start: Some("  npm run\n dev  ".into()),
            test: vec!["".into(), " npm test ".into()],
            stop: Some("   ".into()),
            ready: None,
        }
        .tidy();
        assert_eq!(messy.start.as_deref(), Some("npm run dev"));
        assert_eq!(messy.test, ["npm test"]);
        assert_eq!(messy.stop, None);
        assert!(RunFile::default().is_empty() && !messy.is_empty());
    }

    /// A run file counts only as the user approved it. One that came with
    /// the project, or was changed since, is there but not approved, and
    /// the approval is kept outside the project.
    #[test]
    fn a_run_file_is_approved_by_what_it_says() {
        let home = TempDir::new().unwrap();
        let root = TempDir::new().unwrap();
        assert_eq!(find(root.path(), home.path()), Found::None);
        // It arrived with the project.
        std::fs::create_dir_all(root.path().join(".ryter")).unwrap();
        std::fs::write(root.path().join(FILE), "start = \"make up\"\n").unwrap();
        let theirs = RunFile {
            start: Some("make up".into()),
            ..RunFile::default()
        };
        let Found::Unapproved(run, text) = find(root.path(), home.path()) else {
            panic!("a run file nobody approved is approved");
        };
        assert_eq!(run, theirs);
        // Rewritten while the user was reading: their yes is to what they
        // read, so what is there now is still not approved.
        std::fs::write(root.path().join(FILE), "start = \"make pwned\"\n").unwrap();
        approve(root.path(), home.path(), &text).unwrap();
        assert!(matches!(
            find(root.path(), home.path()),
            Found::Unapproved(..)
        ));
        std::fs::write(root.path().join(FILE), &text).unwrap();
        assert_eq!(find(root.path(), home.path()), Found::Approved(theirs));
        // Saved from the panel.
        let path = save_approved(root.path(), home.path(), &cms()).unwrap();
        assert_eq!(path, root.path().join(".ryter/run.toml"));
        assert_eq!(find(root.path(), home.path()), Found::Approved(cms()));
        assert!(!root.path().join(".ryter/run-approved.toml").exists());
        // Changed afterwards, by anyone.
        let mut text = std::fs::read_to_string(&path).unwrap();
        text = text.replace("docker compose down", "docker compose down -v");
        std::fs::write(&path, &text).unwrap();
        let Found::Unapproved(now, _) = find(root.path(), home.path()) else {
            panic!("a changed run file is still approved");
        };
        assert_eq!(now.stop.as_deref(), Some("docker compose down -v"));
        // Another project's approval is its own.
        let other = TempDir::new().unwrap();
        std::fs::create_dir_all(other.path().join(".ryter")).unwrap();
        std::fs::write(other.path().join(FILE), cms().text()).unwrap();
        assert!(matches!(
            find(other.path(), home.path()),
            Found::Unapproved(..)
        ));
        // A run file that is a link is replaced, not written through.
        let elsewhere = TempDir::new().unwrap();
        let target = elsewhere.path().join("theirs.txt");
        std::fs::write(&target, "not Ryter's\n").unwrap();
        std::fs::remove_file(other.path().join(FILE)).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&target, other.path().join(FILE)).unwrap();
            save_approved(other.path(), home.path(), &cms()).unwrap();
            assert_eq!(std::fs::read_to_string(&target).unwrap(), "not Ryter's\n");
            assert_eq!(find(other.path(), home.path()), Found::Approved(cms()));
        }
        // One that can't be read says why.
        std::fs::write(&path, "start = [1, 2\n").unwrap();
        assert!(matches!(find(root.path(), home.path()), Found::Broken(_)));
    }

    #[test]
    fn an_address_is_read_as_host_port_and_path() {
        assert_eq!(
            parts("http://localhost:8000/healthz"),
            Some((false, "localhost".into(), 8000, "/healthz".into()))
        );
        assert_eq!(
            parts("localhost:3000"),
            Some((false, "localhost".into(), 3000, "/".into()))
        );
        assert_eq!(
            parts("https://cms.localhost/"),
            Some((true, "cms.localhost".into(), 443, "/".into()))
        );
        assert_eq!(
            parts("http://[::1]:8080/x?y=1"),
            Some((false, "::1".into(), 8080, "/x?y=1".into()))
        );
        assert_eq!(
            parts("http://127.0.0.1"),
            Some((false, "127.0.0.1".into(), 80, "/".into()))
        );
        assert_eq!(parts("ftp://localhost/"), None);
        assert_eq!(parts("http://user@localhost/"), None);
        assert_eq!(parts("http://localhost:port/"), None);
    }

    /// A server that answers `status`, on a port of its own, for `hits`
    /// requests.
    fn server(status: u16, hits: usize) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/healthz", listener.local_addr().unwrap());
        let t = std::thread::spawn(move || {
            for stream in listener.incoming().take(hits) {
                let mut s = stream.unwrap();
                let mut buf = [0u8; 512];
                let _ = s.read(&mut buf);
                let _ = s.write_all(
                    format!("HTTP/1.1 {status} X\r\nContent-Length: 0\r\n\r\n").as_bytes(),
                );
            }
        });
        (url, t)
    }

    #[test]
    fn asking_an_address_says_whether_the_product_is_up() {
        let (url, t) = server(200, 1);
        assert_eq!(ask(&url, Duration::from_secs(2)), Answer::Status(200));
        t.join().unwrap();
        let (url, t) = server(503, 1);
        let answer = ask(&url, Duration::from_secs(2));
        assert_eq!(answer, Answer::Status(503));
        assert!(!answer.up(), "a server error is not up");
        t.join().unwrap();
        assert!(
            Answer::Status(404).up(),
            "a page that isn't there still answered"
        );
        // Nothing listening: the port was just released.
        assert!(matches!(
            ask(&url, Duration::from_millis(300)),
            Answer::Nothing(_)
        ));
        assert!(matches!(
            ask("ftp://localhost/", Duration::from_millis(300)),
            Answer::Nothing(_)
        ));
    }

    /// A start command that returns has started the product; one that
    /// fails says so with its output; one that stays in the foreground is
    /// held, and ended when the product is stopped.
    #[cfg(unix)]
    #[test]
    fn the_product_is_started_and_stopped() {
        let root = TempDir::new().unwrap();
        let cancel = crate::cancel::Cancel::new();
        let log = root.path().join("log/project.log");
        // Returns, with a stop command.
        let run = RunFile {
            start: Some("echo up > state".into()),
            stop: Some("echo down > state".into()),
            ..RunFile::default()
        };
        let Start::Up { started, how } =
            start(&run, root.path(), &log, Duration::from_secs(20), &cancel).unwrap()
        else {
            panic!("did not start");
        };
        assert_eq!(how, "the start command finished");
        assert_eq!(started.pgid(), None);
        assert_eq!(
            std::fs::read_to_string(root.path().join("state")).unwrap(),
            "up\n"
        );
        assert_eq!(
            stop(started, root.path(), &cancel).unwrap(),
            "echo down > state"
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("state")).unwrap(),
            "down\n"
        );
        // Fails: its output is the reason.
        let run = RunFile {
            start: Some("echo no such service >&2; exit 3".into()),
            ..RunFile::default()
        };
        match start(&run, root.path(), &log, Duration::from_secs(20), &cancel).unwrap() {
            Start::Failed(why) => assert!(
                why.contains("exited 3") && why.contains("no such service"),
                "{why}"
            ),
            other => panic!("{other:?}"),
        }
        // Stays in the foreground, with no address: held, then ended.
        let run = RunFile {
            start: Some("echo serving; sleep 60".into()),
            ..RunFile::default()
        };
        let Start::Up { started, how } = start_settling(
            &run,
            root.path(),
            &log,
            Duration::from_secs(20),
            Duration::from_millis(400),
            &cancel,
        )
        .unwrap() else {
            panic!("did not start");
        };
        assert!(how.contains("still running"), "{how}");
        let pgid = started.pgid().expect("held");
        assert!(crate::tools::shell::group_alive(pgid));
        assert_eq!(started.note().pid, Some(pgid));
        assert_eq!(
            stop(started, root.path(), &cancel).unwrap(),
            "ended the start command"
        );
        assert!(!crate::tools::shell::group_alive(pgid));
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "serving\n");
        // Returns, having left a server running behind it, with no stop
        // command: what it left is Ryter's to end.
        let run = RunFile {
            start: Some("sleep 60 &".into()),
            ..RunFile::default()
        };
        let Start::Up { started, .. } =
            start(&run, root.path(), &log, Duration::from_secs(20), &cancel).unwrap()
        else {
            panic!("did not start");
        };
        let pgid = started.pgid().expect("what it left running is held");
        assert!(crate::tools::shell::group_alive(pgid));
        assert_eq!(
            stop(started, root.path(), &cancel).unwrap(),
            "ended what the start command left running"
        );
        assert!(!crate::tools::shell::group_alive(pgid));
        // Nothing to start.
        assert!(matches!(
            start(
                &RunFile::default(),
                root.path(),
                &log,
                Duration::from_secs(1),
                &cancel
            )
            .unwrap(),
            Start::Failed(_)
        ));
    }

    /// With an address, the product is up when it answers, and a start
    /// that never does is a failure with the reason.
    #[cfg(unix)]
    #[test]
    fn the_product_is_up_when_its_address_answers() {
        let root = TempDir::new().unwrap();
        let cancel = crate::cancel::Cancel::new();
        let log = root.path().join("project.log");
        let (url, t) = server(200, 1);
        let run = RunFile {
            start: Some("true".into()),
            ready: Some(url.clone()),
            ..RunFile::default()
        };
        let Start::Up { started, how } =
            start(&run, root.path(), &log, Duration::from_secs(20), &cancel).unwrap()
        else {
            panic!("did not start");
        };
        assert_eq!(how, format!("{url} answered 200"));
        assert_eq!(started.address.as_deref(), Some(url.as_str()));
        t.join().unwrap();
        // Nobody answers there now.
        match start(&run, root.path(), &log, Duration::from_millis(600), &cancel).unwrap() {
            Start::Failed(why) => assert!(why.contains("did not answer"), "{why}"),
            other => panic!("{other:?}"),
        }
        // A start that returned but never came up is taken down again: a
        // stack whose health check fails is not left up with nothing to
        // stop it by.
        let run = RunFile {
            start: Some("echo up > state".into()),
            ready: Some(url.clone()),
            stop: Some("echo down > state".into()),
            ..RunFile::default()
        };
        match start(&run, root.path(), &log, Duration::from_millis(600), &cancel).unwrap() {
            Start::Failed(why) => assert!(
                why.contains("Ryter ran `echo down > state` to take it down"),
                "{why}"
            ),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            std::fs::read_to_string(root.path().join("state")).unwrap(),
            "down\n"
        );
        // Cancelled during the wait: the same.
        std::fs::remove_file(root.path().join("state")).unwrap();
        let stopped = crate::cancel::Cancel::new();
        let flag = stopped.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            flag.cancel();
        });
        assert!(matches!(
            start(&run, root.path(), &log, Duration::from_secs(20), &stopped).unwrap(),
            Start::Cancelled
        ));
        t.join().unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("state")).unwrap(),
            "down\n"
        );
    }

    #[test]
    fn a_product_left_running_is_remembered_until_it_is_stopped() {
        let home = TempDir::new().unwrap();
        let root = TempDir::new().unwrap();
        assert_eq!(remembered(home.path(), root.path()), None);
        let left = Left {
            at: "2026-10-01 14:02".into(),
            address: Some("http://localhost:8000".into()),
            stop: Some("docker compose down".into()),
            pid: None,
        };
        remember(home.path(), root.path(), &left).unwrap();
        assert_eq!(remembered(home.path(), root.path()), Some(left.clone()));
        let other = TempDir::new().unwrap();
        assert_eq!(remembered(home.path(), other.path()), None);
        // Two folders whose names differ only in punctuation are two
        // projects.
        let parent = TempDir::new().unwrap();
        let (dash, under) = (parent.path().join("my-app"), parent.path().join("my_app"));
        std::fs::create_dir_all(&dash).unwrap();
        std::fs::create_dir_all(&under).unwrap();
        remember(home.path(), &dash, &left).unwrap();
        assert!(remembered(home.path(), &dash).is_some());
        assert_eq!(remembered(home.path(), &under), None);
        forget(home.path(), root.path());
        assert_eq!(remembered(home.path(), root.path()), None);
    }
}
