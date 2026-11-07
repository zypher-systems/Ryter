//! Deterministic transcript compaction and `/context` accounting.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::llm::Message;

/// How full the model window is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextReport {
    /// Tokens in use: the provider's last `input_tokens` when known, else the
    /// bytes/4 estimate used for admission and compaction.
    pub tokens: u64,
    /// Model window used for the percentage.
    pub window: u64,
    /// `tokens / window` as 0–100 (capped).
    pub pct: u8,
    /// Transcript messages.
    pub messages: usize,
    /// True when this report follows a compact pass.
    pub compacted: bool,
    /// True when [`Self::tokens`] is the provider's reported `input_tokens`.
    pub from_provider: bool,
}

impl ContextReport {
    /// One-line `/context` listing.
    pub fn render(&self) -> String {
        let flag = if self.compacted { "  compacted" } else { "" };
        let approx = if self.from_provider { "" } else { "~" };
        format!(
            "context  {}%  {approx}{}/{} tok  {} messages{flag}",
            self.pct, self.tokens, self.window, self.messages
        )
    }
}

/// Context window of a model Ryter knows; unknown is 200k.
pub fn window_for(model: &str) -> u64 {
    known_window(model).unwrap_or(ASSUMED_WINDOW)
}

/// The window taken for a model nothing says anything about.
pub const ASSUMED_WINDOW: u64 = 200_000;

/// Ryter's own row for `model`; `None` for an id it does not know by name.
pub fn known_window(model: &str) -> Option<u64> {
    Some(match model {
        "grok-4.7" | "grok-4.6" | "grok-4.6-latest" | "grok-4.5" | "grok-4.5-latest" => 500_000,
        "grok-4.3" | "grok-4.3-latest" => 1_000_000,
        m if m.starts_with("gpt-6") || m.starts_with("gpt-5.6") => 1_050_000,
        // Claude 4.6 and later; Haiku 4.5 and older keep 200k.
        m if crate::llm::claude_version(m).is_some_and(|v| v >= (4, 6)) => 1_000_000,
        m if crate::llm::claude_version(m).is_some() => 200_000,
        _ => return None,
    })
}

/// Compact token count (`0`, `12k`, `500k`, `1.2M`).
pub fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        if n % 1_000_000 == 0 {
            format!("{}M", n / 1_000_000)
        } else {
            format!("{:.1}M", n as f64 / 1_000_000.0)
        }
    } else if n >= 1000 {
        if n % 1000 == 0 {
            format!("{}k", n / 1000)
        } else {
            format!("{:.1}k", n as f64 / 1000.0)
        }
    } else {
        n.to_string()
    }
}

/// Sidebar context: `0/500k`.
pub fn format_context_used(used: u64, window: u64) -> String {
    format!("{}/{}", format_tokens(used), format_tokens(window))
}

/// Auto-compact when estimated tokens reach this fraction of the window.
pub const AUTO_COMPACT_PCT: u8 = 85;

/// Keep this many trailing user turns (and everything after the cut).
pub const KEEP_USER_TURNS: usize = 4;

/// Cheap token estimate: UTF-8 bytes / 4, matching common harness heuristics.
pub fn estimate_tokens(system: &str, messages: &[Message]) -> u64 {
    let mut n = system.len() + reasoning_bytes(messages);
    for m in messages {
        n += m.role.len();
        if let Some(id) = &m.tool_call_id {
            n += id.len();
        }
        // When Messages echo blocks are present they are what goes on the wire
        // (thinking + text + tool_use); counting content/tool_calls too would
        // double the assistant turn.
        if let Some(blocks) = &m.messages_blocks {
            for b in blocks {
                n += match b {
                    crate::llm::MessagesBlock::Thinking {
                        thinking,
                        signature,
                    } => thinking.len() + signature.len(),
                    crate::llm::MessagesBlock::RedactedThinking { data } => data.len(),
                    crate::llm::MessagesBlock::Text { text } => text.len(),
                    crate::llm::MessagesBlock::ToolUse { id, name, input } => {
                        id.len() + name.len() + input.to_string().len()
                    }
                };
            }
        } else {
            n += m.content.len();
            if let Some(calls) = &m.tool_calls {
                for c in calls {
                    n += c.name.len() + c.arguments.len() + c.id.len();
                }
            }
        }
    }
    u64::try_from(n).unwrap_or(u64::MAX).div_ceil(4)
}

/// The reasoning a request hands back: the running tool loop's, as
/// `chat_body` sends it. Past turns' is in the transcript and not on the wire.
fn reasoning_bytes(messages: &[Message]) -> usize {
    let loop_starts = messages
        .iter()
        .rposition(|m| m.role == "user")
        .map_or(0, |at| at + 1);
    messages[loop_starts..]
        .iter()
        .filter_map(|m| m.reasoning_details.as_ref())
        .flat_map(|echo| &echo.details)
        .map(|detail| detail.to_string().len())
        .sum()
}

