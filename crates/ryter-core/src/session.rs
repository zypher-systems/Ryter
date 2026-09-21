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
use crate::phase::Phase;
use crate::role::Role;
use crate::spend::Usage;

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
    /// Orchestrator phase.
    pub phase: Phase,
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
    /// Auditor gate for this session.
    #[serde(default = "default_auditor_on")]
    pub auditor_enabled: bool,
    /// The open patch, if the crew is building one.
    #[serde(default)]
    pub patch: Option<Patch>,
    /// Patches opened so far (names the next branch).
    #[serde(default)]
    pub patches_opened: u32,
    /// Build-hat checkpoints, oldest first, for `/undo`.
    #[serde(default)]
    pub checkpoints: Vec<String>,
}

/// Several tasks' work collected on one integration branch. It lands on the
/// user's branch as a single commit, and only once every task in it is done,
/// so the user has nothing to act on until the whole change is in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Patch {
    /// Integration branch (`ryter/patch-<session>-<n>`).
    pub branch: String,
    /// Worktree where the integration branch is checked out.
    pub worktree: PathBuf,
    /// The user's branch it will land on.
    pub target: String,
    /// `target`'s commit when the patch opened.
    pub base: String,
    /// Tasks taken into this patch.
    #[serde(default)]
    pub tasks: Vec<String>,
    /// Tasks whose work is on the integration branch.
    #[serde(default)]
    pub landed: Vec<String>,
    /// Their titles, for the landing commit message.
    #[serde(default)]
    pub titles: Vec<String>,
}

