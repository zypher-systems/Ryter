//! Provider proof: what each built-in connection really sent, replayed through
//! the real HTTP client and the real agent loop, and the budgeted live run
//! that records it. `docs/provider-proof.md` says how to run it and keeps the
//! dated trail.
//!
//! The same checks judge the live run and every replay after it, so a kept
//! capture cannot drift from what was proven. Tool calls, parallel calls,
//! truncation, cancellation and accounting are each read off the capture;
//! a dropped stream is the capture cut short.
use super::*;
use crate::llm::{
    Backend, CompletionRequest, HttpProvider, Tap, backend_for, parse_blocks, parse_sse,
};
use serde_json::json;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// The connections with a recorded capture, each kept by a live run: nothing
/// here is written by hand.
const RECORDED: &[&str] = &["anthropic", "openrouter"];
/// A check's verdict: why it failed, in words a live run can print.
type Proven<T = ()> = std::result::Result<T, String>;

const BUILT_IN: [&str; 4] = ["spacexsi", "openrouter", "anthropic", "openai"];

const NOTES: &str = "The kettle number is 7431.\n";
const FIRST: &str = "The first number is 2718.\n";
const SECOND: &str = "The second number is 3141.\n";
/// The sum is there to make the model think before it calls: a reply that
/// thinks is the only one with a signature to give back.
const ROUND_TRIP: &str = "Think it through first: is 391 prime? Then call read_file on notes.txt, \
     and no other tool, and tell me the number in the file. Say nothing about 391.";
const PARALLEL: &str = "Read a.txt and b.txt, with two read_file calls in the same reply, \
     not one after the other, and no other tool. Then tell me both numbers. Do nothing else.";
const LONG: &str = "Count from 1 to 500, one number per line.";

/// The replies a capture holds, in the order a recording makes them.
const REPLIES: [&str; 5] = [
    "tool_call",
    "after_tool",
    "parallel",
    "after_parallel",
    "truncated",
];

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Meta {
    connection: String,
    model: String,
    captured: String,
    ryter: String,
    commit: String,
}

#[derive(Debug, Clone)]
struct Capture {
    meta: Meta,
    replies: HashMap<String, String>,
}

impl Capture {
    fn backend(&self) -> Backend {
        backend_for(&crate::Config::default().connections[&self.meta.connection])
    }

    fn reply(&self, name: &str) -> &str {
        &self.replies[name]
    }

    fn load(dir: &Path) -> Capture {
        let text = std::fs::read_to_string(dir.join("meta.toml")).unwrap();
        let replies = REPLIES
            .iter()
            .map(|name| {
                let sse = std::fs::read_to_string(dir.join(format!("{name}.sse"))).unwrap();
                (name.to_string(), sse)
            })
            .collect();
        Capture {
            meta: toml::from_str(&text).unwrap(),
            replies,
        }
    }

    fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(
            dir.join("meta.toml"),
            toml::to_string(&self.meta).expect("meta"),
        )?;
        for (name, sse) in &self.replies {
            std::fs::write(dir.join(format!("{name}.sse")), sse)?;
        }
        Ok(())
    }
}

fn wire_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/wire")
}

fn target_wire() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wire")
}

// ---- the local end of the wire ----

/// How a served reply ends.
#[derive(Clone)]
enum End {
    Whole,
    /// The socket closes cleanly at this byte: no sign anything went wrong.
    Closed(usize),
    /// The socket closes at this byte, short of the length it promised.
    Broken(usize),
    /// Sends up to this byte, then the user presses Esc and the reply never
    /// comes.
    Cancelled(usize, Arc<crate::Cancel>),
}

/// A provider on a real socket, serving `replies` to requests in order.
struct Upstream {
    base: String,
    /// Each request as it arrived, headers and body.
    seen: Arc<Mutex<Vec<String>>>,
}

fn serve(replies: Vec<(String, End)>) -> Upstream {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for (sse, end) in replies {
            let Ok((mut sock, _)) = listener.accept() else {
                return;
            };
            log.lock().unwrap().push(read_request(&mut sock));
            let length = match end {
                End::Broken(_) => format!("content-length: {}\r\n", sse.len()),
                _ => String::new(),
            };
            let _ = write!(
                sock,
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n{length}\
                 connection: close\r\n\r\n"
            );
            let upto = match &end {
                End::Whole => sse.len(),
                End::Closed(at) | End::Broken(at) | End::Cancelled(at, _) => *at,
            };
            // Small slices, so a frame or a character can fall across two
            // reads the way it does on a real connection.
            for slice in sse.as_bytes()[..upto].chunks(61) {
                let _ = sock.write_all(slice);
                let _ = sock.flush();
            }
            if let End::Cancelled(_, cancel) = end {
                std::thread::sleep(std::time::Duration::from_millis(30));
                cancel.cancel();
                let mut rest = [0u8; 64];
                let _ = sock.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                let _ = sock.read(&mut rest);
            }
        }
    });
    Upstream { base, seen }
}

fn read_request(sock: &mut std::net::TcpStream) -> String {
    let mut req = Vec::new();
    let mut buf = [0u8; 65_536];
    loop {
        let Ok(n) = sock.read(&mut buf) else { break };
        req.extend_from_slice(&buf[..n]);
        let text = String::from_utf8_lossy(&req);
        if let Some(head) = text.find("\r\n\r\n") {
            let length = text[..head]
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            if req.len() >= head + 4 + length {
                break;
            }
        }
        if n == 0 {
            break;
        }
    }
    String::from_utf8_lossy(&req).into_owned()
}

// ---- one agent on one connection ----

fn provider_on(connection: &str, base: Option<&str>, key: String, tap: Tap) -> HttpProvider {
    let mut conn = crate::Config::default().connections[connection].clone();
    if let Some(base) = base {
        conn.base_url = base.into();
    }
    HttpProvider::new(&conn, key).tapped(tap)
}