/// Per-contributor token estimates for the `/context` inspector.
///
/// Rows: `system prompt`, `user messages`, `assistant text`, `tool calls`,
/// `tool output`. Zero rows are omitted. Same bytes/4 heuristic as
/// [`estimate_tokens`].
pub fn breakdown(system: &str, messages: &[Message]) -> Vec<(String, u64)> {
    let mut sys = system.len();
    let mut user = 0usize;
    let mut assistant = 0usize;
    let mut calls = 0usize;
    let mut tool_out = 0usize;
    for m in messages {
        match m.role.as_str() {
            "user" => user += m.role.len() + m.content.len(),
            "assistant" => assistant += m.role.len() + m.content.len(),
            "tool" => {
                tool_out += m.role.len() + m.content.len();
                if let Some(id) = &m.tool_call_id {
                    tool_out += id.len();
                }
            }
            _ => sys += m.role.len() + m.content.len(),
        }
        if let Some(cs) = &m.tool_calls {
            for c in cs {
                calls += c.name.len() + c.arguments.len() + c.id.len();
            }
        }
    }
    let tok = |n: usize| u64::try_from(n).unwrap_or(u64::MAX).div_ceil(4);
    [
        ("system prompt", sys),
        ("user messages", user),
        ("assistant text", assistant),
        ("tool calls", calls),
        ("tool output", tool_out),
        ("reasoning", reasoning_bytes(messages)),
    ]
    .into_iter()
    .filter(|(_, n)| *n > 0)
    .map(|(k, n)| (k.to_string(), tok(n)))
    .collect()
}

/// Build a report for `/context`.
pub fn report(
    system: &str,
    messages: &[Message],
    model: &str,
    window_override: u64,
) -> ContextReport {
    let window = if window_override > 0 {
        window_override
    } else {
        window_for(model)
    };
    let tokens = estimate_tokens(system, messages);
    let pct = if window == 0 {
        0
    } else {
        ((tokens.saturating_mul(100)) / window).min(100) as u8
    };
    ContextReport {
        tokens,
        window,
        pct,
        messages: messages.len(),
        compacted: false,
        from_provider: false,
    }
}

/// Input, tool schemas and requested output allowance, using one estimate for
/// admission and the context inspector. The estimate is not a model tokenizer.
pub fn request_tokens(
    system: &str,
    messages: &[Message],
    tools: &[crate::llm::ToolSpec],
    output: u32,
) -> u64 {
    let schemas = serde_json::to_vec(tools).map_or(0, |v| v.len() as u64);
    estimate_tokens(system, messages)
        .saturating_add(schemas.div_ceil(4))
        .saturating_add(output.into())
}

/// Context report that includes schemas and space reserved for the answer.
pub fn request_report(
    system: &str,
    messages: &[Message],
    tools: &[crate::llm::ToolSpec],
    window: u64,
    output: u32,
) -> ContextReport {
    let mut rep = report(system, messages, "", window);
    rep.tokens = request_tokens(system, messages, tools, output);
    rep.pct = (rep.tokens.saturating_mul(100) / rep.window.max(1)).min(100) as u8;
    rep
}

/// Reject an estimated oversized request before it can incur provider cost.
pub fn ensure_fits(request: &crate::llm::CompletionRequest, window: u64) -> crate::Result<()> {
    let tokens = request_tokens(
        request.system.as_deref().unwrap_or(""),
        &request.messages,
        &request.tools,
        request.max_tokens.unwrap_or(0),
    );
    if tokens > window {
        return Err(crate::Error::Config(format!(
            "context for {} needs about {tokens} tokens including tools and output allowance, above its {window}-token window after compaction; use a larger-window model, start a new session with the remaining task, or correct [context_windows] if this limit is wrong",
            request.model
        )));
    }
    Ok(())
}

/// Whether auto-compact should run.
pub fn should_compact(rep: &ContextReport) -> bool {
    rep.pct >= AUTO_COMPACT_PCT
}

/// The first line of every extract.
const EXTRACT_HEAD: &str = "[compacted earlier context]";
/// Where an extract's messages begin; they run to its end.
const HISTORY_HEAD: &str = "Earlier messages, oldest first:";
/// The same line in extracts written before extracts were held to a size.
const OLD_HISTORY_HEAD: &str = "Earlier user messages and assistant notes (verbatim):";
/// What Ryter itself says to the model starts so; it is not a user turn.
const RYTER_MARK: &str = "[Ryter]";
/// A note from the model or from Ryter is cut to this when the extract is
/// over its size.
const NOTE_BYTES: usize = 600;
/// A message of the user's longer than this is cut when the extract is
/// still over its size with every note dropped.
const LONG_USER_BYTES: usize = 4_000;
/// Files named in an extract before "and N more".
const NAMES_MAX: usize = 100;
/// An older tool output or call argument longer than this is shortened...
const SHORTEN_OVER: usize = 2_048;
/// ...to this much of each end.
const KEEP_EACH_END: usize = 768;

/// What an extract may take, in bytes, for a model with `window` tokens: a
/// tenth of the window at four bytes a token.
pub fn extract_budget(window: u64) -> usize {
    usize::try_from(window)
        .unwrap_or(usize::MAX)
        .saturating_mul(4)
        / 10
}

