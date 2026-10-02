//! On-disk session: meta, events, transcript, spend.

use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{Error, Result};
use crate::event::AgentEvent;
use crate::ids::SessionId;
use crate::llm::Message;
use crate::role::{Role, Thread};
use crate::spend::Usage;

/// What a build turn left, for an undo of only its files.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TurnRecord {
    /// A snapshot of the files when the turn ended.
    #[serde(default)]
    pub after: Option<String>,
    /// Gitignored files the turn wrote, which snapshots skip.
    #[serde(default)]
    pub ignored: Vec<SavedFile>,
}

/// One file's content before and after, as saved git objects (`None`: no
/// file).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedFile {
    /// Path in the project.
    pub path: String,
    /// Content before.
    pub before: Option<String>,
    /// Content after.
    pub after: Option<String>,
}

/// An undo, kept so `/redo` can reverse it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Redo {
    /// The checkpoint the undo popped, and its record.
    pub checkpoint: String,
    /// Its record.
    pub record: TurnRecord,
    /// The files just before the undo.
    pub files: String,
    /// The files just after it.
    pub undone: String,
    /// What the undo put back.
    pub paths: Vec<String>,
    /// Ignored files the undo put back: `before` is what the undo replaced.
    pub ignored: Vec<SavedFile>,
}

/// Session index row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    /// Session id.
    pub id: SessionId,
    /// Working directory.
    pub cwd: PathBuf,
    /// RFC3339-ish unix millis as string for simplicity.
    pub created_at: String,
    /// Last write.
    pub updated_at: String,
    /// Active connection name.
    pub connection: String,
    /// Active model id.
    pub model: String,
    /// Display title.
    pub title: String,
    /// Sum of known USD. `None` if nothing priced yet.
    pub spend_usd_total: Option<f64>,
    /// True if any turn had unknown rates.
    #[serde(default)]
    pub spend_unknown: bool,
    /// At least one request ended without a complete accounting record.
    #[serde(default)]
    pub spend_incomplete: bool,
    /// The model whose last call had no price. With a budget set, it is not
    /// called again until it has one: the budget could not see it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unpriced_model: Option<String>,
    /// The plan the user last approved, as a path in the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_file: Option<String>,
    /// Build-hat checkpoints, oldest first, for `/undo`.
    #[serde(default)]
    pub checkpoints: Vec<String>,
    /// What each build turn changed, by the checkpoint it started from, so
    /// `/undo` puts back that turn's files and nothing else.
    #[serde(default)]
    pub turn_records: std::collections::BTreeMap<String, TurnRecord>,
    /// Undos `/redo` can reverse, newest last. A new build turn clears them.
    #[serde(default)]
    pub redo: Vec<Redo>,
    /// The files as the latest build turn found them: what `/changes` calls
    /// "last turn". Not moved by a file undone from `/changes`.
    #[serde(default)]
    pub turn_checkpoint: Option<String>,
    /// The hat the user left the session in. `None`, or a role from crew
    /// mode (sessions from before it was removed), means build.
    #[serde(default)]
    pub mode: Option<crate::role::Role>,
}

/// One priced model call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendRecord {
    /// Unix millis.
    pub ts: String,
    /// Connection name.
    pub connection: String,
    /// Model id.
    pub model: String,
    /// The hat it was spent in (`crew` for a row from crew mode).
    pub role: Role,
    /// Prompt tokens.
    pub input_tokens: u64,
    /// Completion tokens.
    pub output_tokens: u64,
    /// Cached tokens.
    pub cached_tokens: u64,
    /// USD, or omitted when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_usd: Option<f64>,
    /// Counts and any estimated cost are a lower bound after interruption.
    #[serde(default)]
    pub incomplete: bool,
}

/// Open session on disk.
#[derive(Debug)]
pub struct Session {
    /// Directory containing the JSONL files.
    pub dir: PathBuf,
    /// Index.
    pub meta: Meta,
    /// Messages sent to the model: the thread in use.
    pub transcript: Vec<Message>,
    /// Which thread that is.
    thread: Thread,
    /// The other thread, while it isn't in use.
    parked: Vec<Message>,
    /// How many build turns in this process have gone on to change files.
    /// "Did that turn change anything?" was asked of the list of
    /// checkpoints, which stops growing at fifty: past that, no fix was
    /// seen as a change and no review of it was offered.
    pub changed_turns: u64,
    /// Recovery notices from opening this session, including backup paths.
    pub recovery_notices: Vec<String>,
}

/// The file a thread's messages are kept in.
fn thread_file(thread: Thread) -> &'static str {
    match thread {
        Thread::Main => "transcript.jsonl",
        Thread::Test => "test.jsonl",
    }
}

impl Session {
    /// Create a new session under `home/sessions/<slug>/<id>/`.
    pub fn create(home: &Path, cwd: &Path, connection: String, model: String) -> Result<Self> {
        let id = SessionId::generate();
        let dir = home.join("sessions").join(cwd_slug(cwd)).join(id.as_str());
        fs::create_dir_all(dir.join("notes")).map_err(|e| Error::Io(e.to_string()))?;
        let now = now_stamp();
        let meta = Meta {
            id: id.clone(),
            cwd: cwd.to_path_buf(),
            created_at: now.clone(),
            updated_at: now,
            connection,
            model,
            title: String::new(),
            spend_usd_total: None,
            spend_unknown: false,
            spend_incomplete: false,
            unpriced_model: None,
            plan_file: None,
            checkpoints: Vec::new(),
            turn_records: Default::default(),
            redo: Vec::new(),
            turn_checkpoint: None,
            mode: None,
        };
        let s = Self {
            dir,
            meta,
            transcript: Vec::new(),
            thread: Thread::Main,
            parked: Vec::new(),
            changed_turns: 0,
            recovery_notices: Vec::new(),
        };
        s.write_meta()?;
        File::create(s.dir.join("events.jsonl")).map_err(|e| Error::Io(e.to_string()))?;
        File::create(s.dir.join("transcript.jsonl")).map_err(|e| Error::Io(e.to_string()))?;
        File::create(s.dir.join("spend.jsonl")).map_err(|e| Error::Io(e.to_string()))?;
        crate::trace::log(
            home,
            &format!("session {} model={}", s.meta.id, s.meta.model),
        );
        Ok(s)
    }