/// The build hat, on this connection, in a project holding the files the
/// prompts name.
fn agent_on(
    connection: &str,
    model: &str,
    base: Option<&str>,
    key: String,
    tap: Tap,
) -> (TempDir, TempDir, Agent) {
    let (home, cwd, mut agent) = setup(ReplayProvider::new(vec![]));
    for (name, text) in [("notes.txt", NOTES), ("a.txt", FIRST), ("b.txt", SECOND)] {
        std::fs::write(cwd.path().join(name), text).unwrap();
    }
    agent.provider = Arc::new(provider_on(connection, base, key, tap));
    agent.connection = connection.into();
    agent.model = model.into();
    (home, cwd, agent)
}

// ---- what proves a reply ----

/// The bill as the wire states it, read without the parser under test.
/// Anthropic's `input_tokens` leaves out the cache; the others' includes it.
fn stated_usage(backend: Backend, sse: &str) -> Usage {
    let mut usage = Usage::default();
    let n = |v: &Value, k: &str| v[k].as_u64().unwrap_or(0);
    for line in sse.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(data.trim()) else {
            continue;
        };
        match backend {
            Backend::ChatCompletions if v["usage"].is_object() => {
                let u = &v["usage"];
                usage = Usage {
                    input_tokens: n(u, "prompt_tokens"),
                    output_tokens: n(u, "completion_tokens"),
                    cached_tokens: n(&u["prompt_tokens_details"], "cached_tokens"),
                    cache_write_tokens: n(&u["prompt_tokens_details"], "cache_write_tokens"),
                };
            }
            Backend::Responses if v["response"]["usage"].is_object() => {
                let u = &v["response"]["usage"];
                usage = Usage {
                    input_tokens: n(u, "input_tokens"),
                    output_tokens: n(u, "output_tokens"),
                    cached_tokens: n(&u["input_tokens_details"], "cached_tokens"),
                    cache_write_tokens: n(&u["input_tokens_details"], "cache_write_tokens"),
                };
            }
            Backend::Messages => {
                let (u, opening) = match v["type"].as_str() {
                    Some("message_start") => (&v["message"]["usage"], true),
                    Some("message_delta") => (&v["usage"], false),
                    _ => continue,
                };
                if opening || u.get("input_tokens").is_some() {
                    let (read, write) = (
                        n(u, "cache_read_input_tokens"),
                        n(u, "cache_creation_input_tokens"),
                    );
                    usage.input_tokens = n(u, "input_tokens") + read + write;
                    usage.cached_tokens = read;
                    usage.cache_write_tokens = write;
                }
                usage.output_tokens = n(u, "output_tokens");
            }
            _ => {}
        }
    }
    usage
}

/// What the agent's records say each reply cost is what its bytes say.
fn prove_bill(agent: &Agent, backend: Backend, replies: &[&str]) -> Proven {
    let rows = agent.session.spend_log().map_err(|e| e.to_string())?;
    if rows.len() != replies.len() {
        return Err(format!(
            "{} replies, {} spend rows",
            replies.len(),
            rows.len()
        ));
    }
    for (row, sse) in rows.iter().zip(replies) {
        let stated = stated_usage(backend, sse);
        let recorded = Usage {
            input_tokens: row.input_tokens,
            output_tokens: row.output_tokens,
            cached_tokens: row.cached_tokens,
            cache_write_tokens: row.cache_write_tokens,
        };
        if recorded != stated {
            return Err(format!(
                "the wire billed {stated:?}, the record says {recorded:?}"
            ));
        }
        if stated.input_tokens == 0 || stated.output_tokens == 0 {
            return Err(format!("the wire billed nothing: {stated:?}"));
        }
        if row.incomplete {
            return Err("a finished reply's bill is marked incomplete".into());
        }
        let reported = parse_sse(backend, sse)
            .map_err(|e| e.to_string())?
            .iter()
            .find_map(|d| match d {
                StreamDelta::ReportedCost(c) => Some(*c),
                _ => None,
            });
        let priced = reported.or_else(|| agent.book.cost(&row.model, stated));
        let same = match (row.total_usd, priced) {
            (Some(a), Some(b)) => (a - b).abs() < 1e-9,
            (a, b) => a == b,
        };
        if !same {
            return Err(format!(
                "cost {:?} recorded, {priced:?} on the wire and the book",
                row.total_usd
            ));
        }
    }
    Ok(())
}

/// The model called `read_file` on exactly `paths` in one reply, each call
/// ran and was answered under its own id, and the turn's answer says what the
/// files said.
fn prove_calls(agent: &Agent, answer: &str, paths: &[&str], numbers: &[&str]) -> Proven {
    let transcript = &agent.session.transcript;
    let calls = transcript
        .iter()
        .find_map(|m| m.tool_calls.as_ref())
        .ok_or("the model called no tool")?;
    let mut read: Vec<String> = calls
        .iter()
        .filter(|c| c.name == "read_file")
        .filter_map(|c| {
            let args: Value = serde_json::from_str(&c.arguments).ok()?;
            Some(args["path"].as_str()?.trim_start_matches("./").to_string())
        })
        .collect();
    read.sort();
    if read != paths {
        return Err(format!("wanted reads of {paths:?} in one reply: {calls:?}"));
    }
    for call in calls {
        let result = transcript
            .iter()
            .find(|m| m.role == "tool" && m.tool_call_id.as_deref() == Some(&call.id))
            .ok_or_else(|| format!("call {} has no result", call.id))?;
        if !numbers.iter().any(|n| result.content.contains(n)) {
            return Err(format!("call {} read {:?}", call.id, result.content));
        }
    }
    for number in numbers {
        if !answer.contains(number) {
            return Err(format!("the answer lost {number}: {answer:?}"));
        }
    }
    Ok(())
}