/// Whether `m` is a message the user sent: not Ryter's own word to the
/// model, and not an extract.
fn is_users_turn(m: &Message) -> bool {
    m.role == "user" && !m.content.starts_with(RYTER_MARK) && !m.content.starts_with(EXTRACT_HEAD)
}

/// Replace the prefix with a deterministic extract. No-op when already short.
/// `note` is kept in the extract: what must outlive the messages dropped.
/// The extract is held to `budget` bytes: a last extract in the prefix is
/// folded in, not carried whole, so compaction after compaction stays the
/// same size however long the session runs.
pub fn compact(
    messages: &[Message],
    note: &str,
    keep_user_turns: usize,
    budget: usize,
) -> Vec<Message> {
    // Only the user's own messages count as turns: Ryter's notes to the
    // model (a check's result, "not done") and the last extract are
    // user-role messages too, and counted they left fewer real turns kept.
    let user_idx: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| is_users_turn(m))
        .map(|(i, _)| i)
        .collect();
    let cut = if user_idx.len() > keep_user_turns {
        user_idx[user_idx.len() - keep_user_turns.max(1)]
    } else {
        0
    };
    let mut out = if cut > 0 {
        let mut next = vec![Message {
            role: "user".into(),
            content: Extract::of(&messages[..cut]).render(note, budget),
            tool_call_id: None,
            tool_calls: None,
            messages_blocks: None,
            reasoning_details: None,
        }];
        next.extend(messages[cut..].iter().cloned());
        next
    } else {
        messages.to_vec()
    };
    // A single user turn can contain many tool rounds. Keep call/result
    // identities and all narrative, but shorten older bulky tool results,
    // and older bulky call arguments (a `write` carries its whole file).
    // The newest call and its results stay whole for the next response.
    let latest_call = out
        .iter()
        .rposition(|m| m.tool_calls.as_ref().is_some_and(|c| !c.is_empty()))
        .unwrap_or(0);
    for m in &mut out[..latest_call] {
        if m.role == "tool" && m.content.len() > SHORTEN_OVER {
            m.content = shorten(
                &m.content,
                "\n[earlier tool output shortened; re-read the source if needed]\n",
            );
        }
        if let Some(calls) = &mut m.tool_calls {
            for c in calls {
                if c.arguments.len() <= SHORTEN_OVER {
                    continue;
                }
                if let Ok(mut v) = serde_json::from_str::<Value>(&c.arguments) {
                    if shorten_strings(&mut v) {
                        c.arguments = v.to_string();
                    }
                }
            }
        }
        // Sent instead of `tool_calls` once thinking is on.
        if let Some(blocks) = &mut m.messages_blocks {
            for b in blocks {
                if let crate::llm::MessagesBlock::ToolUse { input, .. } = b {
                    shorten_strings(input);
                }
            }
        }
    }
    if estimate_tokens("", &out) < estimate_tokens("", messages) {
        out
    } else {
        messages.to_vec()
    }
}

/// `s` with only [`KEEP_EACH_END`] bytes of each end, `mark` between.
fn shorten(s: &str, mark: &str) -> String {
    let mut head = KEEP_EACH_END.min(s.len());
    while !s.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = s.len().saturating_sub(KEEP_EACH_END).max(head);
    while !s.is_char_boundary(tail) {
        tail += 1;
    }
    format!("{}{mark}{}", &s[..head], &s[tail..])
}

/// Shorten every string in `v` longer than [`SHORTEN_OVER`]; true if any was.
fn shorten_strings(v: &mut Value) -> bool {
    match v {
        Value::String(s) if s.len() > SHORTEN_OVER => {
            let left_out = s.len() - 2 * KEEP_EACH_END;
            *s = shorten(
                s,
                &format!(
                    "\n[… {left_out} bytes left out by compaction; read the file for them …]\n"
                ),
            );
            true
        }
        Value::Array(a) => a.iter_mut().fold(false, |any, x| shorten_strings(x) || any),
        Value::Object(o) => o
            .values_mut()
            .fold(false, |any, x| shorten_strings(x) || any),
        _ => false,
    }
}

/// `s` cut to about `max` bytes, its start and its end kept.
fn cut_to(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let half = max / 2;
    let mut head = half;
    while !s.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = s.len() - half;
    while !s.is_char_boundary(tail) {
        tail += 1;
    }
    format!("{} […] {}", s[..head].trim_end(), s[tail..].trim_start())
}

/// Who said a message kept in an extract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Who {
    /// The user: kept longest.
    User,
    /// The model, or Ryter's own word to it: cut and dropped first.
    Note,
}

#[derive(Debug, Clone)]
struct Entry {
    who: Who,
    /// The line it is written under: `user`, `user (plan hat)`,
    /// `assistant`, `ryter`.
    label: String,
    text: String,
}

impl Entry {
    /// Bytes it takes in the extract: `\n{label}:\n{text}\n`.
    fn len(&self) -> usize {
        self.label.len() + self.text.len() + 4
    }

