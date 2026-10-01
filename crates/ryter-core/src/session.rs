//! On-disk session: meta, events, transcript, spend.

use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{Error, Result};
use crate::event::AgentEvent;
use crate::ids::SessionId;
use crate::llm::Message;
use crate::role::Role;
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
}

/// Open session on disk.
#[derive(Debug)]
pub struct Session {
    /// Directory containing the JSONL files.
    pub dir: PathBuf,
    /// Index.
    pub meta: Meta,
    /// Messages sent to the model.
    pub transcript: Vec<Message>,
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
        let text =
            fs::read_to_string(dir.join("meta.json")).map_err(|e| Error::Io(e.to_string()))?;
        let meta: Meta = serde_json::from_str(&text).map_err(|e| Error::Io(e.to_string()))?;
        let transcript = read_jsonl::<Message>(&dir.join("transcript.jsonl"))?;
        Ok(Self {
            dir: dir.to_path_buf(),
            meta,
            transcript,
        })
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

    /// Append a transcript message.
    pub fn push_message(&mut self, msg: Message) -> Result<()> {
        append_jsonl(&self.dir.join("transcript.jsonl"), &msg)?;
        self.transcript.push(msg);
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

    /// Rewrite `transcript.jsonl` after compaction. Events log is unchanged.
    pub fn replace_transcript(&mut self, messages: Vec<Message>) -> Result<()> {
        let path = self.dir.join("transcript.jsonl");
        let tmp = self.dir.join("transcript.jsonl.tmp");
        let mut f = File::create(&tmp).map_err(|e| Error::Io(e.to_string()))?;
        for m in &messages {
            let mut line = serde_json::to_string(m).map_err(|e| Error::Io(e.to_string()))?;
            line.push('\n');
            f.write_all(line.as_bytes())
                .map_err(|e| Error::Io(e.to_string()))?;
        }
        f.flush().map_err(|e| Error::Io(e.to_string()))?;
        f.sync_all().map_err(|e| Error::Io(e.to_string()))?;
        fs::rename(tmp, path).map_err(|e| Error::Io(e.to_string()))?;
        self.transcript = messages;
        self.touch()?;
        Ok(())
    }

    /// Record a priced (or unpriced) model call.
    pub fn record_spend(&mut self, rec: SpendRecord) -> Result<()> {
        self.count_spend(&rec)?;
        append_jsonl(&self.dir.join("spend.jsonl"), &rec)
    }

    /// Add a row to the session's totals.
    fn count_spend(&mut self, rec: &SpendRecord) -> Result<()> {
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
        let tmp = self.dir.join("meta.json.tmp");
        let body = serde_json::to_vec_pretty(&self.meta).map_err(|e| Error::Io(e.to_string()))?;
        fs::write(&tmp, body).map_err(|e| Error::Io(e.to_string()))?;
        fs::rename(tmp, path).map_err(|e| Error::Io(e.to_string()))
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
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| Error::Io(e.to_string()))?;
    let mut line = serde_json::to_string(value).map_err(|e| Error::Io(e.to_string()))?;
    line.push('\n');
    f.write_all(line.as_bytes())
        .map_err(|e| Error::Io(e.to_string()))?;
    f.flush().map_err(|e| Error::Io(e.to_string()))?;
    f.sync_all().map_err(|e| Error::Io(e.to_string()))?;
    Ok(())
}

fn read_jsonl<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Vec<T>> {
    let f = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::Io(e.to_string())),
    };
    let mut out = Vec::new();
    for line in BufReader::new(f).lines() {
        let line = line.map_err(|e| Error::Io(e.to_string()))?;
        if line.trim().is_empty() {
            continue;
        }
        out.push(serde_json::from_str(&line).map_err(|e| Error::Io(e.to_string()))?);
    }
    Ok(out)
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