/// The next request gave the model its own words back: every call's id, and
/// on Messages every thinking signature, unchanged.
fn prove_echo(backend: Backend, reply: &str, next_request: &str) -> Proven {
    let mut calls = ToolCallAccumulator::default();
    for delta in parse_sse(backend, reply).map_err(|e| e.to_string())? {
        match delta {
            StreamDelta::ToolCall {
                stream_key,
                id,
                name,
                arguments,
            } => calls.push_keyed(stream_key.as_deref(), &id, &name, &arguments),
            StreamDelta::ThinkingSignature { signature, .. } => {
                if !next_request.contains(&signature) {
                    return Err(format!("signature {signature} was not echoed"));
                }
            }
            StreamDelta::ReasoningDetail(detail) => {
                if let Some(signature) = detail["signature"].as_str() {
                    if !next_request.contains(signature) {
                        return Err(format!("signature {signature} was not echoed"));
                    }
                }
            }
            _ => {}
        }
    }
    for call in calls.finish() {
        // Once on the assistant's call, once on its result.
        if next_request.matches(&call.id).count() < 2 {
            return Err(format!("call {} was not echoed with its result", call.id));
        }
    }
    Ok(())
}

/// A cut-off reply says so, and is not read as finished.
fn prove_truncated(backend: Backend, sse: &str) -> Proven {
    let deltas = parse_sse(backend, sse).map_err(|e| e.to_string())?;
    if !deltas.iter().any(|d| matches!(d, StreamDelta::Truncated)) {
        return Err(format!("no Truncated in {deltas:?}"));
    }
    let stated = stated_usage(backend, sse);
    if stated.output_tokens == 0 {
        return Err("a cut-off reply billed no output".into());
    }
    Ok(())
}

// ---- the replays ----

/// The byte after the first frame `wanted` finds something in: a stream cut
/// off there has begun its answer and not finished it.
fn cut_after(backend: Backend, sse: &str, wanted: impl Fn(&StreamDelta) -> bool) -> usize {
    let mut at = 0;
    for frame in sse.split_inclusive("\n\n") {
        at += frame.len();
        let (deltas, terminal) = parse_blocks(backend, frame).unwrap();
        assert!(!terminal, "nothing to cut before the end: {sse}");
        if deltas.iter().any(&wanted) {
            return at;
        }
    }
    panic!("nothing to cut after in {sse}");
}

fn mid_call(backend: Backend, sse: &str) -> usize {
    cut_after(
        backend,
        sse,
        |d| matches!(d, StreamDelta::ToolCall { arguments, .. } if !arguments.is_empty()),
    )
}

fn mid_text(backend: Backend, sse: &str) -> usize {
    cut_after(backend, sse, |d| matches!(d, StreamDelta::Text(_)))
}

async fn replay_round_trip(c: &Capture) -> Proven {
    let backend = c.backend();
    let (first, second) = (c.reply("tool_call"), c.reply("after_tool"));
    let up = serve(vec![
        (first.to_string(), End::Whole),
        (second.to_string(), End::Whole),
    ]);
    let (_home, _cwd, mut agent) = agent_on(
        &c.meta.connection,
        &c.meta.model,
        Some(&up.base),
        "replay".into(),
        Default::default(),
    );
    let done = agent.turn(ROUND_TRIP).await.map_err(|e| e.to_string())?;
    prove_calls(&agent, &done.text, &["notes.txt"], &["7431"])?;
    prove_bill(&agent, backend, &[first, second])?;
    let seen = up.seen.lock().unwrap();
    prove_echo(backend, first, &seen[1])
}

async fn replay_parallel(c: &Capture) -> Proven {
    let backend = c.backend();
    let (first, second) = (c.reply("parallel"), c.reply("after_parallel"));
    let up = serve(vec![
        (first.to_string(), End::Whole),
        (second.to_string(), End::Whole),
    ]);
    let (_home, _cwd, mut agent) = agent_on(
        &c.meta.connection,
        &c.meta.model,
        Some(&up.base),
        "replay".into(),
        Default::default(),
    );
    let done = agent.turn(PARALLEL).await.map_err(|e| e.to_string())?;
    prove_calls(&agent, &done.text, &["a.txt", "b.txt"], &["2718", "3141"])?;
    prove_bill(&agent, backend, &[first, second])?;
    let seen = up.seen.lock().unwrap();
    prove_echo(backend, first, &seen[1])
}

async fn replay_truncation(c: &Capture) -> Proven {
    let sse = c.reply("truncated");
    prove_truncated(c.backend(), sse)?;
    let up = serve(vec![(sse.to_string(), End::Whole)]);
    let provider = provider_on(
        &c.meta.connection,
        Some(&up.base),
        "replay".into(),
        Default::default(),
    );
    let mut stream = provider
        .stream(long_request(&c.meta.model))
        .await
        .map_err(|e| e.to_string())?;
    let mut cut = false;
    while let Some(d) = stream.next().await {
        cut |= matches!(d.map_err(|e| e.to_string())?, StreamDelta::Truncated);
    }
    cut.then_some(())
        .ok_or_else(|| "the client missed the cut-off".to_string())
}

/// Nothing of an interrupted reply reaches the conversation: no half-made
/// call runs, and the next request would not carry one.
fn prove_nothing_ran(agent: &Agent) -> Proven {
    let transcript = &agent.session.transcript;
    if let Some(m) = transcript
        .iter()
        .find(|m| m.tool_calls.is_some() || m.role == "tool")
    {
        return Err(format!("an interrupted call got through: {m:?}"));
    }
    Ok(())
}

async fn replay_cancellation(c: &Capture) -> Proven {
    let backend = c.backend();
    for (name, at) in [
        ("tool_call", mid_call(backend, c.reply("tool_call"))),
        ("after_tool", mid_text(backend, c.reply("after_tool"))),
    ] {
        let cancel = crate::Cancel::new();
        let up = serve(vec![(
            c.reply(name).to_string(),
            End::Cancelled(at, cancel.clone()),
        )]);
        let (_home, _cwd, mut agent) = agent_on(
            &c.meta.connection,
            &c.meta.model,
            Some(&up.base),
            "replay".into(),
            Default::default(),
        );
        agent.ctx.cancel = cancel;
        let done = agent.turn(ROUND_TRIP).await.map_err(|e| e.to_string())?;
        if !matches!(done.reason, StopReason::Cancelled) {
            return Err(format!("{name}: stopped as {:?}", done.reason));
        }
        prove_nothing_ran(&agent).map_err(|e| format!("{name}: {e}"))?;
    }
    Ok(())
}