    /// Open an existing session directory.
    pub fn open(dir: &Path) -> Result<Self> {
        let _lock = lock_logs(dir)?;
        let text =
            fs::read_to_string(dir.join("meta.json")).map_err(|e| Error::Io(e.to_string()))?;
        let meta: Meta = serde_json::from_str(&text).map_err(|e| Error::Io(e.to_string()))?;
        let old_meta = serde_json::to_vec(&meta).map_err(|e| Error::Io(e.to_string()))?;
        // Inspect every log before changing any of them. Only a final,
        // unterminated EOF record is eligible for automatic recovery.
        let main_path = dir.join(thread_file(Thread::Main));
        let test_path = dir.join(thread_file(Thread::Test));
        let spend_path = dir.join("spend.jsonl");
        let main = inspect_jsonl::<Message>(&main_path, true)?;
        let test = inspect_jsonl::<Message>(&test_path, true)?;
        let spend = inspect_jsonl::<SpendRecord>(&spend_path, true)?;
        let repairs = [
            (&main_path, main.torn_at),
            (&test_path, test.torn_at),
            (&spend_path, spend.torn_at),
        ];
        let mut session = Self {
            dir: dir.to_path_buf(),
            meta,
            transcript: main.rows,
            thread: Thread::Main,
            parked: test.rows,
            changed_turns: 0,
            recovery_notices: Vec::new(),
        };
        session.reconcile_spend(&spend.rows);
        if repairs.iter().any(|(_, offset)| offset.is_some()) {
            // Persist uncertainty BEFORE discarding an incomplete record. A
            // crash during recovery must not make the next resume trust it.
            session.meta.spend_unknown = true;
            session.meta.spend_incomplete = true;
        }
        if old_meta != serde_json::to_vec(&session.meta).map_err(|e| Error::Io(e.to_string()))? {
            session.write_meta()?;
        }
        for (path, offset) in repairs {
            if let Some(offset) = offset {
                let backup = recover_prefix(path, offset)?;
                session.recovery_notices.push(format!(
                    "Recovered {} through its last complete record; original saved at {}. Spending may be incomplete.",
                    path.file_name().unwrap_or_default().to_string_lossy(), backup.display()
                ));
            }
        }
        Ok(session)
    }

    /// The append-only ledger can be ahead of cached metadata after a crash.
    /// Never reduce an older cached total: legacy writers updated meta first.
    fn reconcile_spend(&mut self, rows: &[SpendRecord]) {
        let cached = self.meta.spend_usd_total;
        let mut total: Option<f64> = None;
        for row in rows {
            self.meta.spend_unknown |= row.incomplete || row.total_usd.is_none();
            self.meta.spend_incomplete |= row.incomplete;
            match row.total_usd {
                Some(usd) if usd.is_finite() && usd >= 0.0 => {
                    total = Some(total.unwrap_or(0.0) + usd);
                    if self.meta.unpriced_model.as_deref() == Some(row.model.as_str()) {
                        self.meta.unpriced_model = None;
                    }
                }
                _ => {
                    self.meta.spend_unknown = true;
                    self.meta.unpriced_model = Some(row.model.clone());
                    if row.total_usd.is_some() {
                        self.meta.spend_incomplete = true;
                    }
                }
            }
        }
        if cached.is_some_and(|v| !v.is_finite() || v < 0.0 || v > total.unwrap_or(0.0) + 1e-9) {
            if !self.meta.spend_incomplete {
                self.recovery_notices.push("Cached spending exceeds the readable ledger; kept the higher total and marked accounting incomplete.".into());
            }
            self.meta.spend_unknown = true;
            self.meta.spend_incomplete = true;
        }
        self.meta.spend_usd_total = match (cached, total) {
            (Some(old), Some(sum)) => Some(old.max(sum)),
            (old, sum) => old.or(sum),
        };
        if cached != self.meta.spend_usd_total {
            self.recovery_notices
                .push("Recovered spending totals from the session ledger.".into());
        }
    }

    /// Most recently updated session for `cwd`, if any.
    pub fn latest(home: &Path, cwd: &Path) -> Result<Option<Self>> {
        let root = home.join("sessions").join(cwd_slug(cwd));
        let Ok(rd) = fs::read_dir(&root) else {
            return Ok(None);
        };
        let mut best: Option<(String, PathBuf)> = None;
        for ent in rd.flatten() {
            let meta_path = ent.path().join("meta.json");
            let Ok(text) = fs::read_to_string(&meta_path) else {
                continue;
            };
            let Ok(meta) = serde_json::from_str::<Meta>(&text) else {
                continue;
            };
            let newer = best
                .as_ref()
                .map(|(t, _)| meta.updated_at.as_str() > t.as_str())
                .unwrap_or(true);
            if newer {
                best = Some((meta.updated_at, ent.path()));
            }
        }
        match best {
            Some((_, p)) => Ok(Some(Self::open(&p)?)),
            None => Ok(None),
        }
    }

