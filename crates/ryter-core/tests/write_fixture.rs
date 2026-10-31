//! Writes what a release keeps on disk, through the release's own code, for
//! `tests/persisted.rs` to load for ever after. It uses only the public API
//! and compiles against a tagged release as well as today's code:
//!
//! ```sh
//! RYTER_FIXTURE_OUT=crates/ryter-core/fixtures/persisted/<version> \
//!     cargo test -p ryter-core --test write_fixture -- --ignored
//! ```
//!
//! Run at a release, and never rewrite a kept folder.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ryter_core::config::{self, Config, RoutingConfig};
use ryter_core::event::AgentEvent;
use ryter_core::llm::{AssistantToolCall, Message, MessagesBlock};
use ryter_core::role::Role;
use ryter_core::run::RunFile;
use ryter_core::session::{Session, TurnRecord, spend_record};
use ryter_core::spend::Usage;

/// The session's folder, as written in the session: nothing reads it.
const CWD: &str = "/work/fixture";

fn messages() -> Vec<Message> {
    let call = AssistantToolCall {
        id: "call_1".into(),
        name: "read_file".into(),
        arguments: r#"{"path":"notes.txt"}"#.into(),
    };
    let tool = Message {
        tool_call_id: Some("call_1".into()),
        ..Message::plain("tool", "   1|The kettle number is 7431.")
    };
    vec![
        Message::plain("user", "What is the number in notes.txt?"),
        Message {
            tool_calls: Some(vec![call]),
            messages_blocks: Some(vec![
                MessagesBlock::Thinking {
                    thinking: "It is in the file.".into(),
                    signature: "sig_1".into(),
                },
                MessagesBlock::ToolUse {
                    id: "call_1".into(),
                    name: "read_file".into(),
                    input: serde_json::json!({"path": "notes.txt"}),
                },
            ]),
            ..Message::plain("assistant", "")
        },
        tool,
        Message::plain("assistant", "The number is 7431."),
    ]
}

fn session(home: &Path) {
    let mut session = Session::create(
        home,
        Path::new(CWD),
        "anthropic".into(),
        "claude-opus-5-5".into(),
    )
    .unwrap();
    session.set_title("The kettle number").unwrap();
    for message in messages() {
        session.push_message(message).unwrap();
    }
    let usage = Usage {
        input_tokens: 900,
        output_tokens: 20,
        cached_tokens: 800,
        cache_write_tokens: 50,
    };
    let row =
        |model: &str, usd| spend_record("anthropic".into(), model.into(), Role::Build, usage, usd);
    session
        .record_spend(row("claude-opus-5-5", Some(0.25)))
        .unwrap();
    session.record_spend(row("no-price", None)).unwrap();
    let mut incomplete = row("claude-opus-5-5", Some(0.5));
    incomplete.incomplete = true;
    incomplete.scout = true;
    incomplete.host = Some("Host".into());
    session.record_spend(incomplete).unwrap();
    session
        .emit(&AgentEvent::Notice {
            message: "kept".into(),
        })
        .unwrap();
    session
        .emit(&AgentEvent::Token {
            text: "The number".into(),
        })
        .unwrap();
    session.meta.plan_file = Some(".ryter/plans/2026-10-30-kettle.md".into());
    session.meta.checkpoints.push("0123456789abcdef".into());
    session.meta.turn_checkpoint = Some("0123456789abcdef".into());
    session.meta.turn_records.insert(
        "0123456789abcdef".into(),
        TurnRecord {
            after: Some("fedcba9876543210".into()),
            ignored: Vec::new(),
        },
    );
    session.set_mode(Role::Plan).unwrap();
}

fn settings_and_routing(home: &Path) {
    let mut cfg = Config::default();
    cfg.spend.session_budget_usd = 5.0;
    cfg.limits.rounds = 40;
    cfg.sandbox.profile = "workspace".into();
    cfg.features.web = false;
    config::save_settings(home, &cfg).unwrap();
    let rules: RoutingConfig =
        toml::from_str("order = [\"baseten\"]\nallow_fallbacks = false\n").unwrap();
    config::save_routing(home, &BTreeMap::from([("openrouter".to_string(), rules)])).unwrap();
}

fn project(root: &Path) {
    std::fs::create_dir_all(root).unwrap();
    ryter_core::plan::save_on(root, "2026-10-30", "Kettle", "Read the number.\n").unwrap();
    let run = RunFile {
        start: Some("python3 -m http.server 8000".into()),
        ready: Some("http://localhost:8000/".into()),
        build: Some("make".into()),
        test: vec!["make test".into()],
        check: Some("make check".into()),
        ..RunFile::default()
    };
    std::fs::write(root.join(ryter_core::run::FILE), run.text()).unwrap();
}

#[test]
#[ignore = "writes a kept fixture: set RYTER_FIXTURE_OUT, see the top of this file"]
fn write_fixture() {
    let out = PathBuf::from(std::env::var("RYTER_FIXTURE_OUT").expect("RYTER_FIXTURE_OUT"));
    assert!(!out.exists(), "a kept fixture is never rewritten: {out:?}");
    let home = out.join("home");
    config::write_default_config(&home).unwrap();
    session(&home);
    settings_and_routing(&home);
    project(&out.join("project"));
    // What a run leaves that no release promises to read.
    let _ = std::fs::remove_dir_all(home.join("logs"));
    for entry in walk(&home) {
        if entry.file_name().is_some_and(|n| n == ".jsonl.lock") {
            std::fs::remove_file(entry).unwrap();
        }
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(walk(&path));
        } else {
            found.push(path);
        }
    }
    found
}