async fn replay_dropped_stream(c: &Capture) -> Proven {
    let backend = c.backend();
    for (name, at) in [
        ("tool_call", mid_call(backend, c.reply("tool_call"))),
        ("after_tool", mid_text(backend, c.reply("after_tool"))),
    ] {
        for end in [End::Closed(at), End::Broken(at)] {
            let how = if matches!(end, End::Closed(_)) {
                "closed"
            } else {
                "broken"
            };
            let up = serve(vec![(c.reply(name).to_string(), end)]);
            let (_home, _cwd, mut agent) = agent_on(
                &c.meta.connection,
                &c.meta.model,
                Some(&up.base),
                "replay".into(),
                Default::default(),
            );
            let Err(error) = agent.turn(ROUND_TRIP).await else {
                return Err(format!(
                    "{name} {how}: a stream that stopped short finished the turn"
                ));
            };
            if !matches!(error, Error::Provider(_)) {
                return Err(format!("{name} {how}: {error}"));
            }
            prove_nothing_ran(&agent).map_err(|e| format!("{name} {how}: {e}"))?;
            let rows = agent.session.spend_log().map_err(|e| e.to_string())?;
            if rows.iter().any(|r| !r.incomplete) {
                return Err(format!("{name} {how}: the bill reads as whole: {rows:?}"));
            }
        }
    }
    Ok(())
}

/// Every replay of one capture, each named on failure.
async fn replay(c: &Capture) {
    let name = &c.meta.connection;
    for (case, outcome) in [
        ("round trip", replay_round_trip(c).await),
        ("parallel calls", replay_parallel(c).await),
        ("truncation", replay_truncation(c).await),
        ("cancellation", replay_cancellation(c).await),
        ("dropped stream", replay_dropped_stream(c).await),
    ] {
        if let Err(e) = outcome {
            panic!("{name}: {case}: {e}");
        }
    }
}

// ---- the recording ----

fn long_request(model: &str) -> CompletionRequest {
    CompletionRequest {
        model: model.into(),
        system: None,
        messages: vec![Message::plain("user", LONG)],
        tools: Vec::new(),
        // OpenAI's lowest.
        max_tokens: Some(16),
        reasoning: None,
    }
}

/// What a recording spent and how the bill read, for the dated trail.
#[derive(Debug, Default)]
struct Proof {
    usd: f64,
    input: u64,
    cached: u64,
    output: u64,
    host: Option<String>,
}

impl Proof {
    fn count(&mut self, agent: &Agent) -> Proven {
        for row in agent.session.spend_log().map_err(|e| e.to_string())? {
            self.usd += row
                .total_usd
                .ok_or_else(|| format!("{} has no price: the budget cannot see it", row.model))?;
            self.input += row.input_tokens;
            self.cached += row.cached_tokens;
            self.output += row.output_tokens;
            self.host = row.host.or(self.host.take());
        }
        Ok(())
    }
}

/// Ids, signatures and opaque blobs are named by order of appearance, and the
/// copy of the prompt a Responses server sends back is dropped.
fn hide(v: &mut Value, names: &mut HashMap<String, String>) {
    const OPAQUE: [&str; 7] = [
        "id",
        "call_id",
        "item_id",
        "tool_use_id",
        "signature",
        "encrypted_content",
        "data",
    ];
    const ECHOED: [&str; 7] = [
        "instructions",
        "tools",
        "input",
        "metadata",
        "user",
        "safety_identifier",
        "prompt_cache_key",
    ];
    match v {
        Value::Array(items) => items.iter_mut().for_each(|i| hide(i, names)),
        Value::Object(map) => {
            if let Some(response) = map.get_mut("response").and_then(Value::as_object_mut) {
                ECHOED.iter().for_each(|k| drop(response.remove(*k)));
            }
            map.remove("obfuscation");
            map.remove("system_fingerprint");
            for (key, value) in map.iter_mut() {
                match value
                    .as_str()
                    .filter(|s| !s.is_empty() && OPAQUE.contains(&key.as_str()))
                {
                    Some(s) => {
                        let next = format!("wire_{}", names.len() + 1);
                        let name = names.entry(s.to_string()).or_insert(next).clone();
                        *value = json!(name);
                    }
                    None => hide(value, names),
                }
            }
        }
        _ => {}
    }
}

/// Something in `text` that must not reach a public repository.
fn leak(text: &str, key: &str) -> Option<String> {
    if !key.is_empty() && text.contains(key) {
        return Some("the key".into());
    }
    for mark in ["Bearer ", "/home/", "/Users/", "/tmp/", "/var/folders/"] {
        if text.contains(mark) {
            return Some(mark.into());
        }
    }
    for prefix in ["sk-", "xai-"] {
        let at_a_word =
            |&(i, _): &(usize, &str)| i == 0 || !text.as_bytes()[i - 1].is_ascii_alphanumeric();
        if text.match_indices(prefix).any(|m| at_a_word(&m)) {
            return Some(prefix.into());
        }
    }
    None
}

/// A raw reply as a capture fit to keep, or why it is not.
fn scrub(raw: &[u8], workspace: &Path, key: &str) -> Proven<String> {
    let text = String::from_utf8_lossy(raw).replace(&*workspace.to_string_lossy(), ".");
    let mut names = HashMap::new();
    let mut out = String::new();
    for line in text.lines() {
        let parsed = line
            .strip_prefix("data:")
            .and_then(|d| serde_json::from_str::<Value>(d.trim()).ok());
        match parsed {
            Some(mut v) => {
                hide(&mut v, &mut names);
                out.push_str(&format!("data: {v}"));
            }
            None => out.push_str(line),
        }
        out.push('\n');
    }
    match leak(&out, key) {
        Some(what) => Err(format!("refused to keep a capture holding {what}")),
        None => Ok(out),
    }
}