    /// Sessions for `cwd`, newest `updated_at` first.
    pub fn list(home: &Path, cwd: &Path) -> Result<Vec<SessionInfo>> {
        list_in(&home.join("sessions").join(cwd_slug(cwd)))
    }

    /// Open by full id or unique prefix. Prefers `cwd`, then any slug under `home/sessions`.
    pub fn find(home: &Path, cwd: Option<&Path>, query: &str) -> Result<Self> {
        let q = query.trim();
        if q.is_empty() {
            return Err(Error::Config("session id is empty".into()));
        }
        if let Some(cwd) = cwd {
            let listed = Self::list(home, cwd)?;
            if let Some(hit) = match_info(&listed, q) {
                return Self::open(&hit.dir);
            }
        }
        let root = home.join("sessions");
        let Ok(rd) = fs::read_dir(&root) else {
            return Err(Error::Config(format!("session {q} not found")));
        };
        let mut found: Vec<SessionInfo> = Vec::new();
        for slug in rd.flatten() {
            let listed = list_in(&slug.path())?;
            found.extend(match_all(&listed, q).into_iter().cloned());
        }
        match found.len() {
            0 => Err(Error::Config(format!("session {q} not found"))),
            1 => Self::open(&found[0].dir),
            _ => Err(Error::Config(format!("session id {q} is ambiguous"))),
        }
    }

    /// Set the display title (trimmed, capped).
    pub fn set_title(&mut self, title: &str) -> Result<()> {
        let t: String = title.trim().chars().take(120).collect();
        self.meta.title = t;
        self.touch()
    }

    /// Delete a session directory. Requires `meta.json` so we never `rm -rf` a random path.
    pub fn remove(dir: &Path) -> Result<()> {
        if !dir.join("meta.json").is_file() {
            return Err(Error::Config(format!(
                "not a session directory: {}",
                dir.display()
            )));
        }
        fs::remove_dir_all(dir).map_err(|e| Error::Io(e.to_string()))?;
        // Its pages go with it: `<home>/sessions/<project>/<id>` →
        // `<home>/pages/<id>`.
        let sessions = dir.parent().and_then(Path::parent);
        if let (Some(sessions), Some(id)) = (sessions, dir.file_name()) {
            if sessions.file_name().is_some_and(|n| n == "sessions") {
                if let Some(home) = sessions.parent() {
                    let _ = fs::remove_dir_all(crate::page::dir(home, &id.to_string_lossy()));
                }
            }
        }
        Ok(())
    }

    /// Append an event and flush.
    pub fn emit(&mut self, ev: &AgentEvent) -> Result<()> {
        append_jsonl(&self.dir.join("events.jsonl"), ev)?;
        self.touch()?;
        Ok(())
    }

    /// The thread `transcript` is.
    pub fn thread(&self) -> Thread {
        self.thread
    }

    /// Make `thread` the one in use: `transcript` is its messages from here
    /// on, and new messages are added to it. The other thread is kept as it
    /// is.
    pub fn use_thread(&mut self, thread: Thread) {
        if thread != self.thread {
            std::mem::swap(&mut self.transcript, &mut self.parked);
            self.thread = thread;
        }
    }

    /// The messages of `thread`, whichever is in use.
    pub fn messages_of(&self, thread: Thread) -> &[Message] {
        if thread == self.thread {
            &self.transcript
        } else {
            &self.parked
        }
    }

    /// Append a message to the thread in use.
    pub fn push_message(&mut self, msg: Message) -> Result<()> {
        self.push_to(self.thread, msg)
    }

    /// Append a message to `thread`, whichever is in use: how the tester's
    /// report reaches the conversation the other hats share.
    pub fn push_to(&mut self, thread: Thread, msg: Message) -> Result<()> {
        append_jsonl(&self.dir.join(thread_file(thread)), &msg)?;
        if thread == self.thread {
            self.transcript.push(msg);
        } else {
            self.parked.push(msg);
        }
        self.touch()?;
        Ok(())
    }

    /// Answer every tool call the transcript left unanswered, so providers
    /// accept it again. A turn stopped between an assistant's tool calls and
    /// their results (Esc during a command, a crash, an older Ryter) left a
    /// transcript every provider rejects, and every later message in the
    /// session failed. Returns how many calls were answered here.
    pub fn repair_unanswered(&mut self) -> Result<usize> {
        let (fixed, n) = answer_unanswered(&self.transcript);
        if n > 0 {
            self.replace_transcript(fixed)?;
        }
        Ok(n)
    }

    /// Rewrite the file of the thread in use, after compaction. Events log
    /// is unchanged.
    pub fn replace_transcript(&mut self, messages: Vec<Message>) -> Result<()> {
        let _lock = lock_logs(&self.dir)?;
        let name = thread_file(self.thread);
        let path = self.dir.join(name);
        atomic_write(&path, |f| {
            for m in &messages {
                let mut line = serde_json::to_string(m).map_err(|e| Error::Io(e.to_string()))?;
                line.push('\n');
                f.write_all(line.as_bytes())
                    .map_err(|e| Error::Io(e.to_string()))?;
            }
            Ok(())
        })?;
        self.transcript = messages;
        self.touch()?;
        Ok(())
    }

    /// Record a priced (or unpriced) model call.
    pub fn record_spend(&mut self, rec: SpendRecord) -> Result<()> {
        append_jsonl(&self.dir.join("spend.jsonl"), &rec)?;
        self.count_spend(&rec)
    }