fn default_auditor_on() -> bool {
    true
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
    /// Who spent it.
    pub role: Role,
    /// Child id when a specialist spent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_id: Option<String>,
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
    pub fn create(
        home: &Path,
        cwd: &Path,
        phase: Phase,
        connection: String,
        model: String,
    ) -> Result<Self> {
        let id = SessionId::generate();
        let dir = home.join("sessions").join(cwd_slug(cwd)).join(id.as_str());
        fs::create_dir_all(dir.join("notes")).map_err(|e| Error::Io(e.to_string()))?;
        let now = now_stamp();
        let meta = Meta {
            id: id.clone(),
            cwd: cwd.to_path_buf(),
            created_at: now.clone(),
            updated_at: now,
            phase,
            connection,
            model,
            title: String::new(),
            spend_usd_total: None,
            spend_unknown: false,
            auditor_enabled: true,
            patch: None,
            patches_opened: 0,
            checkpoints: Vec::new(),
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
            &format!("session {} phase={phase} model={}", s.meta.id, s.meta.model),
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
        fs::remove_dir_all(dir).map_err(|e| Error::Io(e.to_string()))
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

    /// Add a row to the totals only: the crew meter has already written it
    /// to `spend.jsonl` the moment it was charged.
    pub fn count_spend(&mut self, rec: &SpendRecord) -> Result<()> {
        match rec.total_usd {
            Some(v) => {
                self.meta.spend_usd_total = Some(self.meta.spend_usd_total.unwrap_or(0.0) + v);
            }
            None => self.meta.spend_unknown = true,
        }
        self.write_meta()
    }

    /// Where spend rows are appended.
    pub fn spend_path(&self) -> PathBuf {
        self.dir.join("spend.jsonl")
    }

    /// Keep a crew report the lead never saw, because the run stopped before
    /// it could take another round. The next turn hands it over.
    pub fn set_carry(&self, report: &str) -> Result<()> {
        fs::write(self.dir.join("carry.md"), report).map_err(|e| Error::Io(e.to_string()))
    }

    /// The report kept by [`set_carry`](Self::set_carry), removed as it is read.
    pub fn take_carry(&self) -> Option<String> {
        let path = self.dir.join("carry.md");
        let text = fs::read_to_string(&path).ok()?;
        let _ = fs::remove_file(&path);
        (!text.trim().is_empty()).then_some(text)
    }

    /// All spend rows.
    pub fn spend_log(&self) -> Result<Vec<SpendRecord>> {
        read_jsonl(&self.dir.join("spend.jsonl"))
    }

    /// Notes directory (pass notes).
    pub fn notes_dir(&self) -> PathBuf {
        self.dir.join("notes")
    }

    /// Path of the pass note for a phase.
    pub fn note_path(&self, phase: Phase) -> PathBuf {
        self.dir
            .join("notes")
            .join(format!("{}.md", phase.as_str()))
    }

    /// Read a pass note. Missing or empty file → empty string (allowed).
    pub fn read_note(&self, phase: Phase) -> Result<String> {
        match fs::read_to_string(self.note_path(phase)) {
            Ok(s) => Ok(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(Error::Io(e.to_string())),
        }
    }

    /// Write a pass note (empty body is allowed).
    /// Latest crew results, kept outside the repository so the orchestrator
    /// sees them on later turns. Only the tail is kept.
    pub fn write_crew_report(&self, body: &str) -> Result<()> {
        // Appended, not overwritten: a turn can drain the crew more than once,
        // and the last drain's report alone hid what the earlier ones did.
        const KEEP: usize = 32_000;
        let mut all = self.read_crew_report();
        if !all.is_empty() {
            all.push_str("\n---\n\n");
        }
        all.push_str(body);
        let start = all.len().saturating_sub(KEEP);
        let start = (start..=all.len())
            .find(|i| all.is_char_boundary(*i))
            .unwrap_or(all.len());
        fs::create_dir_all(self.notes_dir()).map_err(|e| Error::Io(e.to_string()))?;
        fs::write(self.notes_dir().join("crew.md"), &all[start..])
            .map_err(|e| Error::Io(e.to_string()))
    }

    /// Latest crew results, or empty.
    pub fn read_crew_report(&self) -> String {
        fs::read_to_string(self.notes_dir().join("crew.md")).unwrap_or_default()
    }

    pub fn write_note(&self, phase: Phase, body: &str) -> Result<()> {
        fs::create_dir_all(self.dir.join("notes")).map_err(|e| Error::Io(e.to_string()))?;
        fs::write(self.note_path(phase), body).map_err(|e| Error::Io(e.to_string()))
    }

    /// Record a pass note for the current phase and switch to `to`.
    /// Does not clear the orchestrator transcript.
    pub fn handoff(&mut self, to: Phase, note: &str, back_reason: Option<&str>) -> Result<()> {
        let from = self.meta.phase;
        let mut body = note.to_string();
        if let Some(reason) = back_reason {
            if !body.trim().is_empty() {
                body.push_str("\n\n");
            }
            body.push_str("Reason: ");
            body.push_str(reason);
            body.push('\n');
        }
        self.write_note(from, &body)?;
        self.meta.phase = to;
        self.touch()?;
        Ok(())
    }

    /// Update connection + model on the session index.
    pub fn set_route(&mut self, connection: String, model: String) -> Result<()> {
        self.meta.connection = connection;
        self.meta.model = model;
        self.touch()
    }

    /// Enable or disable the auditor gate.
    /// Replace the open patch.
    pub fn set_patch(&mut self, patch: Option<Patch>) -> Result<()> {
        self.meta.patch = patch;
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

    /// Drop the newest checkpoint.
    pub fn pop_checkpoint(&mut self) -> Result<Option<String>> {
        let c = self.meta.checkpoints.pop();
        self.touch()?;
        Ok(c)
    }

    pub fn set_auditor(&mut self, on: bool) -> Result<()> {
        self.meta.auditor_enabled = on;
        self.touch()
    }

    /// Previous phase for `/handoff back`.
    pub fn previous_phase(phase: Phase) -> Option<Phase> {
        match phase {
            Phase::Plan => None,
            Phase::Build => Some(Phase::Plan),
            Phase::Audit => Some(Phase::Build),
        }
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
        subagent_id: None,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cached_tokens: usage.cached_tokens,
        total_usd,
    }
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

pub(crate) fn cwd_slug(cwd: &Path) -> String {
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
    #[test]
    fn sessions_from_before_the_role_merge_still_load() {
        use crate::phase::Phase;
        use crate::role::Role;
        let phase: Phase = serde_json::from_str("\"architect\"").unwrap();
        assert_eq!(phase, Phase::Plan);
        let role: Role = serde_json::from_str("\"planner\"").unwrap();
        assert_eq!(role, Role::Architect);
    }

    use super::*;
    use tempfile::TempDir;

    #[test]
    fn create_resume_and_spend() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let mut s = Session::create(
            home.path(),
            cwd.path(),
            Phase::Build,
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
            Role::Orchestrator,
            Usage {
                input_tokens: 10,
                output_tokens: 5,
                cached_tokens: 0,
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

    #[test]
    fn handoff_preserves_transcript_and_allows_empty_note() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let mut s = Session::create(
            home.path(),
            cwd.path(),
            Phase::Plan,
            "spacexai".into(),
            "grok-4.6".into(),
        )
        .unwrap();
        s.push_message(Message {
            role: "user".into(),
            content: "keep me".into(),
            tool_call_id: None,
            tool_calls: None,
        })
        .unwrap();
        assert!(s.read_note(Phase::Plan).unwrap().is_empty());
        s.handoff(Phase::Build, "ship it", None).unwrap();
        assert_eq!(s.meta.phase, Phase::Build);
        assert_eq!(s.transcript.len(), 1);
        assert_eq!(s.transcript[0].content, "keep me");
        assert_eq!(s.read_note(Phase::Plan).unwrap(), "ship it");
        assert_eq!(s.transcript.len(), 1);
    }
}