/// Each round of a turn in a line: its calls, or what it said.
fn what_it_did(agent: &Agent) -> String {
    let rounds: Vec<String> = agent
        .session
        .transcript
        .iter()
        .filter(|m| m.role == "assistant")
        .map(|m| match &m.tool_calls {
            Some(calls) => calls
                .iter()
                .map(|c| format!("{}({})", c.name, c.arguments))
                .collect::<Vec<_>>()
                .join(" + "),
            None => format!("said {:?}", m.content),
        })
        .collect();
    rounds.join(" -> ")
}

/// A failed try's replies, scrubbed, and evidence of nothing but that try.
fn keep_failed(dir: &Path, name: &str, attempt: u32, replies: &[String]) {
    let _ = std::fs::create_dir_all(dir);
    for (i, sse) in replies.iter().enumerate() {
        let _ = std::fs::write(dir.join(format!("{name}-{attempt}-{}.sse", i + 1)), sse);
    }
}

/// One tool round trip, one parallel round and one truncation on the real
/// agent and client, kept as a capture. Stops short of a reply that would
/// start past `budget` dollars.
async fn record(
    connection: &str,
    model: &str,
    base: Option<&str>,
    key: &str,
    budget: f64,
    failed: &Path,
) -> Proven<(Capture, Proof)> {
    let mut proof = Proof::default();
    let mut replies = HashMap::new();
    let backend = backend_for(&crate::Config::default().connections[connection]);

    for (prompt, names, paths, numbers) in [
        (
            ROUND_TRIP,
            ["tool_call", "after_tool"],
            vec!["notes.txt"],
            vec!["7431"],
        ),
        (
            PARALLEL,
            ["parallel", "after_parallel"],
            vec!["a.txt", "b.txt"],
            vec!["2718", "3141"],
        ),
    ] {
        let mut attempt = 0;
        let (first, second) = loop {
            attempt += 1;
            let tap = Tap::default();
            let (_home, cwd, mut agent) =
                agent_on(connection, model, base, key.into(), tap.clone());
            agent.budget_usd = (budget - proof.usd).max(0.0);
            let done = agent.turn(prompt).await.map_err(|e| e.to_string())?;
            proof.count(&agent)?;
            let kept = tap
                .lock()
                .unwrap()
                .iter()
                .map(|raw| scrub(raw, cwd.path(), key))
                .collect::<Proven<Vec<_>>>()?;
            let verdict = match kept.as_slice() {
                [first, second] => prove_calls(&agent, &done.text, &paths, &numbers)
                    .map(|()| (first.clone(), second.clone())),
                other => Err(format!(
                    "{} replies, wanted a call and an answer",
                    other.len()
                )),
            };
            match verdict {
                Ok(pair) => break pair,
                // A model may look around first. That is the model's right
                // and no fault of the wire, so it gets one more try, and what
                // it did stays where it can be read without paying again.
                Err(why) => {
                    let why = format!("{}: {why}. It did: {}", names[0], what_it_did(&agent));
                    println!("{connection}: {why}");
                    keep_failed(failed, names[0], attempt, &kept);
                    if attempt == 2 || proof.usd >= budget {
                        return Err(why);
                    }
                }
            }
        };
        replies.insert(names[0].to_string(), first);
        replies.insert(names[1].to_string(), second);
    }

    if proof.usd >= budget {
        return Err(format!("${:.4} spent, past the budget", proof.usd));
    }
    let tap = Tap::default();
    let provider = provider_on(connection, base, key.into(), tap.clone());
    let mut stream = provider
        .stream(long_request(model))
        .await
        .map_err(|e| e.to_string())?;
    while let Some(d) = stream.next().await {
        d.map_err(|e| e.to_string())?;
    }
    drop(stream);
    let raw = tap.lock().unwrap().concat();
    let truncated = scrub(&raw, Path::new("/nowhere"), key)?;
    prove_truncated(backend, &truncated).map_err(|e| format!("truncated: {e}"))?;
    let cut = stated_usage(backend, &truncated);
    let book = PriceBook::new();
    proof.usd += parse_sse(backend, &truncated)
        .map_err(|e| e.to_string())?
        .iter()
        .find_map(|d| match d {
            StreamDelta::ReportedCost(c) => Some(*c),
            _ => None,
        })
        .or_else(|| book.cost(model, cut))
        .unwrap_or(0.0);
    proof.input += cut.input_tokens;
    proof.cached += cut.cached_tokens;
    proof.output += cut.output_tokens;
    replies.insert("truncated".to_string(), truncated);

    let commit = crate::git::git(
        Path::new(env!("CARGO_MANIFEST_DIR")),
        &["rev-parse", "--short", "HEAD"],
    )
    .unwrap_or_default();
    Ok((
        Capture {
            meta: Meta {
                connection: connection.into(),
                model: model.into(),
                captured: crate::clock::today(),
                ryter: env!("CARGO_PKG_VERSION").into(),
                commit: commit.trim().into(),
            },
            replies,
        },
        proof,
    ))
}