    /// Add a row to the session's totals.
    fn count_spend(&mut self, rec: &SpendRecord) -> Result<()> {
        if rec.incomplete {
            self.meta.spend_unknown = true;
            self.meta.spend_incomplete = true;
        }
        match rec.total_usd {
            Some(v) => {
                self.meta.spend_usd_total = Some(self.meta.spend_usd_total.unwrap_or(0.0) + v);
                if self.meta.unpriced_model.as_deref() == Some(rec.model.as_str()) {
                    self.meta.unpriced_model = None;
                }
            }
            None => {
                self.meta.spend_unknown = true;
                self.meta.unpriced_model = Some(rec.model.clone());
            }
        }
        self.write_meta()
    }

    /// Where spend rows are appended.
    pub fn spend_path(&self) -> PathBuf {
        self.dir.join("spend.jsonl")
    }

    /// All spend rows.
    pub fn spend_log(&self) -> Result<Vec<SpendRecord>> {
        read_jsonl(&self.dir.join("spend.jsonl"))
    }

    /// The session's notes folder, which the plan hat may write in.
    pub fn notes_dir(&self) -> PathBuf {
        self.dir.join("notes")
    }

    /// Update connection + model on the session index.
    pub fn set_route(&mut self, connection: String, model: String) -> Result<()> {
        self.meta.connection = connection;
        self.meta.model = model;
        self.touch()
    }

    /// Remember the mode for resume.
    pub fn set_mode(&mut self, mode: crate::role::Role) -> Result<()> {
        self.meta.mode = Some(mode);
        self.touch()
    }

    /// Record a build-hat checkpoint (keeps the last 50).
    pub fn push_checkpoint(&mut self, sha: String) -> Result<()> {
        self.meta.checkpoints.push(sha);
        let n = self.meta.checkpoints.len();
        if n > 50 {
            self.meta.checkpoints.drain(..n - 50);
        }
        self.touch()
    }

    /// Remember where the latest build turn started.
    pub fn set_turn_checkpoint(&mut self, sha: Option<String>) -> Result<()> {
        self.meta.turn_checkpoint = sha;
        self.touch()
    }

    /// Record what the turn from checkpoint `sha` changed.
    pub fn set_turn_record(&mut self, sha: &str, record: TurnRecord) -> Result<()> {
        self.meta.turn_records.insert(sha.to_string(), record);
        // Only live checkpoints need a record.
        let live: std::collections::HashSet<&String> = self.meta.checkpoints.iter().collect();
        self.meta
            .turn_records
            .retain(|k, _| live.contains(k) || k == sha);
        self.touch()
    }

    /// Keep an undo for `/redo`.
    pub fn push_redo(&mut self, redo: Redo) -> Result<()> {
        self.meta.redo.push(redo);
        let n = self.meta.redo.len();
        if n > 20 {
            self.meta.redo.drain(..n - 20);
        }
        self.touch()
    }

    /// The newest undo, removed.
    pub fn pop_redo(&mut self) -> Result<Option<Redo>> {
        let r = self.meta.redo.pop();
        self.touch()?;
        Ok(r)
    }

    /// Forget what `/redo` could reverse: the files moved on.
    pub fn clear_redo(&mut self) -> Result<()> {
        if self.meta.redo.is_empty() {
            return Ok(());
        }
        self.meta.redo.clear();
        self.touch()
    }

    /// Drop the newest checkpoint.
    pub fn pop_checkpoint(&mut self) -> Result<Option<String>> {
        let c = self.meta.checkpoints.pop();
        self.touch()?;
        Ok(c)
    }

    /// Remember the plan the user approved.
    pub fn set_plan_file(&mut self, path: Option<String>) -> Result<()> {
        self.meta.plan_file = path;
        self.touch()
    }

    fn touch(&mut self) -> Result<()> {
        self.meta.updated_at = now_stamp();
        self.write_meta()
    }

    fn write_meta(&self) -> Result<()> {
        let path = self.dir.join("meta.json");
        let body = serde_json::to_vec_pretty(&self.meta).map_err(|e| Error::Io(e.to_string()))?;
        atomic_write(&path, |f| {
            f.write_all(&body).map_err(|e| Error::Io(e.to_string()))
        })
    }
}

/// One row for `/resume` and `ryter sessions`.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    /// Session directory.
    pub dir: PathBuf,
    /// Index.
    pub meta: Meta,
    /// Title, or first user line, or `untitled`.
    pub preview: String,
    /// Transcript length (user + assistant + tool rows).
    pub messages: usize,
}

impl SessionInfo {
    /// Short id (first 8 hex-ish chars).
    pub fn short_id(&self) -> String {
        self.meta.id.as_str().chars().take(8).collect()
    }

    /// `updated_at` as unix milliseconds, if parseable.
    pub fn updated_millis(&self) -> Option<u64> {
        self.meta.updated_at.trim().parse().ok()
    }
}

fn list_in(root: &Path) -> Result<Vec<SessionInfo>> {
    let Ok(rd) = fs::read_dir(root) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for ent in rd.flatten() {
        let dir = ent.path();
        if !dir.join("meta.json").is_file() {
            continue;
        }
        let Ok(s) = Session::open(&dir) else {
            continue;
        };
        let preview = if !s.meta.title.trim().is_empty() {
            s.meta.title.clone()
        } else {
            s.transcript
                .iter()
                .find(|m| m.role == "user")
                .map(|m| m.content.chars().take(60).collect())
                .filter(|t: &String| !t.trim().is_empty())
                .unwrap_or_else(|| "untitled".into())
        };
        out.push(SessionInfo {
            dir,
            messages: s.transcript.len(),
            meta: s.meta,
            preview,
        });
    }
    out.sort_by(|a, b| b.meta.updated_at.cmp(&a.meta.updated_at));
    Ok(out)
}