    /// One user message: the hat's note in front is named, not copied.
    fn user(content: &str) -> Self {
        if let Some(rest) = content.strip_prefix("[hat: ") {
            let hat = rest.split([' ', ']']).next().unwrap_or_default();
            if let Some((_, said)) = content.split_once("\n\n") {
                return Self {
                    who: Who::User,
                    label: format!("user ({hat} hat)"),
                    text: said.trim().to_string(),
                };
            }
        }
        Self {
            who: Who::User,
            label: "user".into(),
            text: content.trim().to_string(),
        }
    }

    fn note(label: &str, content: &str) -> Self {
        Self {
            who: Who::Note,
            label: label.into(),
            text: content.trim().to_string(),
        }
    }

    /// The label line that starts an entry, read back from an extract.
    fn from_label(line: &str) -> Option<(Who, String)> {
        let label = line.strip_suffix(':')?;
        match label {
            "user" => Some((Who::User, label.into())),
            "assistant" | "ryter" => Some((Who::Note, label.into())),
            l if l.starts_with("user (") && l.ends_with(" hat)") => Some((Who::User, l.into())),
            _ => None,
        }
    }
}

/// What an extract says, built from the messages it replaces.
#[derive(Debug, Default)]
struct Extract {
    tools: BTreeSet<String>,
    files: BTreeSet<String>,
    /// Files an earlier extract had no room to name.
    files_more: usize,
    entries: Vec<Entry>,
    /// Messages folded in, by every pass so far.
    folded: usize,
    /// Messages left out to fit, by every pass so far: the user's, notes.
    left_out: (usize, usize),
}