/// `name=value,name=value` from the environment.
fn listed(var: &str) -> HashMap<String, String> {
    std::env::var(var)
        .unwrap_or_default()
        .split(',')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

/// `docs/provider-proof.md`'s row for a run that held.
fn trail_row(c: &Capture, proof: &Proof) -> String {
    format!(
        "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | ${:.4} |\n",
        c.meta.captured,
        c.meta.ryter,
        c.meta.commit,
        c.meta.connection,
        c.meta.model,
        proof.host.as_deref().unwrap_or("-"),
        REPLIES.len(),
        proof.input,
        proof.cached,
        proof.output,
        proof.usd,
    )
}

/// Spends real money on the keys Ryter already holds. See
/// `docs/provider-proof.md`.
#[tokio::test]
#[ignore = "spends money: RYTER_LIVE=1, see docs/provider-proof.md"]
async fn live_tool_round_trip() {
    if std::env::var("RYTER_LIVE").as_deref() != Ok("1") {
        return;
    }
    let cfg = crate::Config::default();
    let models = listed("RYTER_LIVE_MODELS");
    let budget: f64 = std::env::var("RYTER_LIVE_BUDGET")
        .ok()
        .and_then(|b| b.parse().ok())
        .unwrap_or(0.50);
    let only = std::env::var("RYTER_LIVE_ONLY").unwrap_or_default();
    let trail = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/provider-proof.md");
    let mut failures = Vec::new();
    for name in BUILT_IN {
        if !only.is_empty() && !only.split(',').any(|o| o.trim() == name) {
            continue;
        }
        let connection = crate::ids::ConnectionId::new(name);
        let Ok(key) = crate::config::resolve_secret(&cfg, &connection) else {
            println!("{name}: no key, not run");
            continue;
        };
        let default = cfg.connections[name].default_model.clone();
        let Some(model) = models.get(name).cloned().or(default) else {
            println!("{name}: no model, not run");
            continue;
        };
        println!("{name}: {model}, up to ${budget:.2}");
        let failed = target_wire().join(name).join("failed");
        let _ = std::fs::remove_dir_all(&failed);
        match record(name, &model, None, &key, budget, &failed).await {
            Ok((capture, proof)) => {
                let dir = target_wire().join(name);
                capture.save(&dir).unwrap();
                replay(&Capture::load(&dir)).await;
                capture.save(&wire_dir().join(name)).unwrap();
                let row = trail_row(&capture, &proof);
                let mut file = std::fs::OpenOptions::new()
                    .append(true)
                    .open(&trail)
                    .unwrap();
                file.write_all(row.as_bytes()).unwrap();
                println!("{name}: held, ${:.4}", proof.usd);
                let reasoned = parse_sse(capture.backend(), capture.reply("tool_call"))
                    .unwrap()
                    .iter()
                    .any(|d| {
                        matches!(
                            d,
                            StreamDelta::ThinkingSignature { .. } | StreamDelta::ReasoningDetail(_)
                        )
                    });
                println!("{name}: reasoning to echo: {reasoned}");
            }
            Err(e) => {
                println!("{name}: FAILED: {e}");
                failures.push(format!("{name}: {e}"));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

// ---- what is kept ----

#[tokio::test]
async fn recorded_captures_replay() {
    for name in RECORDED {
        replay(&Capture::load(&wire_dir().join(name))).await;
    }
}

#[test]
fn every_kept_capture_is_on_the_list_and_holds_nothing_private() {
    let mut kept: Vec<String> = std::fs::read_dir(wire_dir())
        .map(|dir| {
            dir.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    kept.sort();
    let mut listed: Vec<String> = RECORDED.iter().map(|n| n.to_string()).collect();
    listed.sort();
    assert_eq!(
        kept, listed,
        "a kept capture nothing replays, or the reverse"
    );
    for name in RECORDED {
        let capture = Capture::load(&wire_dir().join(name));
        assert!(BUILT_IN.contains(&capture.meta.connection.as_str()));
        for (reply, sse) in &capture.replies {
            assert_eq!(leak(sse, ""), None, "{name}/{reply}");
        }
        // Handing a signature back is what a Claude round trip is there to
        // prove; a capture where the model did not think proves nothing of it.
        if capture.backend() == Backend::Messages || capture.meta.model.starts_with("anthropic/") {
            let thought = parse_sse(capture.backend(), capture.reply("tool_call"))
                .unwrap()
                .iter()
                .any(|d| match d {
                    StreamDelta::ThinkingSignature { .. } => true,
                    StreamDelta::ReasoningDetail(detail) => detail["signature"].is_string(),
                    _ => false,
                });
            assert!(thought, "{name}: no signature to echo; record it again");
        }
    }
}

// ---- the recorder against a stand-in ----

/// A stand-in for a provider, close to what each protocol sends and written by
/// hand. It proves the recorder and the replays against each other when no
/// key is in reach; it is never kept as a capture, and proves nothing about
/// any provider.
fn stand_in(backend: Backend) -> [String; 5] {
    let data = |v: Value| format!("data: {v}\n\n");
    let event = |v: Value| format!("event: {}\ndata: {v}\n\n", v["type"].as_str().unwrap());
    let reply = |calls: &[(&str, &str)],
                 text: &str,
                 cut: bool,
                 prompt: u64,
                 cached: u64|
     -> String {
        let spent = if cut { 16 } else { 21 };
        let halves: Vec<(String, String)> = calls
            .iter()
            .map(|(_, path)| {
                let args = json!({ "path": path }).to_string();
                let (a, b) = args.split_at(args.len() / 2);
                (a.to_string(), b.to_string())
            })
            .collect();
        let (one, two) = text.split_at(text.len() / 2);
        let mut out = String::new();
        match backend {
            Backend::ChatCompletions => {
                let chunk = |delta: Value, finish: Value| {
                    data(json!({
                        "id": "gen-1", "provider": "StandIn", "object": "chat.completion.chunk",
                        "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
                    }))
                };
                out += ": OPENROUTER PROCESSING\n\n";
                out += &chunk(json!({"role": "assistant", "content": ""}), Value::Null);
                if !calls.is_empty() {
                    let detail = |fields: Value| {
                        let mut detail = json!({"type": "reasoning.text", "index": 0});
                        for (key, value) in fields.as_object().unwrap() {
                            detail[key] = value.clone();
                        }
                        json!({ "reasoning_details": [detail] })
                    };
                    out += &chunk(detail(json!({"text": "Reading."})), Value::Null);
                    let signature = format!("sig-{prompt}");
                    out += &chunk(detail(json!({ "signature": signature })), Value::Null);
                }
                for t in [one, two].into_iter().filter(|t| !t.is_empty()) {
                    out += &chunk(json!({ "content": t }), Value::Null);
                }
                for (i, (id, _)) in calls.iter().enumerate() {
                    out += &chunk(
                        json!({"tool_calls": [{"index": i, "id": id, "type": "function",
                            "function": {"name": "read_file", "arguments": ""}}]}),
                        Value::Null,
                    );
                }
                for pick in [0, 1] {
                    for (i, half) in halves.iter().enumerate() {
                        let fragment = if pick == 0 { &half.0 } else { &half.1 };
                        out += &chunk(
                            json!({"tool_calls": [{"index": i, "function": {"arguments": fragment}}]}),
                            Value::Null,
                        );
                    }
                }
                let finish = match (cut, calls.is_empty()) {
                    (true, _) => "length",
                    (_, false) => "tool_calls",
                    _ => "stop",
                };
                out += &chunk(json!({}), json!(finish));
                out += &data(json!({
                    "id": "gen-1", "provider": "StandIn", "object": "chat.completion.chunk",
                    "choices": [],
                    "usage": {"prompt_tokens": prompt, "completion_tokens": spent,
                        "prompt_tokens_details": {"cached_tokens": cached}, "cost": 0.0012},
                }));
                out += "data: [DONE]\n\n";
            }
            Backend::Responses => {
                out += &event(json!({"type": "response.created", "response": {
                    "id": "resp_1", "status": "in_progress",
                    "instructions": "The prompt, echoed.", "tools": [], "input": []}}));
                for t in [one, two].into_iter().filter(|t| !t.is_empty()) {
                    out += &event(json!({"type": "response.output_text.delta",
                        "item_id": "msg_1", "output_index": 0, "delta": t}));
                }
                for (i, (id, _)) in calls.iter().enumerate() {
                    out += &event(
                        json!({"type": "response.output_item.added", "output_index": i,
                        "item": {"id": format!("fc_{i}"), "type": "function_call", "call_id": id,
                            "name": "read_file", "arguments": ""}}),
                    );
                }
                for pick in [0, 1] {
                    for (i, half) in halves.iter().enumerate() {
                        let fragment = if pick == 0 { &half.0 } else { &half.1 };
                        out += &event(json!({"type": "response.function_call_arguments.delta",
                            "item_id": format!("fc_{i}"), "output_index": i, "delta": fragment}));
                    }
                }
                let usage = json!({"input_tokens": prompt, "output_tokens": spent,
                    "input_tokens_details": {"cached_tokens": cached}});
                out += &if cut {
                    event(json!({"type": "response.incomplete", "response": {
                        "id": "resp_1", "status": "incomplete", "usage": usage,
                        "incomplete_details": {"reason": "max_output_tokens"}}}))
                } else {
                    event(json!({"type": "response.completed", "response": {
                        "id": "resp_1", "status": "completed", "usage": usage}}))
                };
            }
            Backend::Messages => {
                out += &event(json!({"type": "message_start", "message": {"id": "msg_1",
                    "usage": {"input_tokens": prompt - cached, "cache_read_input_tokens": cached,
                        "cache_creation_input_tokens": 0, "output_tokens": 1}}}));
                let mut index = 0;
                if !cut {
                    out += &event(json!({"type": "content_block_start", "index": index,
                        "content_block": {"type": "thinking", "thinking": "", "signature": ""}}));
                    out += &event(json!({"type": "content_block_delta", "index": index,
                        "delta": {"type": "thinking_delta", "thinking": "Reading."}}));
                    out += &event(json!({"type": "content_block_delta", "index": index,
                        "delta": {"type": "signature_delta", "signature": format!("sig-{prompt}")}}));
                    out += &event(json!({"type": "content_block_stop", "index": index}));
                    index += 1;
                }
                if !text.is_empty() {
                    out += &event(json!({"type": "content_block_start", "index": index,
                        "content_block": {"type": "text", "text": ""}}));
                    for t in [one, two] {
                        out += &event(json!({"type": "content_block_delta", "index": index,
                            "delta": {"type": "text_delta", "text": t}}));
                    }
                    out += &event(json!({"type": "content_block_stop", "index": index}));
                    index += 1;
                }
                for (i, (id, _)) in calls.iter().enumerate() {
                    out += &event(json!({"type": "content_block_start", "index": index + i,
                        "content_block": {"type": "tool_use", "id": id, "name": "read_file",
                            "input": {}}}));
                }
                for pick in [0, 1] {
                    for (i, half) in halves.iter().enumerate() {
                        let fragment = if pick == 0 { &half.0 } else { &half.1 };
                        out += &event(json!({"type": "content_block_delta", "index": index + i,
                            "delta": {"type": "input_json_delta", "partial_json": fragment}}));
                    }
                }
                for i in 0..calls.len() {
                    out += &event(json!({"type": "content_block_stop", "index": index + i}));
                }
                let stop = match (cut, calls.is_empty()) {
                    (true, _) => "max_tokens",
                    (_, false) => "tool_use",
                    _ => "end_turn",
                };
                out += &event(
                    json!({"type": "message_delta", "delta": {"stop_reason": stop},
                    "usage": {"input_tokens": prompt - cached, "cache_read_input_tokens": cached,
                        "cache_creation_input_tokens": 0, "output_tokens": spent}}),
                );
                out += &event(json!({"type": "message_stop"}));
            }
        }
        out
    };
    [
        reply(&[("call_a", "notes.txt")], "", false, 900, 0),
        reply(&[], "The number is 7431 — that’s it.", false, 1000, 800),
        reply(
            &[("call_b", "a.txt"), ("call_c", "b.txt")],
            "",
            false,
            920,
            0,
        ),
        reply(&[], "They are 2718 and 3141.", false, 1100, 800),
        reply(&[], "1\n2\n3\n4\n5\n", true, 40, 0),
    ]
}

async fn recorded_from_a_stand_in(connection: &str, model: &str, key: &str) -> Capture {
    let backend = backend_for(&crate::Config::default().connections[connection]);
    let replies = stand_in(backend)
        .into_iter()
        .map(|sse| (sse, End::Whole))
        .collect();
    let up = serve(replies);
    let failed = TempDir::new().unwrap();
    let (capture, proof) = record(connection, model, Some(&up.base), key, 1.0, failed.path())
        .await
        .unwrap();
    assert!(proof.usd > 0.0, "{proof:?}");
    capture
}

/// A model that looks around first is not a broken wire: it is tried again,
/// and the capture is the try that went as asked.
#[tokio::test]
async fn a_model_that_went_another_way_is_tried_once_more() {
    let backend = backend_for(&crate::Config::default().connections["openrouter"]);
    let [call, answer, parallel, after_parallel, truncated] = stand_in(backend);
    let replies = [
        parallel.clone(),
        call.clone(),
        answer.clone(),
        call.clone(),
        answer.clone(),
        parallel.clone(),
        after_parallel,
        truncated,
    ];
    let up = serve(replies.into_iter().map(|sse| (sse, End::Whole)).collect());
    let failed = TempDir::new().unwrap();
    let (capture, proof) = record(
        "openrouter",
        "anthropic/claude-sonnet-4.6",
        Some(&up.base),
        "not-a-real-key",
        1.0,
        &failed.path().join("failed"),
    )
    .await
    .unwrap();
    assert!(failed.path().join("failed/tool_call-1-3.sse").exists());
    assert_eq!(up.seen.lock().unwrap().len(), 8);
    // What the failed try spent still counts against the budget.
    assert!(proof.usd > 0.0072, "{proof:?}");
    replay(&capture).await;
}

#[tokio::test]
async fn what_the_recorder_keeps_replays_on_every_wire() {
    for (connection, model) in [
        ("spacexsi", "grok-4.6"),
        ("openrouter", "anthropic/claude-sonnet-4.6"),
        ("anthropic", "claude-opus-5-5"),
        ("openai", "gpt-6.1-sol"),
    ] {
        let dir = TempDir::new().unwrap();
        let capture = recorded_from_a_stand_in(connection, model, "not-a-real-key").await;
        capture.save(dir.path()).unwrap();
        replay(&Capture::load(dir.path())).await;
    }
}

/// The other side of a dropped stream: a server may close the socket without
/// `[DONE]` or `message_stop` once it has said the reply is over.
#[tokio::test]
async fn a_stream_that_said_it_was_over_needs_no_closing_event() {
    for (connection, model, closing) in [
        ("openrouter", "anthropic/claude-sonnet-4.6", "data: [DONE]"),
        ("anthropic", "claude-opus-5-5", "event: message_stop"),
    ] {
        let backend = backend_for(&crate::Config::default().connections[connection]);
        let [_, answer, ..] = stand_in(backend);
        let at = answer.find(closing).unwrap();
        let up = serve(vec![(answer.clone(), End::Closed(at))]);
        let provider = provider_on(connection, Some(&up.base), "k".into(), Default::default());
        let mut stream = provider.stream(long_request(model)).await.unwrap();
        let mut text = String::new();
        while let Some(d) = stream.next().await {
            if let StreamDelta::Text(t) = d.unwrap() {
                text.push_str(&t);
            }
        }
        assert!(text.contains("7431"), "{connection}: {text:?}");
    }
}

#[test]
fn a_capture_keeps_no_identity_and_no_echoed_prompt() {
    let raw = concat!(
        "event: response.created\n",
        r#"data: {"type":"response.created","response":{"id":"resp_real","instructions":"secret prompt","tools":[{"name":"x"}],"user":"me"}}"#,
        "\n\n",
        r#"data: {"type":"response.output_item.added","item":{"id":"fc_real","call_id":"call_real","arguments":"{\"path\":\"/w/notes.txt\"}"}}"#,
        "\n\n",
        r#"data: {"type":"content_block_delta","delta":{"signature":"sig_real"}}"#,
        "\n\n",
        r#"data: {"again":"call_real","call_id":"call_real"}"#,
        "\n\ndata: [DONE]\n\n",
    );
    let kept = scrub(raw.as_bytes(), Path::new("/w"), "the-key").unwrap();
    for gone in [
        "secret prompt",
        "resp_real",
        "fc_real",
        "sig_real",
        "\"me\"",
    ] {
        assert!(!kept.contains(gone), "{gone} in {kept}");
    }
    assert!(kept.contains("./notes.txt"), "{kept}");
    assert!(kept.contains("data: [DONE]"), "{kept}");
    // One id keeps one name, or a result stops matching its call.
    let name = |line: &str| -> String {
        let v: Value = serde_json::from_str(line.strip_prefix("data: ").unwrap()).unwrap();
        v["item"]["call_id"]
            .as_str()
            .or(v["call_id"].as_str())
            .unwrap_or("")
            .to_string()
    };
    let lines: Vec<&str> = kept.lines().filter(|l| l.contains("wire_")).collect();
    assert_eq!(name(lines[1]), name(lines[3]));
}

/// Anthropic opens a thinking block with an empty signature; named, the
/// blank would read as a signature of its own.
#[test]
fn a_blank_stays_blank() {
    let raw = r#"data: {"content_block":{"type":"thinking","signature":""}}"#;
    let kept = scrub(raw.as_bytes(), Path::new("/w"), "the-key").unwrap();
    assert!(kept.contains(r#""signature":"""#), "{kept}");
}

#[test]
fn a_capture_holding_a_key_or_a_path_is_refused() {
    let with = |text: &str| format!("data: {{\"text\":\"{text}\"}}\n\n");
    assert!(scrub(with("the-key").as_bytes(), Path::new("/w"), "the-key").is_err());
    for private in ["Bearer abc", "/home/me/x", "/tmp/x", "sk-abc", "xai-abc"] {
        assert!(
            scrub(with(private).as_bytes(), Path::new("/w"), "the-key").is_err(),
            "{private}"
        );
    }
    // A word that only ends in a key's prefix is a word.
    assert!(scrub(with("a task-list").as_bytes(), Path::new("/w"), "the-key").is_ok());
}

#[test]
fn the_stated_bill_counts_anthropics_cache_as_part_of_the_prompt() {
    let [_, after, ..] = stand_in(Backend::Messages);
    let usage = stated_usage(Backend::Messages, &after);
    assert_eq!(
        (usage.input_tokens, usage.cached_tokens, usage.output_tokens),
        (1000, 800, 21)
    );
}