fn match_info<'a>(listed: &'a [SessionInfo], q: &str) -> Option<&'a SessionInfo> {
    let hits = match_all(listed, q);
    if hits.len() == 1 { Some(hits[0]) } else { None }
}

fn match_all<'a>(listed: &'a [SessionInfo], q: &str) -> Vec<&'a SessionInfo> {
    let exact: Vec<_> = listed.iter().filter(|s| s.meta.id.as_str() == q).collect();
    if !exact.is_empty() {
        return exact;
    }
    listed
        .iter()
        .filter(|s| s.meta.id.as_str().starts_with(q) || s.short_id() == q)
        .collect()
}

/// Build a spend row from usage + optional USD.
pub fn spend_record(
    connection: String,
    model: String,
    role: Role,
    usage: Usage,
    total_usd: Option<f64>,
) -> SpendRecord {
    SpendRecord {
        ts: now_stamp(),
        connection,
        model,
        role,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cached_tokens: usage.cached_tokens,
        total_usd,
        incomplete: false,
    }
}

/// What stands in for a result that never came.
pub const UNANSWERED: &str = "not run: the turn was stopped before this call ran";

/// `messages` with a stand-in result after every tool call that has none.
fn answer_unanswered(messages: &[Message]) -> (Vec<Message>, usize) {
    let mut out = Vec::with_capacity(messages.len());
    let mut added = 0;
    let mut i = 0;
    while i < messages.len() {
        let m = &messages[i];
        out.push(m.clone());
        i += 1;
        let Some(calls) = m.tool_calls.as_ref().filter(|_| m.role == "assistant") else {
            continue;
        };
        let mut answered = std::collections::HashSet::new();
        while i < messages.len() && messages[i].role == "tool" {
            if let Some(id) = &messages[i].tool_call_id {
                answered.insert(id.clone());
            }
            out.push(messages[i].clone());
            i += 1;
        }
        for c in calls.iter().filter(|c| !answered.contains(&c.id)) {
            out.push(Message {
                role: "tool".into(),
                content: UNANSWERED.into(),
                tool_call_id: Some(c.id.clone()),
                tool_calls: None,
            });
            added += 1;
        }
    }
    (out, added)
}

pub(crate) fn append_jsonl<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let _lock = lock_logs(path.parent().unwrap_or_else(|| Path::new(".")))?;
    let mut f = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)
        .map_err(|e| Error::Io(e.to_string()))?;
    // A complete final JSON value without a newline is valid. Separate it
    // from the next append instead of concatenating two JSON objects.
    if f.metadata().map_err(|e| Error::Io(e.to_string()))?.len() > 0 {
        f.seek(SeekFrom::End(-1))
            .map_err(|e| Error::Io(e.to_string()))?;
        let mut last = [0];
        f.read_exact(&mut last)
            .map_err(|e| Error::Io(e.to_string()))?;
        if last[0] != b'\n' {
            f.write_all(b"\n").map_err(|e| Error::Io(e.to_string()))?;
        }
    }
    let mut line = serde_json::to_string(value).map_err(|e| Error::Io(e.to_string()))?;
    line.push('\n');
    f.write_all(line.as_bytes())
        .map_err(|e| Error::Io(e.to_string()))?;
    f.flush().map_err(|e| Error::Io(e.to_string()))?;
    f.sync_all().map_err(|e| Error::Io(e.to_string()))?;
    Ok(())
}

struct JsonLog<T> {
    rows: Vec<T>,
    torn_at: Option<u64>,
}

fn read_jsonl<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Vec<T>> {
    let _lock = lock_logs(path.parent().unwrap_or_else(|| Path::new(".")))?;
    Ok(inspect_jsonl(path, false)?.rows)
}

fn inspect_jsonl<T: for<'de> Deserialize<'de>>(path: &Path, recover: bool) -> Result<JsonLog<T>> {
    let f = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(JsonLog {
                rows: Vec::new(),
                torn_at: None,
            });
        }
        Err(e) => return Err(Error::Io(e.to_string())),
    };
    let mut reader = BufReader::new(f);
    let mut out = JsonLog {
        rows: Vec::new(),
        torn_at: None,
    };
    let mut offset = 0;
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = reader
            .read_until(b'\n', &mut line)
            .map_err(|e| Error::Io(e.to_string()))?;
        if n == 0 {
            break;
        }
        if !line.iter().all(u8::is_ascii_whitespace) {
            match serde_json::from_slice(&line) {
                Ok(row) => out.rows.push(row),
                Err(e) => {
                    let split_utf8 = std::str::from_utf8(&line).err().is_some_and(|e| {
                        e.error_len().is_none()
                            && serde_json::from_slice::<serde_json::Value>(&line[..e.valid_up_to()])
                                .is_err_and(|error| error.is_eof())
                    });
                    if recover && !line.ends_with(b"\n") && (e.is_eof() || split_utf8) {
                        out.torn_at = Some(offset);
                        break;
                    }
                    return Err(Error::Io(format!(
                        "{} at byte {offset}: {e}; original left unchanged",
                        path.display()
                    )));
                }
            }
        }
        offset += n as u64;
    }
    Ok(out)
}

/// Readers that may repair a tail and writers agree on one directory lock.
/// Advisory locking leaves stale lock files harmless after a process exits.
fn lock_logs(dir: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(dir.join(".jsonl.lock"))
        .map_err(|e| Error::Io(e.to_string()))?;
    #[cfg(unix)]
    rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive)
        .map_err(|e| Error::Io(e.to_string()))?;
    Ok(file)
}