impl Extract {
    fn of(old: &[Message]) -> Self {
        let mut x = Self::default();
        for m in old {
            if let Some(calls) = &m.tool_calls {
                for c in calls {
                    if !c.name.is_empty() {
                        x.tools.insert(c.name.clone());
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(&c.arguments) {
                        if let Some(p) = v.get("path").and_then(Value::as_str) {
                            if !p.is_empty() {
                                x.files.insert(p.to_string());
                            }
                        }
                    }
                }
            }
            match m.role.as_str() {
                "user" if m.content.starts_with(EXTRACT_HEAD) => {
                    x.fold(&m.content);
                    continue;
                }
                "user" if m.content.starts_with(RYTER_MARK) => {
                    x.entries.push(Entry::note("ryter", &m.content));
                }
                "user" if !m.content.trim().is_empty() => {
                    x.entries.push(Entry::user(&m.content));
                }
                "assistant" if !m.content.trim().is_empty() => {
                    x.entries.push(Entry::note("assistant", &m.content));
                }
                _ => {}
            }
            x.folded += 1;
        }
        x
    }

    /// Take in a last extract (this format, or the one before it): its
    /// tools, files, counts and messages, not its note, which is said anew.
    fn fold(&mut self, text: &str) {
        let (meta, history) = match text
            .split_once(&format!("\n{HISTORY_HEAD}\n"))
            .or_else(|| text.split_once(&format!("\n{OLD_HISTORY_HEAD}\n")))
        {
            Some((meta, history)) => (meta, history),
            None => (text, ""),
        };
        let number = |s: &str| {
            s.chars()
                .filter(char::is_ascii_digit)
                .collect::<String>()
                .parse::<usize>()
                .unwrap_or(0)
        };
        for line in meta.lines() {
            if let Some(list) = line.strip_prefix("Tools used: ") {
                self.tools
                    .extend(list.split(", ").filter(|t| !t.is_empty()).map(String::from));
            } else if let Some(list) = line.strip_prefix("Files touched: ") {
                let (list, more) = match list.split_once(" (and ") {
                    Some((list, more)) => (list, number(more)),
                    None => (list, 0),
                };
                self.files_more += more;
                self.files
                    .extend(list.split(", ").filter(|f| !f.is_empty()).map(String::from));
            } else if let Some(n) = line.strip_prefix("Messages folded in so far: ") {
                self.folded += number(n);
            } else if let Some(rest) = line.strip_prefix("Left out to fit: ") {
                if let Some((users, notes)) = rest.split_once(", ") {
                    self.left_out.0 += number(users);
                    self.left_out.1 += number(notes);
                }
            }
        }
        // The extract before this format ended with its count.
        let mut history = history.trim_end();
        if let Some((rest, last)) = history.rsplit_once('\n') {
            if let Some(n) = last
                .strip_prefix("Dropped ")
                .and_then(|l| l.strip_suffix(" older messages."))
            {
                self.folded += number(n);
                history = rest;
            }
        }
        let mut current: Option<Entry> = None;
        let mut blank = true;
        for line in history.lines() {
            match Entry::from_label(line).filter(|_| blank) {
                Some((who, label)) => {
                    self.entries.extend(current.take().map(Self::tidy));
                    current = Some(Entry {
                        who,
                        label,
                        text: String::new(),
                    });
                }
                None => {
                    if let Some(e) = &mut current {
                        e.text.push_str(line);
                        e.text.push('\n');
                    }
                }
            }
            blank = line.trim().is_empty();
        }
        self.entries.extend(current.map(Self::tidy));
    }

    /// An entry read back: trimmed, and in the older format the user's
    /// messages carried Ryter's notes and the hat's note too.
    fn tidy(mut e: Entry) -> Entry {
        e.text = e.text.trim().to_string();
        if e.label == "user" {
            if e.text.starts_with(RYTER_MARK) {
                return Entry::note("ryter", &e.text);
            }
            if e.text.starts_with("[hat: ") {
                return Entry::user(&e.text);
            }
        }
        e
    }

    fn meta(&self, note: &str) -> String {
        let mut s = format!("{EXTRACT_HEAD}\n");
        if !self.tools.is_empty() {
            s.push_str("Tools used: ");
            s.push_str(&self.tools.iter().cloned().collect::<Vec<_>>().join(", "));
            s.push('\n');
        }
        if !self.files.is_empty() {
            let more = self.files.len().saturating_sub(NAMES_MAX) + self.files_more;
            s.push_str("Files touched: ");
            s.push_str(
                &self
                    .files
                    .iter()
                    .take(NAMES_MAX)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            if more > 0 {
                s.push_str(&format!(" (and {more} more)"));
            }
            s.push('\n');
        }
        let note = note.trim();
        if !note.is_empty() {
            s.push_str(note);
            s.push('\n');
        }
        s.push_str(&format!("Messages folded in so far: {}\n", self.folded));
        if self.left_out != (0, 0) {
            s.push_str(&format!(
                "Left out to fit: {} of the user's, {} notes from the assistant and Ryter\n",
                self.left_out.0, self.left_out.1
            ));
        }
        s
    }

    /// The extract, held to `budget` bytes: the oldest notes are cut, then
    /// dropped; then the user's oldest long messages are cut, then dropped.
    fn render(mut self, note: &str, budget: usize) -> String {
        // Room for the counts' own line to grow as entries are left out.
        let room = budget.saturating_sub(self.meta(note).len() + HISTORY_HEAD.len() + 160);
        let mut used: usize = self.entries.iter().map(Entry::len).sum();
        for (who, max) in [(Who::Note, NOTE_BYTES), (Who::User, LONG_USER_BYTES)] {
            for e in self.entries.iter_mut().filter(|e| e.who == who) {
                if used <= room {
                    break;
                }
                let before = e.len();
                e.text = cut_to(&e.text, max);
                used -= before - e.len();
            }
            while used > room {
                let Some(i) = self.entries.iter().position(|e| e.who == who) else {
                    break;
                };
                used -= self.entries.remove(i).len();
                match who {
                    Who::User => self.left_out.0 += 1,
                    Who::Note => self.left_out.1 += 1,
                }
            }
        }
        let mut s = self.meta(note);
        s.push_str(HISTORY_HEAD);
        s.push('\n');
        for e in &self.entries {
            s.push_str(&format!("\n{}:\n{}\n", e.label, e.text));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 200k window's extract.
    const BUDGET: usize = 80_000;

    fn msg(role: &str, content: &str) -> Message {
        Message {
            role: role.into(),
            content: content.into(),
            tool_call_id: None,
            tool_calls: None,
            messages_blocks: None,
            reasoning_details: None,
        }
    }

    #[test]
    fn grok_window_is_500k() {
        assert_eq!(window_for("grok-4.6"), 500_000);
        assert_eq!(window_for("grok-4.7"), 500_000);
        assert_eq!(window_for("grok-4.3"), 1_000_000);
        assert_eq!(window_for("claude-opus-5-5"), 1_000_000);
        assert_eq!(window_for("claude-sonnet-4-6"), 1_000_000);
        assert_eq!(window_for("claude-haiku-4-5"), 200_000);
        assert_eq!(window_for("gpt-6.1-sol"), 1_050_000);
        assert_eq!(window_for("gpt-5.6-luna"), 1_050_000);
        assert_eq!(window_for("mystery"), ASSUMED_WINDOW);
        assert_eq!(known_window("mystery"), None);
        assert_eq!(known_window("claude-haiku-4-5"), Some(200_000));
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(500_000), "500k");
        assert_eq!(format_context_used(0, 500_000), "0/500k");
        assert_eq!(format_context_used(12_000, 500_000), "12k/500k");
    }

    #[test]
    fn context_render_marks_estimates_only() {
        let est = ContextReport {
            tokens: 1_000,
            window: 10_000,
            pct: 10,
            messages: 2,
            compacted: false,
            from_provider: false,
        };
        assert!(est.render().contains("~1000/10000"));
        let exact = ContextReport {
            from_provider: true,
            ..est
        };
        assert!(exact.render().contains(" 1000/10000"));
        assert!(!exact.render().contains("~1000"));
    }

    #[test]
    fn compact_keeps_tail_and_extracts_files() {
        let mut messages = Vec::new();
        for i in 0..6 {
            messages.push(msg("user", &format!("turn {i}")));
            messages.push(Message {
                role: "assistant".into(),
                content: String::new(),
                tool_call_id: None,
                tool_calls: Some(vec![crate::llm::AssistantToolCall {
                    id: format!("c{i}"),
                    name: "read_file".into(),
                    arguments: format!(r#"{{"path":"src/f{i}.rs"}}"#),
                }]),
                messages_blocks: None,
                reasoning_details: None,
            });
            messages.push(Message {
                role: "tool".into(),
                content: "x".repeat(8_000),
                tool_call_id: Some(format!("c{i}")),
                tool_calls: None,
                messages_blocks: None,
                reasoning_details: None,
            });
        }
        let out = compact(&messages, "ship the flag", 2, BUDGET);
        assert!(
            out.len() < messages.len(),
            "{} vs {}",
            out.len(),
            messages.len()
        );
        assert!(out[0].content.contains("[compacted"));
        assert!(out[0].content.contains("src/f0.rs"));
        assert!(out[0].content.contains("read_file"));
        assert!(out[0].content.contains("ship the flag"));
        assert!(out.iter().any(|m| m.content == "turn 5"));
        assert!(!out.iter().any(|m| m.content == "turn 0"));
        let before = estimate_tokens("", &messages);
        let after = estimate_tokens("", &out);
        assert!(after < before, "{after} vs {before}");
    }

    #[test]
    fn breakdown_sums_to_estimate() {
        let messages = vec![
            msg("user", "hello there"),
            msg("assistant", "hi"),
            Message {
                role: "tool".into(),
                content: "x".repeat(400),
                tool_call_id: Some("c1".into()),
                tool_calls: None,
                messages_blocks: None,
                reasoning_details: None,
            },
        ];
        let rows = breakdown("sys", &messages);
        let names: Vec<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            names,
            [
                "system prompt",
                "user messages",
                "assistant text",
                "tool output"
            ]
        );
        let total: u64 = rows.iter().map(|(_, n)| n).sum();
        let est = estimate_tokens("sys", &messages);
        assert!(
            total >= est && total <= est + rows.len() as u64,
            "{total} vs {est}"
        );
    }

    #[test]
    fn a_long_single_turn_shrinks_without_losing_constraints_or_tool_identity() {
        let mut messages = vec![msg(
            "user",
            "Never change the public API; keep compatibility.",
        )];
        for i in 0..3 {
            messages.push(Message {
                role: "assistant".into(),
                content: "Still need to validate compatibility".into(),
                tool_call_id: None,
                tool_calls: Some(vec![crate::llm::AssistantToolCall {
                    id: format!("c{i}"),
                    name: "read_file".into(),
                    arguments: "{}".into(),
                }]),
                messages_blocks: None,
                reasoning_details: None,
            });
            messages.push(Message {
                role: "tool".into(),
                content: "界".repeat(6000),
                tool_call_id: Some(format!("c{i}")),
                tool_calls: None,
                messages_blocks: None,
                reasoning_details: None,
            });
        }
        let next = compact(&messages, "", 4, BUDGET);
        assert_eq!(next.len(), messages.len());
        assert!(estimate_tokens("", &next) < estimate_tokens("", &messages) / 2);
        assert_eq!(next[0], messages[0]);
        assert_eq!(next.last(), messages.last());
        for (before, after) in messages.iter().zip(&next) {
            assert_eq!(before.tool_calls, after.tool_calls);
            assert_eq!(before.tool_call_id, after.tool_call_id);
            if before.role != "tool" {
                assert_eq!(before.content, after.content);
            }
        }
        assert!(next[2].content.contains("re-read the source"));
    }

    #[test]
    fn extracted_history_preserves_user_constraints_and_unfinished_work() {
        let mut history = vec![
            msg("user", "Keep the CSV format compatible."),
            msg("assistant", "Still need the empty-file regression."),
        ];
        history.push(msg("tool", &"bulky output".repeat(2000)));
        history.push(msg("user", "Continue"));
        let next = compact(&history, "approved plan path", 1, BUDGET);
        assert!(next[0].content.contains("Keep the CSV format compatible."));
        assert!(
            next[0]
                .content
                .contains("Still need the empty-file regression.")
        );
        assert!(next[0].content.contains("approved plan path"));
        assert_eq!(next.last(), history.last());
    }

    /// One build turn as a real session has them: a request, rounds of a
    /// line of text with a call and its output, and a summary at the end.
    fn build_turn(i: usize) -> Vec<Message> {
        let mut turn = vec![msg(
            "user",
            &format!("turn {i}: {}", "make the next change. ".repeat(12)),
        )];
        for r in 0..8 {
            let id = format!("t{i}r{r}");
            turn.push(Message {
                role: "assistant".into(),
                content: "Reading the module before the change. ".repeat(4),
                tool_call_id: None,
                tool_calls: Some(vec![crate::llm::AssistantToolCall {
                    id: id.clone(),
                    name: "read_file".into(),
                    arguments: format!(r#"{{"path":"src/m{r}.rs"}}"#),
                }]),
                messages_blocks: None,
                reasoning_details: None,
            });
            turn.push(Message {
                role: "tool".into(),
                content: "x".repeat(6_000),
                tool_call_id: Some(id),
                tool_calls: None,
                messages_blocks: None,
                reasoning_details: None,
            });
        }
        turn.push(msg("assistant", &"Done: what changed and why. ".repeat(50)));
        turn
    }

    /// A long session is compacted many times. Each pass has to bring the
    /// transcript back under the line that set it off, however many passes
    /// came before: an extract that carries the last extract whole grows
    /// with every pass, until a pass can no longer get under the line, the
    /// next round compacts again, and the request is finally refused.
    #[test]
    fn repeated_compaction_stays_under_the_line_however_long_the_session() {
        let window: u64 = 200_000;
        let line = window * u64::from(AUTO_COMPACT_PCT) / 100;
        let mut transcript: Vec<Message> = Vec::new();
        let mut passes = 0;
        let mut first_after = None;
        for i in 0..600 {
            transcript.extend(build_turn(i));
            if estimate_tokens("", &transcript) < line {
                continue;
            }
            transcript = compact(&transcript, "", KEEP_USER_TURNS, extract_budget(window));
            passes += 1;
            let after = estimate_tokens("", &transcript);
            let first = *first_after.get_or_insert(after);
            assert!(
                after < line,
                "turn {i}, pass {passes}: compaction left {after} tokens, over the \
                 {line}-token line that set it off (the first pass left {first})"
            );
            let extract = transcript[0].content.len();
            assert!(
                extract <= extract_budget(window),
                "turn {i}, pass {passes}: the extract is {extract} bytes, over its {}",
                extract_budget(window)
            );
        }
        assert!(passes > 3, "the session never compacted enough: {passes}");
    }

    /// What a call sent is bulk too: a `write` carries the whole file. An
    /// older call's arguments are shortened like its output, and still say
    /// which file, as JSON the provider takes; the Messages echo of the
    /// same call (sent instead of `tool_calls` once thinking is on) too.
    #[test]
    fn older_write_arguments_are_shortened_like_their_output() {
        let body = "fn f() {}\n".repeat(4_000);
        let mut messages = vec![msg("user", "write the files")];
        for i in 0..4 {
            let id = format!("w{i}");
            let args = serde_json::json!({"path": format!("src/f{i}.rs"), "content": body});
            messages.push(Message {
                role: "assistant".into(),
                content: String::new(),
                tool_call_id: None,
                tool_calls: Some(vec![crate::llm::AssistantToolCall {
                    id: id.clone(),
                    name: "write".into(),
                    arguments: args.to_string(),
                }]),
                messages_blocks: (i % 2 == 1).then(|| {
                    vec![
                        crate::llm::MessagesBlock::Thinking {
                            thinking: "plan".into(),
                            signature: "sig".into(),
                        },
                        crate::llm::MessagesBlock::ToolUse {
                            id: id.clone(),
                            name: "write".into(),
                            input: args.clone(),
                        },
                    ]
                }),
                reasoning_details: None,
            });
            messages.push(Message {
                role: "tool".into(),
                content: format!("wrote src/f{i}.rs"),
                tool_call_id: Some(id),
                tool_calls: None,
                messages_blocks: None,
                reasoning_details: None,
            });
        }
        let next = compact(&messages, "", KEEP_USER_TURNS, BUDGET);
        // The newest call is the one the next reply answers: left whole.
        let newest = next.len() - 2;
        for (i, m) in next
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == "assistant")
        {
            let call = &m.tool_calls.as_ref().unwrap()[0];
            let args: serde_json::Value = serde_json::from_str(&call.arguments)
                .unwrap_or_else(|e| panic!("arguments of {} are not JSON: {e}", call.id));
            assert!(args.get("path").is_some(), "{} lost its path", call.id);
            let echoed = m.messages_blocks.iter().flatten().find_map(|b| match b {
                crate::llm::MessagesBlock::ToolUse { input, .. } => Some(input.to_string()),
                _ => None,
            });
            if i == newest {
                assert!(call.arguments.len() > body.len(), "the newest call was cut");
                continue;
            }
            assert!(
                call.arguments.len() < 4_096,
                "{}: an older write still carries {} bytes",
                call.id,
                call.arguments.len()
            );
            if let Some(echoed) = echoed {
                assert!(
                    echoed.len() < 4_096,
                    "{}: the Messages echo still carries {} bytes",
                    call.id,
                    echoed.len()
                );
            }
        }
    }

    fn extract_of(messages: &[Message]) -> &str {
        assert!(
            messages[0].content.starts_with(EXTRACT_HEAD),
            "{:?}",
            messages[0]
        );
        &messages[0].content
    }

    /// Over its size, an extract gives up the assistant's and Ryter's notes
    /// before any of the user's messages, says how many went, and says the
    /// plan's note once however many passes carry it.
    #[test]
    fn a_full_extract_drops_notes_before_the_users_words_and_says_so() {
        let mut transcript = vec![msg(
            "user",
            "[hat: build — you may change files]\n\nNever touch the public API.",
        )];
        for i in 0..40 {
            transcript.push(msg("assistant", &format!("note {i} {}", "n".repeat(900))));
            transcript.push(msg(
                "user",
                &format!("[Ryter] `cargo test` passed after your edits {i}."),
            ));
            transcript.push(msg("user", &format!("request {i}")));
        }
        let mut next = transcript.clone();
        for pass in 0..3 {
            next.push(msg("user", &format!("more {pass}")));
            next.push(msg("assistant", "ok"));
            // Bulk, so each pass has something to take: it folds again.
            next.push(msg("tool", &"o".repeat(8_000)));
            next = compact(&next, "The plan is in `.ryter/plan.md`.", 2, 8_000);
        }
        let x = extract_of(&next);
        assert!(x.len() <= 8_000, "{}", x.len());
        assert!(
            x.contains("user (build hat):\nNever touch the public API."),
            "{x}"
        );
        assert!(!x.contains("[hat: build"), "the hat's note was copied: {x}");
        for i in 0..40 {
            let said = format!("\nrequest {i}\n");
            assert!(x.contains(&said), "request {i} went: {x}");
        }
        assert!(!x.contains("note 0 "), "the oldest note stayed: {x}");
        assert!(x.contains("\nmore 0\n"), "the last pass did not fold: {x}");
        assert_eq!(x.matches("The plan is in").count(), 1, "{x}");
        assert!(x.contains("Left out to fit: 0 of the user's,"), "{x}");
        let left: usize = x
            .lines()
            .find_map(|l| l.strip_prefix("Left out to fit: 0 of the user's, "))
            .and_then(|l| l.split(' ').next())
            .and_then(|n| n.parse().ok())
            .unwrap();
        assert!(left > 0);
        assert!(x.contains("Messages folded in so far: "), "{x}");
    }

    /// A session compacted before extracts had a size carries one written
    /// the old way. It is folded like any other, not kept whole as one of
    /// the user's messages, and its count goes on.
    #[test]
    fn an_extract_in_the_old_format_is_folded_not_carried() {
        let mut old = String::from(
            "[compacted earlier context]\nTools used: read_file\nFiles touched: src/a.rs\n\
             Earlier user messages and assistant notes (verbatim):\n",
        );
        old.push_str("\nuser:\n[hat: plan — nothing may change]\n\nKeep the CSV format.\n");
        old.push_str("\nuser:\n[Ryter] `make` passed after your edits to a.rs.\n");
        for i in 0..30 {
            old.push_str(&format!("\nassistant:\nold note {i} {}\n", "o".repeat(900)));
        }
        old.push_str("Dropped 120 older messages.\n");
        let mut transcript = vec![msg("user", &old)];
        for i in 0..6 {
            transcript.push(msg("user", &format!("ask {i}")));
            transcript.push(msg("assistant", "done"));
        }
        let next = compact(&transcript, "", 2, 8_000);
        let x = extract_of(&next);
        assert!(x.len() <= 8_000, "{}", x.len());
        assert!(x.contains("user (plan hat):\nKeep the CSV format."), "{x}");
        assert!(x.contains("Tools used: read_file"), "{x}");
        assert!(x.contains("Files touched: src/a.rs"), "{x}");
        assert!(!x.contains("Dropped 120"), "{x}");
        // 120 before, and 4 asks with their answers.
        assert!(x.contains("Messages folded in so far: 128\n"), "{x}");
        assert!(
            !x.contains("user:\n[Ryter]"),
            "Ryter's note read as the user's: {x}"
        );
    }

    /// Ryter's own word to the model (a check's result, "not done") is a
    /// user-role message, but not a turn: counted, it left fewer of the
    /// user's turns kept than the setting says.
    #[test]
    fn ryters_notes_do_not_count_as_the_users_turns() {
        let mut transcript = Vec::new();
        for i in 0..3 {
            transcript.push(msg("user", &format!("real turn {i} {}", "r".repeat(3000))));
            for j in 0..3 {
                transcript.push(msg("assistant", "editing"));
                transcript.push(msg("tool", &"o".repeat(8_000)));
                transcript.push(msg(
                    "user",
                    &format!("[Ryter] Not done: `make` still fails. {j}"),
                ));
            }
        }
        // Three turns of the user's, four kept: nothing is folded away.
        let next = compact(&transcript, "", 4, BUDGET);
        assert_eq!(next, transcript);
        // Two kept: the first turn goes, with its notes, the second stays whole.
        let next = compact(&transcript, "", 2, BUDGET);
        assert!(next[1].content.starts_with("real turn 1"), "{:?}", next[1]);
        assert!(extract_of(&next).contains("real turn 0"));
    }

    #[test]
    fn short_transcript_is_unchanged() {
        let messages = vec![msg("user", "hi"), msg("assistant", "hello")];
        assert_eq!(compact(&messages, "", 4, BUDGET), messages);
    }

    #[test]
    fn report_pct_uses_window() {
        let messages = vec![msg("user", &"a".repeat(400))];
        let r = report("", &messages, "mystery", 0);
        assert_eq!(r.window, 200_000);
        assert!(r.pct < 5);
        let r2 = report("", &messages, "mystery", 200);
        assert!(r2.pct >= 50, "{}", r2.pct);
        assert!(r2.render().contains("context"));
    }
}