fn sync_parent(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path.parent().unwrap_or_else(|| Path::new(".")))
        .and_then(|d| d.sync_all())
        .map_err(|e| Error::Io(e.to_string()))?;
    Ok(())
}

fn create_private(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|e| Error::Io(e.to_string()))
}

/// Unique staging files prevent concurrent writes from sharing a temporary
/// pathname. The replacement and its directory entry are synced before return.
fn atomic_write(path: &Path, write: impl FnOnce(&mut File) -> Result<()>) -> Result<()> {
    let tmp = path.with_extension(format!("tmp-{}", SessionId::generate()));
    let result = (|| {
        let mut f = create_private(&tmp)?;
        write(&mut f)?;
        f.sync_all().map_err(|e| Error::Io(e.to_string()))?;
        fs::rename(&tmp, path).map_err(|e| Error::Io(e.to_string()))?;
        sync_parent(path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn recover_prefix(path: &Path, length: u64) -> Result<PathBuf> {
    let backup = path.with_extension(format!("jsonl.recovery-{}.bak", SessionId::generate()));
    let mut original = File::open(path).map_err(|e| Error::Io(e.to_string()))?;
    let mut saved = create_private(&backup)?;
    std::io::copy(&mut original, &mut saved).map_err(|e| Error::Io(e.to_string()))?;
    saved.sync_all().map_err(|e| Error::Io(e.to_string()))?;
    sync_parent(&backup)?;
    original.rewind().map_err(|e| Error::Io(e.to_string()))?;
    atomic_write(path, |f| {
        std::io::copy(&mut original.take(length), f).map_err(|e| Error::Io(e.to_string()))?;
        Ok(())
    })?;
    Ok(backup)
}

fn now_stamp() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    ms.to_string()
}

pub fn cwd_slug(cwd: &Path) -> String {
    let raw = cwd.to_string_lossy();
    let mut enc = String::new();
    for b in raw.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => enc.push(*b as char),
            _ => enc.push_str(&format!("%{b:02X}")),
        }
    }
    if enc.len() <= 200 {
        enc
    } else {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        raw.hash(&mut h);
        format!("p-{:016x}", h.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[cfg(unix)]
    #[test]
    fn recovery_waits_for_a_live_append_to_finish() {
        use std::sync::mpsc;
        use std::time::Duration;
        let home = TempDir::new().unwrap();
        let session = Session::create(home.path(), home.path(), "c".into(), "m".into()).unwrap();
        let path = session.dir.join("transcript.jsonl");
        let lock = lock_logs(&session.dir).unwrap();
        let bytes = serde_json::to_vec(&msg("user", "complete append")).unwrap();
        let cut = bytes.len() / 2;
        fs::write(&path, &bytes[..cut]).unwrap();
        let dir = session.dir.clone();
        let (started, start_rx) = mpsc::channel();
        let (done, done_rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            started.send(()).unwrap();
            done.send(Session::open(&dir)).unwrap();
        });
        start_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let blocked = matches!(
            done_rx.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );
        let mut writer = OpenOptions::new().append(true).open(&path).unwrap();
        writer.write_all(&bytes[cut..]).unwrap();
        writer.write_all(b"\n").unwrap();
        writer.sync_all().unwrap();
        drop(lock);
        let resumed = done_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        reader.join().unwrap();
        assert!(blocked);
        assert_eq!(said(&resumed.transcript), ["complete append"]);
        assert!(resumed.recovery_notices.is_empty());
    }

    #[test]
    fn torn_thread_tails_keep_history_backup_and_allow_future_appends() {
        for thread in [Thread::Main, Thread::Test] {
            for tail in [
                b"{\"role\":\"assistant\",\"content\":\"unfinished".as_slice(),
                b"{\"role\":\"assistant\",\"content\":\"\xe2\x82".as_slice(),
            ] {
                let home = TempDir::new().unwrap();
                let mut session =
                    Session::create(home.path(), home.path(), "c".into(), "m".into()).unwrap();
                session
                    .push_to(thread, msg("user", "keep this constraint"))
                    .unwrap();
                let path = session.dir.join(thread_file(thread));
                let prefix = fs::read(&path).unwrap();
                OpenOptions::new()
                    .append(true)
                    .open(&path)
                    .unwrap()
                    .write_all(tail)
                    .unwrap();
                let original = fs::read(&path).unwrap();
                let mut resumed = Session::open(&session.dir).unwrap();
                assert_eq!(said(resumed.messages_of(thread)), ["keep this constraint"]);
                assert_eq!(fs::read(&path).unwrap(), prefix);
                assert_eq!(resumed.recovery_notices.len(), 1);
                assert!(resumed.meta.spend_incomplete);
                let backup = fs::read_dir(&session.dir)
                    .unwrap()
                    .flatten()
                    .map(|e| e.path())
                    .find(|p| p.extension().is_some_and(|ext| ext == "bak"))
                    .unwrap();
                assert_eq!(fs::read(backup).unwrap(), original);
                resumed
                    .push_to(thread, msg("assistant", "continue"))
                    .unwrap();
                let again = Session::open(&session.dir).unwrap();
                assert_eq!(
                    said(again.messages_of(thread)),
                    ["keep this constraint", "continue"]
                );
                assert!(again.recovery_notices.is_empty());
                assert!(again.meta.spend_incomplete);
            }
        }
    }

    #[test]
    fn malformed_complete_or_middle_records_are_left_unchanged() {
        for tail in [
            "{\"role\":\"assistant\"\n",
            "not json",
            "{\"role\":false}",
            "{\"role\":\"assistant\"\n{}\n",
        ] {
            let home = TempDir::new().unwrap();
            let session =
                Session::create(home.path(), home.path(), "c".into(), "m".into()).unwrap();
            let path = session.dir.join("transcript.jsonl");
            fs::write(&path, tail).unwrap();
            assert!(Session::open(&session.dir).is_err(), "{tail}");
            assert_eq!(fs::read_to_string(path).unwrap(), tail);
            assert!(
                !fs::read_dir(&session.dir)
                    .unwrap()
                    .flatten()
                    .any(|e| e.path().extension().is_some_and(|x| x == "bak"))
            );
        }
    }

    #[test]
    fn complete_unterminated_record_is_separated_from_next_append() {
        let home = TempDir::new().unwrap();
        let session = Session::create(home.path(), home.path(), "c".into(), "m".into()).unwrap();
        fs::write(
            session.dir.join("transcript.jsonl"),
            serde_json::to_vec(&msg("user", "first")).unwrap(),
        )
        .unwrap();
        let mut resumed = Session::open(&session.dir).unwrap();
        assert!(resumed.recovery_notices.is_empty());
        resumed.push_message(msg("assistant", "second")).unwrap();
        assert_eq!(
            said(&Session::open(&session.dir).unwrap().transcript),
            ["first", "second"]
        );
    }

    #[test]
    fn spend_ledger_recovers_stale_meta_and_torn_cost_remains_unknown() {
        let home = TempDir::new().unwrap();
        let mut session =
            Session::create(home.path(), home.path(), "c".into(), "m".into()).unwrap();
        let record = |cost| {
            spend_record(
                "c".into(),
                "m".into(),
                Role::SoloBuild,
                Usage::default(),
                cost,
            )
        };
        session.record_spend(record(Some(0.25))).unwrap();
        // Emulate a durable ledger append followed by a crash before meta.
        append_jsonl(&session.spend_path(), &record(Some(0.75))).unwrap();
        let resumed = Session::open(&session.dir).unwrap();
        assert_eq!(resumed.meta.spend_usd_total, Some(1.0));
        assert!(!resumed.meta.spend_incomplete);
        assert_eq!(resumed.recovery_notices.len(), 1);
        assert_eq!(
            Session::open(&session.dir).unwrap().meta.spend_usd_total,
            Some(1.0)
        );
        OpenOptions::new()
            .append(true)
            .open(session.spend_path())
            .unwrap()
            .write_all(b"{\"total_usd\":")
            .unwrap();
        let resumed = Session::open(&session.dir).unwrap();
        assert_eq!(resumed.spend_log().unwrap().len(), 2);
        assert_eq!(resumed.meta.spend_usd_total, Some(1.0));
        assert!(resumed.meta.spend_unknown && resumed.meta.spend_incomplete);
        assert!(Session::open(&session.dir).unwrap().meta.spend_incomplete);
    }

    #[test]
    fn recovery_never_reduces_a_legacy_cached_spend_total() {
        let home = TempDir::new().unwrap();
        let mut session =
            Session::create(home.path(), home.path(), "c".into(), "m".into()).unwrap();
        session.meta.spend_usd_total = Some(2.0);
        session.write_meta().unwrap();
        let resumed = Session::open(&session.dir).unwrap();
        assert_eq!(resumed.meta.spend_usd_total, Some(2.0));
        assert!(resumed.meta.spend_unknown && resumed.meta.spend_incomplete);
        assert_eq!(resumed.recovery_notices.len(), 1);
    }

    /// A session saved in crew mode has fields and roles that are gone.
    /// It still opens, with its conversation and its spend, in the build
    /// hat.
    #[test]
    fn a_session_from_crew_mode_still_opens() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let s = Session::create(home.path(), cwd.path(), "c".into(), "m".into()).unwrap();
        let dir = s.dir.clone();
        drop(s);
        let meta = serde_json::json!({
            "id": "01a0f7ef-234b-718a-a02b-53d2d8f7884f",
            "cwd": cwd.path(),
            "created_at": "1790000000000",
            "updated_at": "1790000000001",
            "phase": "build",
            "connection": "openrouter",
            "model": "deepseek/deepseek-v4.1-flash",
            "title": "a docker stack",
            "spend_usd_total": 4.35,
            "auditor_enabled": true,
            "patch": {"branch": "ryter/patch-x-1", "worktree": "/w", "target": "main",
                      "base": "abc", "tasks": ["T-scaffold"]},
            "patches_opened": 1,
            "checks_offered": false,
            "rejections": {"T-scaffold": 7},
            "mode": "orchestrator"
        });
        fs::write(dir.join("meta.json"), meta.to_string()).unwrap();
        fs::write(
            dir.join("spend.jsonl"),
            "{\"ts\":\"1\",\"connection\":\"openrouter\",\"model\":\"z-ai/glm-5.3\",\"role\":\"builder\",\"input_tokens\":10,\"output_tokens\":5,\"cached_tokens\":0,\"total_usd\":0.5}\n\
             {\"ts\":\"2\",\"connection\":\"openrouter\",\"model\":\"x-ai/grok-4.7\",\"role\":\"auditor\",\"input_tokens\":10,\"output_tokens\":5,\"cached_tokens\":0,\"total_usd\":0.25}\n",
        )
        .unwrap();
        let s = Session::open(&dir).unwrap();
        assert_eq!(s.meta.title, "a docker stack");
        assert_eq!(s.meta.mode, Some(Role::Crew));
        assert_eq!(s.meta.mode.map(Role::hat), Some(Role::SoloBuild));
        let spend = s.spend_log().unwrap();
        assert_eq!(spend.len(), 2);
        assert!(spend.iter().all(|r| r.role == Role::Crew));
        assert_eq!(spend[0].total_usd, Some(0.5));
    }

    #[test]
    fn unanswered_calls_get_a_stand_in_result_where_providers_expect_it() {
        let call = |id: &str| crate::llm::AssistantToolCall {
            id: id.into(),
            name: "bash".into(),
            arguments: "{}".into(),
        };
        let msg = |role: &str,
                   content: &str,
                   id: Option<&str>,
                   calls: Option<Vec<crate::llm::AssistantToolCall>>| Message {
            role: role.into(),
            content: content.into(),
            tool_call_id: id.map(str::to_string),
            tool_calls: calls,
        };
        let t = vec![
            msg("user", "go", None, None),
            msg("assistant", "", None, Some(vec![call("a"), call("b")])),
            msg("tool", "ok", Some("a"), None),
            msg("user", "hello?", None, None),
        ];
        let (fixed, n) = answer_unanswered(&t);
        assert_eq!(n, 1);
        let shape: Vec<(String, Option<String>)> = fixed
            .iter()
            .map(|m| (m.role.clone(), m.tool_call_id.clone()))
            .collect();
        assert_eq!(
            shape,
            vec![
                ("user".into(), None),
                ("assistant".into(), None),
                ("tool".into(), Some("a".into())),
                ("tool".into(), Some("b".into())),
                ("user".into(), None),
            ]
        );
        assert_eq!(answer_unanswered(&fixed).1, 0);
    }

    fn msg(role: &str, content: &str) -> Message {
        Message {
            role: role.into(),
            content: content.into(),
            tool_call_id: None,
            tool_calls: None,
        }
    }

    fn said(messages: &[Message]) -> Vec<&str> {
        messages.iter().map(|m| m.content.as_str()).collect()
    }

    /// The tester's thread is kept apart from the conversation the other
    /// hats share: each has its own messages and its own file, a message
    /// can be put in either from the other, and both come back when the
    /// session is opened again.
    #[test]
    fn the_test_thread_is_kept_apart_from_the_main_one() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let mut s = Session::create(home.path(), cwd.path(), "c".into(), "m".into()).unwrap();
        assert_eq!(s.thread(), Thread::Main);
        s.push_message(msg("user", "build the list")).unwrap();
        s.use_thread(Thread::Test);
        assert!(s.transcript.is_empty());
        s.push_message(msg("user", "test the list")).unwrap();
        s.push_message(msg("assistant", "it fails")).unwrap();
        // The report, put in the main thread while the test one is in use.
        s.push_to(Thread::Main, msg("user", "the test's report"))
            .unwrap();
        assert_eq!(said(&s.transcript), ["test the list", "it fails"]);
        assert_eq!(
            said(s.messages_of(Thread::Main)),
            ["build the list", "the test's report"]
        );
        // Compacting one thread leaves the other alone.
        s.replace_transcript(vec![msg("user", "test, compacted")])
            .unwrap();
        s.use_thread(Thread::Main);
        s.use_thread(Thread::Main);
        assert_eq!(said(&s.transcript), ["build the list", "the test's report"]);
        assert_eq!(said(s.messages_of(Thread::Test)), ["test, compacted"]);
        let dir = s.dir.clone();
        drop(s);
        let again = Session::open(&dir).unwrap();
        assert_eq!(again.thread(), Thread::Main);
        assert_eq!(
            said(&again.transcript),
            ["build the list", "the test's report"]
        );
        assert_eq!(said(again.messages_of(Thread::Test)), ["test, compacted"]);
        // A session from before the test hat has no file for it.
        std::fs::remove_file(dir.join("test.jsonl")).unwrap();
        let old = Session::open(&dir).unwrap();
        assert!(old.messages_of(Thread::Test).is_empty());
        assert_eq!(old.transcript.len(), 2);
    }

    #[test]
    fn create_resume_and_spend() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let mut s = Session::create(
            home.path(),
            cwd.path(),
            "spacexai".into(),
            "grok-4.6".into(),
        )
        .unwrap();
        s.push_message(Message {
            role: "user".into(),
            content: "hi".into(),
            tool_call_id: None,
            tool_calls: None,
        })
        .unwrap();
        s.record_spend(spend_record(
            "spacexai".into(),
            "grok-4.6".into(),
            Role::SoloBuild,
            Usage {
                input_tokens: 10,
                output_tokens: 5,
                cached_tokens: 0,
                cache_write_tokens: 0,
            },
            Some(0.01),
        ))
        .unwrap();
        let dir = s.dir.clone();
        drop(s);
        let s2 = Session::open(&dir).unwrap();
        assert_eq!(s2.transcript.len(), 1);
        assert_eq!(s2.meta.spend_usd_total, Some(0.01));
        let latest = Session::latest(home.path(), cwd.path()).unwrap().unwrap();
        assert_eq!(latest.meta.id, s2.meta.id);
        let listed = Session::list(home.path(), cwd.path()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].preview, "hi");
        let mut s2 = s2;
        s2.set_title("my chat").unwrap();
        let listed = Session::list(home.path(), cwd.path()).unwrap();
        assert_eq!(listed[0].preview, "my chat");
        let found =
            Session::find(home.path(), Some(cwd.path()), listed[0].short_id().as_str()).unwrap();
        assert_eq!(found.meta.id, s2.meta.id);
        let dir = s2.dir.clone();
        drop(s2);
        Session::remove(&dir).unwrap();
        assert!(Session::list(home.path(), cwd.path()).unwrap().is_empty());
    }
}
