//! Crate-level tests (§15): golden snapshots, scroll invariants, secret
//! redaction, help ↔ keymap parity, and the redraw budget.
//!
//! Snapshots live in `crates/ryter-tui/snapshots/`. Regenerate with
//! `UPDATE_SNAPSHOTS=1 cargo test -p ryter-tui`.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ryter_core::{AgentEvent, Role};

use crate::action::{Action, PanelId, SessionsMode};
use crate::activity::Mode as ActivityMode;
use crate::chat::MessageKind;
use crate::draw::{render_buffer, render_to_string, render_with_theme};
use crate::keymap::{self, Ctx};
use crate::panel::modal::PermissionModal;
use crate::panel::{self, PanelEnv};
use crate::theme::{ColorMode, Theme};
use crate::view::View;

/// Sizes from `R-TEST-01`.
const SIZES: [(u16, u16); 4] = [(80, 24), (100, 30), (120, 40), (160, 50)];

fn snapshot_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("snapshots")
}

/// Compare (or, with `UPDATE_SNAPSHOTS=1`, rewrite) a golden file.
fn check_snapshot(name: &str, actual: &str) {
    let path = snapshot_dir().join(format!("{name}.txt"));
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(snapshot_dir()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "missing snapshot {}: {e} (run with UPDATE_SNAPSHOTS=1)",
            path.display()
        )
    });
    if expected != actual {
        let diff: Vec<String> = expected
            .lines()
            .zip(actual.lines())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .take(6)
            .map(|(i, (a, b))| format!("line {i}:\n  expected: {a}\n  actual:   {b}"))
            .collect();
        panic!(
            "snapshot {name} differs ({} vs {} lines)\n{}",
            expected.lines().count(),
            actual.lines().count(),
            diff.join("\n")
        );
    }
}

fn env() -> PanelEnv {
    let home = std::env::temp_dir().join(format!("ryter-tui-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&home);
    PanelEnv {
        home,
        cwd: PathBuf::from("/tmp/proj"),
        trusted: true,
        sandbox: ryter_core::sandbox::SandboxProfile::Off,
    }
}

/// Deterministic idle session: no clock, fixed username, one connection.
fn idle() -> View {
    let mut v = View::new(
        "spacexai".into(),
        "grok-4.6".into(),
        "~/workspace/ryter".into(),
    );
    v.ui.timestamps = false;
    v.username = "dusty".into();
    v.git_branch = Some("0.2.0-patch".into());
    v.session_id = "0193abcd-ef01-7000-8000-000000000001".into();
    v.has_key = true;
    v.ctx_tokens = Some(12_400);
    v.ctx_window = Some(256_000);
    v.ctx_pct = Some(5);
    v.price_label = "$2/M in · $6/M out".into();
    v.connections.push(crate::view::ConnRow {
        name: "spacexai".into(),
        kind: "spacexai".into(),
        model: "grok-4.6".into(),
        has_key: true,
    });
    v.connections.push(crate::view::ConnRow {
        name: "openrouter".into(),
        kind: "openrouter".into(),
        model: "anthropic/claude-sonnet-4.6".into(),
        has_key: false,
    });
    v.system("welcome · /help for keys");
    v
}

/// A session with one finished turn and a streaming second one.
fn mid_stream(reasoning: ActivityMode) -> View {
    let mut v = idle();
    v.activity = crate::activity::Activity::new(reasoning);
    let _ = v.submit_user("summarise the plan".into(), "summarise the plan".into());
    v.on_token("Here is the **plan**:\n\n1. wire the loop\n2. write tests\n");
    crate::run_events_apply(
        &mut v,
        AgentEvent::TurnFinished {
            turn: 1,
            tools: 0,
            duration_ms: 4200,
        },
    );
    let _ = v.submit_user(
        "now implement step one".into(),
        "now implement step one".into(),
    );
    crate::run_events_apply(
        &mut v,
        AgentEvent::Reasoning {
            text: "The user wants the loop wired. I should read run.rs first, then ".repeat(3),
        },
    );
    crate::run_events_apply(
        &mut v,
        AgentEvent::ToolCall {
            id: "t1".into(),
            name: "read_file".into(),
            args: serde_json::json!({"path": "crates/ryter-tui/src/run/mod.rs"}),
            role: Role::SoloBuild,
            summary: Some("read crates/ryter-tui/src/run/mod.rs".into()),
        },
    );
    crate::run_events_apply(
        &mut v,
        AgentEvent::ToolResult {
            id: "t1".into(),
            output: "…".into(),
            is_error: false,
            duration_ms: Some(120),
            diff: None,
        },
    );
    v.on_token("Reading the loop now. The event loop drains ");
    v.now_ms = 12_345;
    v.tick(12_345);
    v
}

fn with_panel(id: PanelId) -> View {
    let mut v = idle();
    let e = env();
    let _ = panel::open(&mut v, id, &e);
    panel::sync_composer(&mut v);
    v
}

fn all_sizes(name: &str, view: &View) {
    for (w, h) in SIZES {
        check_snapshot(&format!("{name}-{w}x{h}"), &render_to_string(view, w, h));
    }
}

// -- R-TEST-01 golden snapshots -------------------------------------------------

#[test]
fn snapshot_idle() {
    all_sizes("idle", &idle());
}

#[test]
fn snapshot_mid_stream_collapsed() {
    all_sizes("stream-collapsed", &mid_stream(ActivityMode::Collapsed));
}

#[test]
fn snapshot_mid_stream_expanded() {
    all_sizes("stream-expanded", &mid_stream(ActivityMode::Expanded));
}

#[test]
fn snapshot_palette_open() {
    let mut v = idle();
    v.composer.set_text("/mo");
    crate::palette::refresh(&mut v);
    assert!(v.palette.is_some());
    all_sizes("palette", &v);
}

#[test]
fn snapshot_every_panel() {
    let panels: [(&str, PanelId); 16] = [
        ("providers", PanelId::Providers),
        ("models", PanelId::Models),
        ("sessions", PanelId::Sessions(SessionsMode::Browse)),
        ("spend", PanelId::Spend),
        ("budget", PanelId::Budget),
        ("settings", PanelId::Settings),
        ("theme", PanelId::Theme),
        ("tools", PanelId::Tools),
        ("mcp", PanelId::Mcp),
        ("skills", PanelId::Skills),
        ("hooks", PanelId::Hooks),
        ("context", PanelId::Context),
        ("help", PanelId::Help),
        ("doctor", PanelId::Doctor),
        ("changes", PanelId::Changes),
        ("commit", PanelId::Commit),
    ];
    for (name, id) in panels {
        let v = with_panel(id);
        assert!(!v.panels.is_empty(), "{name} did not open");
        all_sizes(&format!("panel-{name}"), &v);
    }
}

/// Every panel opens at every width, beside the rail and without it.
/// `/models` asks for 124 columns; beside the rail, on a screen under 158
/// wide, it was drawn past the right edge and the program crashed.
#[test]
fn every_panel_fits_the_screen_at_every_width() {
    let panels = [
        PanelId::Providers,
        PanelId::Models,
        PanelId::Sessions(SessionsMode::Browse),
        PanelId::Spend,
        PanelId::Budget,
        PanelId::Settings,
        PanelId::Theme,
        PanelId::Tools,
        PanelId::Mcp,
        PanelId::Skills,
        PanelId::Rules,
        PanelId::Hooks,
        PanelId::Context,
        PanelId::Help,
        PanelId::Doctor,
        PanelId::Changes,
        PanelId::Commit,
    ];
    for id in panels {
        for (layout, rail) in [("ledger", true), ("ledger", false), ("classic", true)] {
            let mut v = with_panel(id);
            v.ui.layout = layout.into();
            v.panel_visible = rail;
            for width in (40..=200).step_by(6) {
                for height in [12, 24, 40] {
                    // Drawing off the screen panics.
                    let drawn = render_to_string(&v, width, height);
                    assert!(
                        drawn
                            .lines()
                            .all(|l| l.chars().count() <= usize::from(width)),
                        "{id:?} at {width}x{height}, {layout}, rail {rail}"
                    );
                }
            }
        }
    }
    // The case that crashed: its right border is on the screen.
    let mut v = with_panel(PanelId::Models);
    v.ui.layout = "ledger".into();
    v.panel_visible = true;
    let drawn = render_to_string(&v, 110, 32);
    let top = drawn.lines().find(|l| l.contains("models")).unwrap_or("");
    assert!(top.trim_end().ends_with('╮'), "{drawn}");
}

#[test]
fn snapshot_permission_modal() {
    let mut v = mid_stream(ActivityMode::Collapsed);
    v.panels.push(Box::new(PermissionModal::new(
        "bash".into(),
        "rm -rf target/ && cargo build --release".into(),
    )));
    all_sizes("modal-permission", &v);
}

/// An edit asks with the change it would make, not just its path.
#[test]
fn snapshot_edit_permission_shows_the_change() {
    let mut v = mid_stream(ActivityMode::Collapsed);
    let old = "const app = express()\n\napp.get('/search', (req, res) => {\n  const q = req.query.q\n  res.json(find(q))\n})\n";
    let new = old.replace(
        "  const q = req.query.q\n",
        "  const q = (req.query.q || '').trim()\n  if (!q) return res.json([])\n",
    );
    let diff = ryter_core::diff::FileDiff::new("app/server.js", Some(old), &new);
    v.panels.push(Box::new(
        PermissionModal::new("search_replace".into(), "app/server.js".into())
            .with_preview(Some(Box::new(diff))),
    ));
    all_sizes("modal-permission-edit", &v);
}

#[test]
fn snapshot_no_color_and_16_color() {
    let v = mid_stream(ActivityMode::Collapsed);
    let none = Theme::truecolor_dark().degrade(ColorMode::Mono);
    let sixteen = Theme::truecolor_dark().degrade(ColorMode::Ansi16);
    for (w, h) in SIZES {
        check_snapshot(
            &format!("nocolor-{w}x{h}"),
            &render_with_theme(&v, w, h, none),
        );
        check_snapshot(
            &format!("16color-{w}x{h}"),
            &render_with_theme(&v, w, h, sixteen),
        );
    }
    // `NO_COLOR` really means no color: every cell is Reset (`R-THEME-08`).
    let buf = render_buffer(&v, 100, 30, none);
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            let c = &buf[(x, y)];
            assert_eq!(c.fg, ratatui::style::Color::Reset, "fg at {x},{y}");
            assert_eq!(c.bg, ratatui::style::Color::Reset, "bg at {x},{y}");
        }
    }
    // 16-color mode never emits RGB or indexed colors.
    let buf = render_buffer(&v, 100, 30, sixteen);
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            let c = &buf[(x, y)];
            for col in [c.fg, c.bg] {
                assert!(
                    !matches!(
                        col,
                        ratatui::style::Color::Rgb(..) | ratatui::style::Color::Indexed(_)
                    ),
                    "{col:?} at {x},{y}"
                );
            }
        }
    }
}

// -- R-TEST-02 panels have borders and titles -----------------------------------

#[test]
fn panels_have_rounded_borders_and_titles() {
    for (name, id) in [
        ("providers", PanelId::Providers),
        ("settings", PanelId::Settings),
        ("help", PanelId::Help),
    ] {
        let v = with_panel(id);
        let s = render_to_string(&v, 120, 40);
        assert!(
            s.contains('╭') && s.contains('╯'),
            "{name}: no rounded corners\n{s}"
        );
        assert!(
            s.contains(&format!(" {} ", v.panels.top().unwrap().title(&v))) || s.contains(name),
            "{name}: title missing\n{s}"
        );
    }
    // The chat pane itself has no box (`R-CHROME-03`).
    let s = render_to_string(&idle(), 120, 40);
    let first_body_row = s.lines().nth(2).unwrap_or("");
    assert!(
        !first_body_row.starts_with('┌') && !first_body_row.starts_with('╭'),
        "{first_body_row}"
    );
}

// -- R-TEST-03 scroll -----------------------------------------------------------

fn many_turns(n: usize) -> View {
    let mut v = idle();
    for i in 1..=n {
        let _ = v.submit_user(format!("question {i}"), format!("question {i}"));
        v.on_token(&format!("answer {i}\n\nline a\nline b\nline c\n"));
        v.busy = false;
    }
    v
}

/// The screen says which hat gets the next message (its badge on the
/// composer). Never "orchestrator", a phase, or the crew.
#[test]
fn the_screen_shows_the_hat() {
    for mode in [
        ryter_core::Role::SoloBuild,
        ryter_core::Role::SoloPlan,
        ryter_core::Role::SoloReview,
    ] {
        for base in [idle(), mid_stream(ActivityMode::Collapsed)] {
            let mut view = base;
            view.mode = mode;
            for (w, h) in SIZES {
                let frame = render_to_string(&view, w, h);
                let badge = format!(" {} ", view.mode_label().to_ascii_uppercase());
                assert!(frame.contains(&badge), "{w}x{h} no {badge:?}:\n{frame}");
                for gone in ["orchestrator", "handoff", "phase", "crew", "lead"] {
                    assert!(!frame.contains(gone), "{w}x{h} shows {gone:?}:\n{frame}");
                }
            }
        }
    }
}

#[test]
fn hint_bar_never_drops_cancel_or_quit() {
    // At 80 columns the streaming bar used to truncate its tail, losing `^c
    // quit` exactly when a turn was running (G-05).
    let view = mid_stream(ActivityMode::Collapsed);
    for width in [80u16, 60, 48, 40, 32] {
        let frame = render_to_string(&view, width, 24);
        let hint = frame.lines().last().unwrap_or_default().to_string();
        assert!(hint.contains("^c"), "width {width} lost quit: {hint:?}");
        assert!(hint.contains("esc"), "width {width} lost cancel: {hint:?}");
        assert!(
            hint.chars().count() <= usize::from(width),
            "width {width} overflowed: {hint:?}"
        );
    }
}

#[test]
fn empty_cards_are_absent_not_blank() {
    // Principle 1: the conversation gets the space. An empty card used to
    // hold a column open to say nothing (G-03).
    let view = idle();
    for (w, h) in SIZES {
        let frame = render_to_string(&view, w, h);
        assert!(!frame.contains("no servers"), "{w}x{h}: empty mcp card");
    }
}

/// The raw session id is operator chrome; `/sessions` is where it belongs (G-07).
#[test]
fn session_card_leads_with_title_and_the_hat() {
    let view = idle();
    let frame = render_to_string(&view, 120, 40);
    let card_line = frame
        .lines()
        .find(|l| l.contains("untitled"))
        .unwrap_or_default()
        .to_string();
    assert!(
        card_line.contains("build"),
        "the mode shares the row: {card_line:?}"
    );
    assert!(
        !frame.contains("0193abcd"),
        "the uuid should not lead the column"
    );
}

/// An open panel owns the body: no sidebar card survives underneath it to be
/// sliced into fragments by its border (G-01, G-02, G-06).
#[test]
fn an_open_panel_leaves_no_card_fragments() {
    for id in [PanelId::Help, PanelId::Settings, PanelId::Providers] {
        let view = with_panel(id);
        for (w, h) in SIZES {
            let frame = render_to_string(&view, w, h);
            for needle in ["╭─ session", "╭─ model", "╭─ spend", "price unknown"] {
                assert!(
                    !frame.contains(needle),
                    "{id:?} at {w}x{h} still shows {needle:?}"
                );
            }
        }
    }
}

#[test]
fn composer_present_at_every_message_count() {
    for n in [0usize, 1, 5, 40, 200] {
        let v = many_turns(n);
        let s = render_to_string(&v, 100, 30);
        assert!(
            s.contains("what should change") || s.contains("you"),
            "n={n}\n{s}"
        );
        assert!(s.contains('›'), "prompt glyph missing at n={n}");
    }
}

#[test]
fn in_flight_message_or_sticky_header_always_visible() {
    let mut v = many_turns(3);
    let _ = v.submit_user("the live question".into(), "the live question".into());
    for i in 0..60 {
        v.on_token(&format!("streamed row {i}\n"));
        let s = render_to_string(&v, 100, 24);
        assert!(s.contains("the live question"), "row {i}\n{s}");
    }
}

#[test]
fn scroll_up_detaches_and_new_content_does_not_move_view() {
    let mut v = many_turns(30);
    let before = render_to_string(&v, 100, 24);
    v.scroll.page_up(false);
    assert!(!v.scroll.follow);
    let detached = render_to_string(&v, 100, 24);
    assert_ne!(before, detached);
    // Model output keeps streaming into the last message while detached.
    v.on_token("new content\n".repeat(10).as_str());
    let after = render_to_string(&v, 100, 24);
    let body = |s: &str| s.lines().skip(2).take(15).collect::<Vec<_>>().join("\n");
    assert_eq!(
        body(&detached),
        body(&after),
        "viewport moved while detached"
    );
    assert!(
        after.contains("↓") && after.contains("new"),
        "pill missing\n{after}"
    );
    v.scroll.to_bottom();
    assert!(v.scroll.follow);
    let bottom = render_to_string(&v, 100, 24);
    assert!(bottom.contains("new content"));
}

// -- R-TEST-08 help matches KEYMAP ----------------------------------------------

#[test]
fn help_lists_every_binding() {
    // The keys tab pages, so check parity one binding at a time through the
    // panel's own filter: every `KEYMAP` row is reachable with its context title.
    for b in keymap::KEYMAP {
        let mut v = with_panel(PanelId::Help);
        v.composer.set_text(b.help);
        let s = render_to_string(&v, 160, 50);
        assert!(
            s.contains(&keymap::display(b.key)),
            "{} ({:?}) not in /help\n{s}",
            b.key,
            b.ctx
        );
        assert!(s.contains(b.help), "{} help text missing\n{s}", b.key);
        assert!(
            s.to_ascii_lowercase().contains(b.ctx.title()),
            "{} shown under the wrong context\n{s}",
            b.key
        );
    }
    let _ = Ctx::ALL;
}

// -- R-TEST-09 secret redaction --------------------------------------------------

#[test]
fn secret_never_reaches_the_frame() {
    let mut v = idle();
    v.composer.begin_secret("spacexai".into());
    let secret = "sk-ZZtopSECRET-9f8e7d";
    for c in secret.chars() {
        v.composer.insert_char(c);
    }
    for (w, h) in SIZES {
        let s = render_to_string(&v, w, h);
        assert!(!s.contains(secret));
        for frag in ["ZZtop", "SECRET", "9f8e7d"] {
            assert!(!s.contains(frag), "{frag} leaked at {w}x{h}\n{s}");
        }
        assert!(s.contains('•'), "mask missing at {w}x{h}");
    }
    let dbg = format!("{v:?}");
    assert!(
        !dbg.contains("ZZtop") || dbg.contains("secret"),
        "Debug must not print the key in the clear"
    );
}

// -- R-TEST-11 redraw budget ------------------------------------------------------

#[test]
fn five_hundred_cached_messages_redraw_fast() {
    let mut v = idle();
    for i in 0..250 {
        let _ = v.submit_user(format!("q{i}"), format!("q{i}"));
        v.on_token(&format!(
            "**a{i}** with `code` and a [link](https://x.y/{i})\n\n```rust\nfn f{i}() {{}}\n```\n"
        ));
        v.busy = false;
    }
    assert_eq!(
        v.messages
            .iter()
            .filter(|m| matches!(m.kind, MessageKind::User | MessageKind::Assistant { .. }))
            .count(),
        500
    );
    // Warm the cache.
    let _ = render_to_string(&v, 120, 40);
    let started = std::time::Instant::now();
    for _ in 0..5 {
        let _ = render_to_string(&v, 120, 40);
    }
    let per_frame = started.elapsed() / 5;
    // 16 ms is the release target; debug builds get a generous multiple.
    let ceiling = if cfg!(debug_assertions) { 250 } else { 16 };
    assert!(
        per_frame.as_millis() < ceiling,
        "redraw took {per_frame:?} (ceiling {ceiling} ms)"
    );
}

// -- key routing through the real dispatcher ---------------------------------------

#[test]
fn f1_opens_help_and_esc_closes_it() {
    let mut v = idle();
    let a = crate::run_keys_handle(&mut v, KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
    assert_eq!(a, crate::action::Action::OpenPanel(PanelId::Help));
    let e = env();
    let _ = panel::open(&mut v, PanelId::Help, &e);
    assert!(!v.panels.is_empty());
    let _ = crate::run_keys_handle(&mut v, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(v.panels.is_empty());
}

/// A build turn that edits one file and creates another: the chat shows
/// each edit as numbered rows with the file's own line numbers.
fn edited() -> View {
    edited_on(idle())
}

fn edited_on(mut v: View) -> View {
    let _ = v.submit_user("trim the query".into(), "trim the query".into());
    let old = "import express from 'express'\nconst app = express()\n\napp.get('/search', (req, res) => {\n  const q = req.query.q\n  res.json(find(q))\n})\n\napp.listen(3000)\n";
    let new = old.replace(
        "  const q = req.query.q\n",
        "  const q = (req.query.q || '').trim()\n  if (!q) return res.json([])\n",
    );
    let edit = serde_json::json!({
        "path": "app/server.js",
        "old_string": "  const q = req.query.q\n",
        "new_string": "  const q = (req.query.q || '').trim()\n  if (!q) return res.json([])\n",
    });
    let steps = [
        ("e1", "search_replace", edit, "updated /tmp/proj/app/server.js · −1 +2 lines", Some(old), new),
        (
            "e2",
            "write",
            serde_json::json!({"path": "test/search.test.js", "content": "…"}),
            "created /tmp/proj/test/search.test.js · 6 lines",
            None,
            "import { test } from 'node:test'\nimport assert from 'node:assert'\n\ntest('an empty query finds nothing', async () => {\n  assert.deepEqual(await search(''), [])\n})\n".to_string(),
        ),
    ];
    for (id, name, args, out, before, after) in steps {
        crate::run_events_apply(
            &mut v,
            AgentEvent::ToolCall {
                id: id.into(),
                name: name.into(),
                args: args.clone(),
                role: Role::SoloBuild,
                summary: None,
            },
        );
        let path = args["path"].as_str().unwrap();
        crate::run_events_apply(
            &mut v,
            AgentEvent::ToolResult {
                id: id.into(),
                output: out.into(),
                is_error: false,
                duration_ms: Some(3),
                diff: Some(Box::new(ryter_core::diff::FileDiff::new(
                    path, before, &after,
                ))),
            },
        );
    }
    crate::run_events_apply(
        &mut v,
        AgentEvent::TurnFinished {
            turn: 1,
            tools: 2,
            duration_ms: 2100,
        },
    );
    v
}

#[test]
fn snapshot_edit_diff() {
    all_sizes("edit-diff", &edited());
}

/// The ledger (0.6.0's default layout): an answered turn folded to one line,
/// then an edit turn on the timeline, closed by what it came to.
fn ledger() -> View {
    let mut v = idle();
    v.ui.layout = "ledger".into();
    v.panel_visible = false;
    v.cwd = "~/workspace/muzak".into();
    v.git_branch = Some("main".into());
    v.spend = Some(0.012);
    let _ = v.submit_user(
        "why does load() ignore a missing file?".into(),
        "why does load() ignore a missing file?".into(),
    );
    crate::run_events_apply(
        &mut v,
        AgentEvent::TurnStarted {
            turn: 1,
            role: Role::SoloBuild,
        },
    );
    v.on_token("A first run has no config yet, so it falls back to the defaults.");
    v.spend = Some(0.014);
    crate::run_events_apply(
        &mut v,
        AgentEvent::TurnFinished {
            turn: 1,
            tools: 0,
            duration_ms: 1800,
        },
    );
    v.turn_spend_from = Some(0.014);
    v.spend = Some(0.018);
    edited_on(v)
}

/// Design S2: solo mode gets a rail with the name, the session, the hat
/// as a block in its color, the model, the spend and what the last turn
/// changed. The prompt is boxed in the hat's color, the keys on its edge.
fn with_rail() -> View {
    let mut v = ledger();
    v.panel_visible = true;
    v.session_title = "Show elapsed and total time on the progress bar".into();
    v.ctx_tokens = Some(24_000);
    v.ctx_window = Some(200_000);
    v
}

#[test]
fn the_rail_names_the_session_and_shows_the_hat_and_spend() {
    let v = with_rail();
    let text = render_to_string(&v, 140, 44);
    if std::env::var_os("SHOW").is_some() {
        println!("{text}");
    }
    for want in [
        "R Y T E R",
        "muzak · main",
        "SESSION",
        "Show elapsed and total time",
        "2 turns",
        " BUILD",
        "edits files, runs commands",
        "plan·review·test   tab switch",
        "MODEL",
        "24k of 200k tokens",
        "SPEND",
        "this turn",
        "session",
        "budget      off",
        "CHANGED",
        "app/server.js",
        "chat  changes ^t",
        "hide rail",
    ] {
        assert!(text.contains(want), "missing {want:?}:\n{text}");
    }
    // Neither the strip nor the bottom bar: the rail says what they said.
    assert!(!text.contains("crew board"), "{text}");
    assert!(!text.contains("│ turn "), "{text}");
    // The hat follows the mode.
    let mut v = with_rail();
    v.mode = Role::SoloPlan;
    let text = render_to_string(&v, 140, 44);
    assert!(
        text.contains(" PLAN") && text.contains("build·review·test"),
        "{text}"
    );
    v.mode = Role::SoloTest;
    let text = render_to_string(&v, 140, 44);
    for want in [
        " TEST",
        " uses the product",
        "plan·build·review   tab switch",
    ] {
        assert!(text.contains(want), "missing {want:?}:\n{text}");
    }
    // Narrow, or hidden with ^b: the ledger as it was.
    for v in [
        with_rail(),
        View {
            panel_visible: false,
            ..with_rail()
        },
    ] {
        let width = if v.panel_visible { 100 } else { 140 };
        let text = render_to_string(&v, width, 44);
        assert!(
            !text.contains("R Y T E R") && text.contains("chat     changes  ^t"),
            "{text}"
        );
    }
}

/// When the project cost is counted for a repository around the folder
/// (`~/workspace` holding `muzak`), the rail and the spend drawer name it:
/// a total that carries over from other folders explains itself.
#[test]
fn the_rail_names_the_folder_the_project_cost_is_counted_in() {
    let mut v = with_rail();
    let mut p = ryter_core::project::ProjectSpend::default();
    p.total_usd = 20.14;
    v.project_spend = Some(p);
    let text = render_to_string(&v, 140, 44);
    assert!(text.contains("project     $20.14"), "{text}");
    assert!(!text.contains("in ~/"), "the folder is the project: {text}");
    v.project_root = Some("~/workspace".into());
    let text = render_to_string(&v, 140, 44);
    assert!(text.contains("            in ~/workspace"), "{text}");
    // A long root keeps its end, where the folder's name is.
    v.project_root = Some("~/a/very/long/path/to/some/client/folder/workspace".into());
    let text = render_to_string(&v, 140, 44);
    assert!(text.contains("in …lder/workspace │"), "{text}");
    // The drawer's project column is named for that folder too.
    v.project_root = Some("~/workspace".into());
    let e = env();
    let _ = panel::open(&mut v, PanelId::SpendDrawer, &e);
    let text = render_to_string(&v, 120, 40);
    assert!(text.contains("project · workspace"), "{text}");
}

/// The root is noted only when it isn't the folder Ryter runs in.
#[test]
fn the_project_root_is_noted_only_for_a_folder_inside_a_repository() {
    let home = tempfile::TempDir::new().unwrap();
    let repo = tempfile::TempDir::new().unwrap();
    ryter_core::git::ensure_repo(repo.path()).unwrap();
    std::fs::create_dir_all(repo.path().join("app")).unwrap();
    let mut v = with_rail();
    crate::run::load_project_spend(&mut v, home.path(), repo.path());
    assert!(v.project_spend.is_some());
    assert_eq!(v.project_root, None, "Ryter runs at the root");
    crate::run::load_project_spend(&mut v, home.path(), &repo.path().join("app"));
    let name = repo
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(
        v.project_root
            .as_deref()
            .is_some_and(|r| r.ends_with(&name)),
        "{:?}",
        v.project_root
    );
}

/// The rail draws at every size without losing the prompt, and steps
/// aside below its width.
#[test]
fn the_rail_fits_every_size() {
    for (w, h) in [
        (60, 20),
        (80, 24),
        (100, 30),
        (110, 30),
        (140, 44),
        (220, 60),
    ] {
        let text = render_to_string(&with_rail(), w, h);
        assert_eq!(
            text.contains("R Y T E R"),
            w >= crate::rail::RAIL_MIN_SCREEN,
            "{w}x{h}:\n{text}"
        );
        assert!(text.contains("what should change?"), "{w}x{h}:\n{text}");
    }
}

#[test]
fn snapshot_ledger() {
    all_sizes("ledger", &ledger());
}

/// The model picker says where its list stands: from the cache while a
/// fresh one downloads, or that the provider is slow.
#[test]
fn the_model_picker_says_where_its_list_stands() {
    let mut v = edited();
    let p = crate::panel::models::Models::new(&mut v, None);
    v.panels.push(Box::new(p));
    crate::run_events_apply(
        &mut v,
        AgentEvent::ModelsNote {
            note: Some("from 2 h ago · refreshing".into()),
        },
    );
    let text = render_to_string(&v, 120, 30);
    assert!(text.contains("from 2 h ago · refreshing"), "{text}");
    crate::run_events_apply(&mut v, AgentEvent::ModelsNote { note: None });
    assert!(!render_to_string(&v, 120, 30).contains("refreshing"));
}

/// The commit's receipt says whether the review hat reviewed these files:
/// its verdict holds for the files it read, and for no others.
#[test]
fn the_commit_receipt_says_whether_this_work_was_reviewed() {
    use ryter_core::review;
    let repo = workbench_repo();
    let home = tempfile::tempdir().unwrap();
    let env = crate::panel::PanelEnv {
        home: home.path().to_path_buf(),
        cwd: repo.path().to_path_buf(),
        trusted: false,
        sandbox: ryter_core::sandbox::SandboxProfile::Off,
    };
    let receipt = |v: &View| crate::panel::commit::Commit::new(v, &env).0.receipt_line();
    let mut v = idle();
    assert!(receipt(&v).contains("· not reviewed"), "{}", receipt(&v));
    // Reviewed as the files are now.
    let base = review::head_base(repo.path());
    let now = review::changes(repo.path(), &base).unwrap().now;
    let tree = review::tree_of(repo.path(), &now);
    assert!(tree.is_some());
    v.last_review = Some((tree.clone(), "x-ai/grok-4.7".into(), Some(true)));
    assert!(
        receipt(&v).contains("· review ✓ grok-4.7"),
        "{}",
        receipt(&v)
    );
    v.last_review = Some((tree.clone(), "x-ai/grok-4.7".into(), Some(false)));
    assert!(
        receipt(&v).contains("· review ✗ grok-4.7 · not tested"),
        "{}",
        receipt(&v)
    );
    // And tested, by the test hat's model.
    v.last_test = Some((tree, "moonshot/kimi-k3".into(), true));
    assert!(
        receipt(&v).ends_with("· review ✗ grok-4.7 · test ✓ kimi-k3"),
        "{}",
        receipt(&v)
    );
    // A change after the review: what is committed is not what was read.
    std::fs::write(repo.path().join("config.rs"), "rewritten\n").unwrap();
    assert!(
        receipt(&v)
            .ends_with("· not reviewed after the last change · not tested after the last change"),
        "{}",
        receipt(&v)
    );
}

/// A repo with one committed file changed in two places.
fn workbench_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.email", "ryter@test"]);
    git(&["config", "user.name", "ryter"]);
    git(&["config", "commit.gpgsign", "false"]);
    let old: String = (1..=30).map(|i| format!("line {i}\n")).collect();
    std::fs::write(dir.path().join("config.rs"), &old).unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-qm", "base"]);
    let new = old
        .replace("line 3\n", "line three\n")
        .replace("line 25\n", "line twenty-five\nextra\n");
    std::fs::write(dir.path().join("config.rs"), new).unwrap();
    dir
}

/// The workbench: the changed file on the left, the chat in the middle, its
/// changes on the right one at a time; `j` moves to the next, `x` undoes that
/// one alone, and nothing typed reaches the composer.
#[test]
fn workbench_shows_changes_and_undoes_one() {
    let repo = workbench_repo();
    let mut v = ledger();
    v.workbench = Some(crate::workbench::Workbench::open(
        &v,
        repo.path().to_path_buf(),
    ));
    let text = render_to_string(&v, 160, 44);
    check_snapshot("workbench-160x44", &text);
    for want in [
        "CHANGES · UNCOMMITTED",
        "▸ config.rs",
        "+3 −2",
        "config.rs  +3 −2 · change 1 of 2",
        "change 1 · line 1",
        "change 2 · line 23",
        "line three",
        "WORKBENCH",
        "x undo change",
    ] {
        assert!(text.contains(want), "missing {want:?}:\n{text}");
    }
    assert!(
        !text.contains("what should change?"),
        "no composer in the workbench"
    );
    let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
    // `j`: the second change; `x`: undo exactly that one.
    assert_eq!(
        crate::run::keys::handle(&mut v, key(KeyCode::Char('j'))),
        Action::None
    );
    let a = crate::run::keys::handle(&mut v, key(KeyCode::Char('x')));
    let Action::RevertHunk { base, path, hunk } = a else {
        panic!("x should undo the change: {a:?}");
    };
    assert_eq!((path.as_str(), hunk), ("config.rs", 1));
    ryter_core::review::revert_hunk(repo.path(), &base, &path, hunk).unwrap();
    let now = std::fs::read_to_string(repo.path().join("config.rs")).unwrap();
    assert!(
        now.contains("line three") && !now.contains("extra"),
        "{now}"
    );
    assert!(
        v.composer.is_empty(),
        "keys went to the workbench, not the composer"
    );
    // `X` asks first; Esc leaves.
    assert_eq!(
        crate::run::keys::handle(&mut v, key(KeyCode::Char('X'))),
        Action::None
    );
    assert_eq!(
        crate::run::keys::handle(&mut v, key(KeyCode::Char('n'))),
        Action::None
    );
    crate::run::keys::handle(&mut v, key(KeyCode::Esc));
    assert!(v.workbench.is_none());
}

/// The strip across the top names the views there are and lights the one
/// on screen. Crew mode's board is not one of them any more.
#[test]
fn the_view_strip_names_every_view() {
    let v = ledger();
    let text = render_to_string(&v, 140, 40);
    let top = text.lines().next().unwrap();
    assert!(top.contains("chat") && top.contains("changes  ^t"), "{top}");
    assert!(!text.contains("crew"), "{text}");
    // The workbench lights `changes`.
    let repo = workbench_repo();
    let mut v = ledger();
    v.workbench = Some(crate::workbench::Workbench::open(
        &v,
        repo.path().to_path_buf(),
    ));
    let text = render_to_string(&v, 140, 40);
    let top = text.lines().next().unwrap().to_string();
    assert!(top.contains("chat  esc"), "{top}");
    assert!(text.contains("CHANGES"), "{text}");
}

/// `$` opens the spend drawer above the composer: the turn, the session, and
/// the project side by side, by role, and the budget.
#[test]
fn snapshot_ledger_spend_drawer() {
    let mut v = ledger();
    let mut p = ryter_core::project::ProjectSpend::default();
    p.total_usd = 2.29;
    p.sessions = 6;
    p.calls = 501;
    p.by_role.insert("builder".into(), 0.83);
    p.by_role.insert("auditor".into(), 0.73);
    p.by_role.insert("orchestrator".into(), 0.46);
    p.by_role.insert("architect".into(), 0.27);
    v.project_spend = Some(p);
    let a = crate::run::keys::handle(
        &mut v,
        KeyEvent::new(KeyCode::Char('$'), KeyModifiers::NONE),
    );
    assert_eq!(a, Action::OpenPanel(PanelId::SpendDrawer));
    let e = env();
    let _ = panel::open(&mut v, PanelId::SpendDrawer, &e);
    let text = render_to_string(&v, 120, 40);
    check_snapshot("ledger-spend-drawer-120x40", &text);
    assert!(
        text.contains("project · muzak") && text.contains("$2.29"),
        "{text}"
    );
    // A dollar sign inside a message is just text.
    let mut v = ledger();
    v.composer.insert_str("echo ");
    let a = crate::run::keys::handle(
        &mut v,
        KeyEvent::new(KeyCode::Char('$'), KeyModifiers::NONE),
    );
    assert_ne!(a, Action::OpenPanel(PanelId::SpendDrawer));
}

/// The whole ledger document: the welcome note, the first turn folded to one
/// line, the edit turn on the timeline and its closing line.
#[test]
fn ledger_folds_finished_turns_and_closes_each() {
    let mut v = ledger();
    // A new turn is pinned to the top; scroll up to see what came before.
    render_to_string(&v, 120, 60);
    v.scroll.to_top(false);
    let text = render_to_string(&v, 120, 60);
    check_snapshot("ledger-whole-120x60", &text);
    assert!(text.contains("·  welcome · /help for keys"), "{text}");
    assert!(
        text.contains("why does load() ignore a missing file?")
            && text.contains("✓ answered · 1.8s · $0.002  ▸"),
        "the first turn folds to one line:\n{text}"
    );
    assert!(
        !text.contains("A first run has no config"),
        "its answer is folded away"
    );
    assert!(
        text.contains("└─ ✓ 2 tools · 2 files (1 new, 1 changed, +8 −1) · 2.1s · $0.004"),
        "{text}"
    );
    // ^O shows everything whole.
    let mut v = ledger();
    v.diffs_expanded = true;
    render_to_string(&v, 120, 60);
    v.scroll.to_top(false);
    assert!(render_to_string(&v, 120, 60).contains("A first run has no config"));
}

/// `S-01` for diffs: the glyph snapshot can't see the tint, so check the
/// cells. An added row is tinted green to the chat's right edge, a removed
/// row red, and context not at all.
#[test]
fn edit_rows_are_tinted_to_the_edge() {
    let t = Theme::truecolor_dark();
    let buf = render_buffer(&edited(), 120, 40, t);
    let row_of = |needle: &str| {
        (0..buf.area.height)
            .find(|y| {
                let line: String = (0..buf.area.width)
                    .map(|x| buf[(x, *y)].symbol().to_string())
                    .collect();
                line.contains(needle)
            })
            .unwrap_or_else(|| panic!("no row with {needle}"))
    };
    let bg_at = |x: u16, y: u16| buf[(x, y)].bg;
    let added = row_of("+   if (!q)");
    let removed = row_of("-   const q = req.query.q");
    let context = row_of("app.get('/search'");
    // Where the chat pane ends: the last column the added row tints.
    let edge = (0..buf.area.width)
        .rev()
        .find(|x| bg_at(*x, added) == t.diff_add_bg)
        .unwrap();
    assert!(edge > 60, "tint stops at column {edge}");
    for x in 4..=edge {
        assert_eq!(bg_at(x, added), t.diff_add_bg, "added row, column {x}");
        assert_eq!(bg_at(x, removed), t.diff_del_bg, "removed row, column {x}");
        assert_ne!(bg_at(x, context), t.diff_add_bg);
        assert_ne!(bg_at(x, context), t.diff_del_bg);
    }
}

/// A review: the cost prompt, then the review hat's answer in the chat,
/// its verdict under it, and the hat the user was in put back.
fn reviewed(verdict: Option<bool>) -> View {
    let mut v = edited();
    v.specialists.insert(
        "review".into(),
        ryter_core::config::RoleModel {
            connection: Some("openrouter".into()),
            model: Some("anthropic/claude-opus-5.5".into()),
        },
    );
    let body = match verdict {
        Some(false) => {
            "- **blocking** `app/server.js:6`: an empty query returns early, but `find` still runs on `undefined` when `q` is missing.\n- note: the test covers `\"\"` only.\n\nVERDICT: FAIL"
        }
        _ => {
            "- **note** `app/server.js:6`: an empty query returns `[]`; the test covers it.\n\nVERDICT: PASS"
        }
    };
    for ev in [
        AgentEvent::ModeChanged {
            role: ryter_core::Role::SoloReview,
        },
        AgentEvent::TurnStarted {
            turn: 2,
            role: Role::SoloBuild,
        },
        AgentEvent::Token { text: body.into() },
        AgentEvent::TurnFinished {
            turn: 2,
            tools: 0,
            duration_ms: 3000,
        },
        AgentEvent::ModeChanged {
            role: ryter_core::Role::SoloBuild,
        },
        AgentEvent::Reviewed {
            model: "anthropic/claude-opus-5.5".into(),
            connection: "openrouter".into(),
            verdict,
            tree: Some("4b825dc".into()),
            total_usd: Some(0.04),
        },
    ] {
        crate::run_events_apply(&mut v, ev);
    }
    v
}

#[test]
fn snapshot_review() {
    let mut v = edited();
    v.panels.push(Box::new(PermissionModal::new(
        "review offer".into(),
        "Review this work before you commit?\nx-ai/grok-4.7 on openrouter (the review hat's model)\nreviews 2 files, +8 −1, read-only\nabout $0.02–$0.31 of your $5.00 limit\nyour last 4 reviews with it cost $0.03–$0.19".into(),
    )));
    all_sizes("modal-review", &v);
    all_sizes("review", &reviewed(Some(false)));
}

/// When a hat on another model speaks in the same turn, the ledger names
/// each model as it takes over. Named once a turn, the builder's words
/// after a review read as the reviewer's.
#[test]
fn a_model_is_named_again_after_another_has_spoken() {
    let mut v = idle();
    v.ui.layout = "ledger".into();
    v.specialists.insert(
        "review".into(),
        ryter_core::config::RoleModel {
            connection: Some("openrouter".into()),
            model: Some("x-ai/reviewer-x".into()),
        },
    );
    let _ = v.submit_user("change it".into(), "change it".into());
    let hat = |v: &mut View, role| crate::run_events_apply(v, AgentEvent::ModeChanged { role });
    v.on_token("Changing the greeting.");
    hat(&mut v, ryter_core::Role::SoloReview);
    v.on_token("The word is wrong.\n\nVERDICT: FAIL");
    hat(&mut v, ryter_core::Role::SoloBuild);
    v.on_token("Fixing the word.");
    hat(&mut v, ryter_core::Role::SoloReview);
    v.on_token("Nothing to report.\n\nVERDICT: PASS");
    let screen = render_to_string(&v, 120, 50);
    let at = |what: &str| {
        screen
            .lines()
            .position(|l| l.contains(what))
            .unwrap_or_else(|| panic!("no {what:?} in\n{screen}"))
    };
    let named: Vec<usize> = screen
        .lines()
        .enumerate()
        .filter(|(_, l)| l.contains("◆  grok-4.6") || l.contains("◆  reviewer-x"))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(named.len(), 4, "each takeover is named:\n{screen}");
    // Each name is the line above the words it heads.
    for (name, words) in named.iter().zip([
        "Changing the greeting.",
        "The word is wrong.",
        "Fixing the word.",
        "Nothing to report.",
    ]) {
        assert_eq!(*name + 1, at(words), "{screen}");
    }
    let name = |i: usize| screen.lines().nth(named[i]).unwrap_or("").to_string();
    assert!(
        name(0).contains("grok-4.6") && name(2).contains("grok-4.6"),
        "{screen}"
    );
    assert!(
        name(1).contains("reviewer-x") && name(3).contains("reviewer-x"),
        "{screen}"
    );
    // The same model speaking twice running is named once.
    v.on_token(" Done.");
    crate::run_events_apply(
        &mut v,
        AgentEvent::Notice {
            message: "a note".into(),
        },
    );
    v.on_token("And a second step.");
    let screen = render_to_string(&v, 120, 50);
    assert_eq!(
        screen
            .lines()
            .filter(|l| l.contains("◆  reviewer-x") || l.contains("◆  grok-4.6"))
            .count(),
        4,
        "{screen}"
    );
}

/// The verdict is said under the review, and kept for the commit's receipt
/// until a commit is made.
#[test]
fn a_reviews_verdict_is_said_and_kept_for_the_commit() {
    for (verdict, said) in [
        (Some(true), "✓ no blocking problems"),
        (Some(false), "✗ blocking problems"),
        (None, "no verdict"),
    ] {
        let mut v = reviewed(verdict);
        let screen = render_to_string(&v, 120, 40);
        assert!(
            screen.contains(&format!("review · claude-opus-5.5 · {said} · $0.04")),
            "{screen}"
        );
        // The review is headed by the model that wrote it, not the one
        // every other hat uses.
        let heads: Vec<&str> = v
            .messages
            .iter()
            .filter_map(|m| match &m.kind {
                crate::chat::MessageKind::Assistant { model } => Some(model.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            heads.last(),
            Some(&"anthropic/claude-opus-5.5"),
            "{heads:?}"
        );
        assert_eq!(
            v.last_review,
            Some((
                Some("4b825dc".to_string()),
                "anthropic/claude-opus-5.5".to_string(),
                verdict
            ))
        );
        // The hat the user was in is back.
        assert_eq!(v.mode, ryter_core::Role::SoloBuild);
        crate::run_events_apply(
            &mut v,
            AgentEvent::Committed {
                summary: Some("abc1234 Fix the query".into()),
                error: None,
            },
        );
        assert_eq!(v.last_review, None);
    }
}

/// Not a test: renders preview scenes in the themes named by
/// `RYTER_PREVIEW_THEMES` (`name=path.toml,…`; `dark` is built in) as ANSI
/// files in `RYTER_PREVIEW_OUT`, for comparing looks side by side.
#[test]
#[ignore]
fn render_theme_preview() {
    let Ok(out) = std::env::var("RYTER_PREVIEW_OUT") else {
        return;
    };
    let themes = std::env::var("RYTER_PREVIEW_THEMES").unwrap_or_default();
    let ansi = |buf: &ratatui::buffer::Buffer| {
        use ratatui::style::{Color, Modifier};
        let rgb = |c: Color| crate::theme::to_rgb(c);
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let cell = &buf[(x, y)];
                s.push_str("\x1b[0m");
                if let Some((r, g, b)) = rgb(cell.fg) {
                    s.push_str(&format!("\x1b[38;2;{r};{g};{b}m"));
                }
                if let Some((r, g, b)) = rgb(cell.bg) {
                    s.push_str(&format!("\x1b[48;2;{r};{g};{b}m"));
                }
                if cell.modifier.contains(Modifier::BOLD) {
                    s.push_str("\x1b[1m");
                }
                s.push_str(cell.symbol());
            }
            s.push_str("\x1b[0m\n");
        }
        s
    };
    type Scene = (&'static str, Box<dyn Fn() -> View>);
    let scenes: Vec<Scene> = vec![
        ("chat", Box::new(|| reviewed(Some(true)))),
        (
            "models",
            Box::new(|| {
                let mut v = edited();
                let m = |id: &str, i: f64, o: f64| ryter_core::ModelInfo {
                    id: id.into(),
                    context_length: Some(256_000),
                    input_per_million: Some(i),
                    output_per_million: Some(o),
                    connection: Some("openrouter".into()),
                    created: None,
                    tools: Some(true),
                };
                let mut p = crate::panel::models::Models::new(&mut v, None);
                p.set_models(
                    &v,
                    &[
                        m("anthropic/claude-opus-5.5", 5.0, 25.0),
                        m("deepseek/deepseek-v4.1-flash", 0.04, 0.29),
                        m("openai/gpt-5.5", 5.0, 15.0),
                        m("x-ai/grok-4.7", 1.6, 4.8),
                        m("z-ai/glm-5.3", 0.4, 1.6),
                        m("moonshotai/kimi-k3", 1.0, 4.0),
                    ],
                );
                v.panels.push(Box::new(p));
                crate::panel::sync_composer(&mut v);
                v
            }),
        ),
        (
            "permission",
            Box::new(|| {
                let mut v = edited();
                let old = "app.get('/search', (req, res) => {\n  const q = req.query.q\n  res.json(find(q))\n})\n";
                let new = old.replace(
                    "  const q = req.query.q\n",
                    "  const q = (req.query.q || '').trim()\n  if (!q) return res.json([])\n",
                );
                v.panels.push(Box::new(
                    PermissionModal::new("search_replace".into(), "app/server.js".into())
                        .with_preview(Some(Box::new(ryter_core::diff::FileDiff::new(
                            "app/server.js",
                            Some(old),
                            &new,
                        )))),
                ));
                v
            }),
        ),
    ];
    for spec in std::iter::once("dark=").chain(themes.split(',').filter(|s| !s.is_empty())) {
        let (name, path) = spec.split_once('=').unwrap();
        let theme = if path.is_empty() {
            Theme::truecolor_dark()
        } else {
            Theme::from_file(std::path::Path::new(path)).unwrap()
        };
        for (scene, make) in &scenes {
            let buf = render_buffer(&make(), 120, 34, theme);
            std::fs::write(format!("{out}/{scene}-{name}.ansi"), ansi(&buf)).unwrap();
        }
    }
}

fn chat_bodies(v: &View) -> Vec<String> {
    v.messages.iter().map(|m| m.body.clone()).collect()
}

fn one_turn(v: &mut View, role: Role, say: &str) {
    crate::run_events_apply(v, AgentEvent::TurnStarted { turn: 9, role });
    crate::run_events_apply(v, AgentEvent::Token { text: say.into() });
    crate::run_events_apply(
        v,
        AgentEvent::TurnFinished {
            turn: 9,
            tools: 0,
            duration_ms: 10,
        },
    );
}

/// The test hat shows the tester's own conversation, under a line that
/// names it. Any other hat shows the one they share, as it was left.
#[test]
fn the_test_hat_shows_its_own_conversation() {
    use ryter_core::Thread;
    let mut v = with_rail();
    let main_before = chat_bodies(&v);
    assert!(!main_before.is_empty());
    v.mode = Role::SoloTest;
    v.show(Thread::Test);
    assert!(v.messages.is_empty(), "{:?}", chat_bodies(&v));
    let text = render_to_string(&v, 140, 44);
    assert!(
        text.contains("TEST THREAD · its own conversation · tab: main chat"),
        "{text}"
    );
    assert!(!text.contains("why does load()"), "{text}");
    assert!(text.contains("ask the tester · Tab: build"), "{text}");
    // The rail's session block still counts the whole session.
    assert!(text.contains("2 turns"), "{text}");
    // What is typed here is the tester's.
    let _ = v.submit_user("test the search".into(), "test the search".into());
    assert_eq!(v.turn_thread, Thread::Test);
    one_turn(&mut v, Role::SoloTest, "the search fails");
    assert!(chat_bodies(&v).iter().any(|b| b == "the search fails"));
    assert!(v.test_thread_started());
    // Back in the shared conversation: nothing of the tester's.
    v.mode = Role::SoloBuild;
    v.show(Thread::Main);
    assert_eq!(chat_bodies(&v), main_before);
    let text = render_to_string(&v, 140, 44);
    assert!(
        !text.contains("TEST THREAD") && !text.contains("the search fails"),
        "{text}"
    );
    assert!(v.test_thread_started());
    // Without the rail, the strip at the top names it.
    v.panel_visible = false;
    v.show(Thread::Test);
    let text = render_to_string(&v, 100, 30);
    assert!(text.contains(" test thread "), "{text}");
}

/// What a turn says goes into the conversation the turn is part of,
/// whichever is on screen: the user can look at the main chat while a test
/// runs, and at the tester's while a build does.
#[test]
fn a_running_turn_writes_to_its_own_conversation() {
    use ryter_core::Thread;
    let mut v = ledger();
    let main_before = chat_bodies(&v).len();
    // A test turn starts; the user tabs back to the main chat mid-turn.
    v.show(Thread::Test);
    let _ = v.submit_user("test it".into(), "test it".into());
    crate::run_events_apply(
        &mut v,
        AgentEvent::TurnStarted {
            turn: 3,
            role: Role::SoloTest,
        },
    );
    v.show(Thread::Main);
    crate::run_events_apply(
        &mut v,
        AgentEvent::Token {
            text: "two scenarios fail".into(),
        },
    );
    crate::run_events_apply(
        &mut v,
        AgentEvent::Notice {
            message: "a note from the turn".into(),
        },
    );
    assert_eq!(v.shown, Thread::Main, "the screen stays where it was put");
    assert_eq!(chat_bodies(&v).len(), main_before, "{:?}", chat_bodies(&v));
    crate::run_events_apply(
        &mut v,
        AgentEvent::TurnFinished {
            turn: 3,
            tools: 0,
            duration_ms: 10,
        },
    );
    assert_eq!(chat_bodies(&v).len(), main_before);
    v.show(Thread::Test);
    let test = chat_bodies(&v);
    assert!(
        test.contains(&"two scenarios fail".to_string())
            && test.contains(&"a note from the turn".to_string()),
        "{test:?}"
    );
    // Between turns, a note goes to the conversation on screen.
    crate::run_events_apply(
        &mut v,
        AgentEvent::Notice {
            message: "said between turns".into(),
        },
    );
    assert!(chat_bodies(&v).contains(&"said between turns".to_string()));
}

/// When the agent changes hats itself into the tester's conversation or
/// out of it, the screen follows, and the change is said in the main
/// conversation. A change within one conversation leaves the screen where
/// the user put it.
#[test]
fn the_screen_follows_the_agent_into_a_test_and_back() {
    use ryter_core::Thread;
    let mut v = ledger();
    crate::run_events_apply(
        &mut v,
        AgentEvent::ModeChanged {
            role: Role::SoloTest,
        },
    );
    assert_eq!((v.shown, v.mode), (Thread::Test, Role::SoloTest));
    one_turn(&mut v, Role::SoloTest, "all five pass");
    crate::run_events_apply(
        &mut v,
        AgentEvent::ModeChanged {
            role: Role::SoloBuild,
        },
    );
    assert_eq!((v.shown, v.mode), (Thread::Main, Role::SoloBuild));
    let main = chat_bodies(&v);
    assert!(
        main.iter()
            .any(|b| b.starts_with("switched to the test hat"))
            && main
                .iter()
                .any(|b| b.starts_with("switched to the build hat")),
        "{main:?}"
    );
    assert!(!main.contains(&"all five pass".to_string()), "{main:?}");
    // The user is reading the tester's conversation while a review starts
    // in the main one: the screen is not taken from them.
    v.show(Thread::Test);
    crate::run_events_apply(
        &mut v,
        AgentEvent::ModeChanged {
            role: Role::SoloReview,
        },
    );
    assert_eq!(v.shown, Thread::Test);
}

/// A resumed session brings both conversations back, each in its place.
#[test]
fn a_resumed_session_has_both_conversations() {
    use ryter_core::Thread;
    let home = tempfile::TempDir::new().unwrap();
    let cwd = tempfile::TempDir::new().unwrap();
    let mut s =
        ryter_core::Session::create(home.path(), cwd.path(), "c".into(), "m".into()).unwrap();
    let msg = |role: &str, content: &str| ryter_core::Message {
        role: role.into(),
        content: content.into(),
        tool_call_id: None,
        tool_calls: None,
    };
    s.push_message(msg("user", "[hat: build — x]\n\nbuild the list"))
        .unwrap();
    s.push_message(msg("assistant", "built")).unwrap();
    s.push_to(
        Thread::Test,
        msg("user", "[hat: test — x]\n\ntest the list"),
    )
    .unwrap();
    s.push_to(Thread::Test, msg("assistant", "it fails"))
        .unwrap();
    // A test Ryter started: its brief is the model's to read.
    s.push_to(
        Thread::Test,
        msg(
            "user",
            "[hat: test — x]\n\n[Ryter] Test the work as its user would. The plan the user \
             approved is in `.ryter/plans/x.md`.",
        ),
    )
    .unwrap();
    s.set_mode(Role::SoloTest).unwrap();
    let mut v = ledger();
    crate::run::fill_view_from_session(&mut v, &s);
    // Left in the test hat: the tester's conversation is on screen.
    assert_eq!((v.mode, v.shown), (Role::SoloTest, Thread::Test));
    assert_eq!(
        chat_bodies(&v),
        [
            "test the list",
            "it fails",
            "Ryter · test the work as its user would"
        ]
    );
    v.show(Thread::Main);
    assert_eq!(chat_bodies(&v), ["build the list", "built"]);
    // A new session has neither.
    v.reset_transcript();
    assert!(v.messages.is_empty() && !v.test_thread_started());
    assert_eq!(v.shown, Thread::Main);
}

fn product(running: bool) -> AgentEvent {
    AgentEvent::Product {
        running,
        at: "2026-10-01 14:02".into(),
        address: Some("http://localhost:8000".into()),
        stop: Some("docker compose down".into()),
    }
}

/// A product Ryter started is said once when it comes up, `/stop` is how
/// it is stopped, and quitting asks about it instead of leaving it behind
/// without a word.
#[test]
fn a_running_product_is_named_and_asked_about_on_quit() {
    let mut v = ledger();
    crate::run_events_apply(&mut v, product(true));
    crate::run_events_apply(&mut v, product(true));
    let said: Vec<String> = chat_bodies(&v)
        .into_iter()
        .filter(|b| b.contains("the project is running"))
        .collect();
    assert_eq!(
        said,
        [
            "the project is running at http://localhost:8000, started 14:02 · /stop stops it (docker compose down)"
        ]
    );
    assert!(v.product.is_some());
    // The question on quit, as it was drawn for the user.
    v.panels.push(Box::new(crate::panel::modal::StopModal));
    let text = render_to_string(&v, 120, 40);
    for want in [
        "stop the project?",
        "Ryter started it for the test at 14:02.",
        "docker compose down",
        "⏎ stop it",
        "n leave it running",
    ] {
        assert!(text.contains(want), "missing {want:?}:\n{text}");
    }
    let key = |v: &mut View, code| {
        crate::run_keys_handle(
            v,
            crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE),
        )
    };
    assert_eq!(
        key(&mut v, crossterm::event::KeyCode::Enter),
        Action::QuitAnswer { stop: true }
    );
    v.panels.push(Box::new(crate::panel::modal::StopModal));
    assert_eq!(
        key(&mut v, crossterm::event::KeyCode::Char('n')),
        Action::QuitAnswer { stop: false }
    );
    // Stopped: nothing left to ask about.
    crate::run_events_apply(&mut v, product(false));
    assert!(v.product.is_none());
}

/// The project's own commands read as what they do in the chat.
#[test]
fn the_projects_commands_are_named_for_what_they_do() {
    let mut v = ledger();
    for (id, action, cmd) in [
        ("p1", "start", "docker compose up -d --wait"),
        (
            "p2",
            "test",
            "docker compose run --rm web pytest -q (+1 more)",
        ),
    ] {
        crate::run_events_apply(
            &mut v,
            AgentEvent::ToolCall {
                id: id.into(),
                name: "run_project".into(),
                args: serde_json::json!({ "action": action }),
                role: Role::SoloTest,
                summary: Some(cmd.into()),
            },
        );
    }
    // Each says how it went: started, and the tests' own totals.
    for (id, output) in [
        (
            "p1",
            "Started in 14s: http://localhost:8000/healthz answered 200.",
        ),
        ("p2", "$ pytest -q\n21 passed in 3.2s\n"),
    ] {
        crate::run_events_apply(
            &mut v,
            AgentEvent::ToolResult {
                id: id.into(),
                output: output.into(),
                is_error: false,
                duration_ms: Some(14_000),
                diff: None,
            },
        );
    }
    let details: Vec<String> = v
        .messages
        .iter()
        .filter(|m| matches!(m.kind, crate::chat::MessageKind::Tool { .. }))
        .map(|m| m.meta.detail.clone().unwrap_or_default())
        .collect();
    assert!(
        details.contains(&"✓".to_string()) && details.iter().any(|d| d.starts_with("✓ 21 passed")),
        "{details:?}"
    );
    let steps: Vec<(String, String)> = v
        .messages
        .iter()
        .filter_map(|m| match &m.kind {
            crate::chat::MessageKind::Tool { name, .. } => {
                Some((name.clone(), m.meta.label.clone().unwrap_or_default()))
            }
            _ => None,
        })
        .collect();
    assert!(
        steps.contains(&("start".into(), "docker compose up -d --wait".into()))
            && steps.contains(&(
                "test".into(),
                "docker compose run --rm web pytest -q (+1 more)".into()
            )),
        "{steps:?}"
    );
}

fn tested(passed: bool) -> AgentEvent {
    AgentEvent::Tested {
        model: "moonshot/kimi-k3".into(),
        headline: if passed {
            "✓ 5 of 5 passed".into()
        } else {
            "✗ 2 of 5 failed".into()
        },
        passed,
        rows: [
            "✓ 1  the stack starts and is healthy",
            "✓ 2  first-run setup creates the admin",
            "✗ 3  /manage/ after login",
            "     expected the page list",
            "     got 500: NoReverseMatch 'pages:list'",
            "     to see it: start the stack, log in, open /manage/",
            "✗ 4  publish a page · not reached (needs 3)",
            "✓ 5  pytest in the container · 21 passed",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
        file: ".ryter/tests/2026-10-01-cms-2.md".into(),
        first_failed: (!passed).then_some(3),
        tree: Some("t1".into()),
        total_usd: Some(0.21),
        duration_ms: 100_000,
    }
}

/// The tester's report comes back into the conversation the other hats
/// share, as the user approved it: failures opened out, passes one line,
/// the file it is in, and where the product was left running. The tester's
/// own turn closes on what it reported.
#[test]
fn a_tests_report_is_a_card_in_the_main_conversation() {
    use ryter_core::Thread;
    let mut v = with_rail();
    crate::run_events_apply(&mut v, product(true));
    // A test run: the agent puts on the test hat, works, files its report,
    // and the hat the user was in comes back.
    crate::run_events_apply(
        &mut v,
        AgentEvent::ModeChanged {
            role: Role::SoloTest,
        },
    );
    crate::run_events_apply(
        &mut v,
        AgentEvent::TurnStarted {
            turn: 7,
            role: Role::SoloTest,
        },
    );
    crate::run_events_apply(&mut v, tested(false));
    crate::run_events_apply(
        &mut v,
        AgentEvent::TurnFinished {
            turn: 7,
            tools: 6,
            duration_ms: 100_000,
        },
    );
    // In the tester's thread: its turn closes on the report, and the line
    // over the thread counts the run.
    assert_eq!(v.shown, Thread::Test);
    let thread = render_to_string(&v, 140, 44);
    assert!(thread.contains("✗ 2 of 5 failed · 1:40"), "{thread}");
    assert!(
        thread.contains("TEST THREAD · 1 run this session · tab: main chat"),
        "{thread}"
    );
    assert!(
        thread.contains("ask the tester, or: retest 3 · Tab: build"),
        "{thread}"
    );
    assert!(!thread.contains("full report"), "{thread}");
    crate::run_events_apply(
        &mut v,
        AgentEvent::ModeChanged {
            role: Role::SoloBuild,
        },
    );
    assert_eq!(v.shown, Thread::Main);
    let text = render_to_string(&v, 140, 60);
    let mut at = 0;
    for want in [
        "switched to the test hat",
        "▣  test · kimi-k3 · ✗ 2 of 5 failed · 1:40 · $0.21",
        "│  ✓ 1  the stack starts and is healthy",
        "│  ✓ 2  first-run setup creates the admin",
        "│  ✗ 3  /manage/ after login",
        "│       expected the page list",
        "│       got 500: NoReverseMatch 'pages:list'",
        "│       to see it: start the stack, log in, open /manage/",
        "│  ✗ 4  publish a page · not reached (needs 3)",
        "│  ✓ 5  pytest in the container · 21 passed",
        "│  full report  .ryter/tests/2026-10-01-cms-2.md",
        "the project is still running at http://localhost:8000",
        "/stop stops it (docker compose down)",
        "switched to the build hat",
    ] {
        let i = text[at..]
            .find(want)
            .unwrap_or_else(|| panic!("{want:?} is missing or out of order:\n{text}"));
        at += i;
    }
    // The commit's receipt carries it, for the files that were tested.
    assert_eq!(
        v.last_test,
        Some((Some("t1".into()), "moonshot/kimi-k3".into(), false))
    );
    // A second run, all passing: counted, and nothing to retest.
    crate::run_events_apply(&mut v, tested(true));
    assert_eq!((v.test_runs, v.retest), (2, None));
    v.show(Thread::Test);
    let thread = render_to_string(&v, 140, 44);
    assert!(
        thread.contains("TEST THREAD · 2 runs this session"),
        "{thread}"
    );
}

/// A resumed session shows the reports it was given as the cards they
/// were, and counts them.
#[test]
fn a_resumed_session_shows_its_reports_as_cards() {
    use ryter_core::Thread;
    let home = tempfile::TempDir::new().unwrap();
    let cwd = tempfile::TempDir::new().unwrap();
    let mut s =
        ryter_core::Session::create(home.path(), cwd.path(), "c".into(), "m".into()).unwrap();
    let report = "[Ryter] The test hat (kimi-k3) used the product and filed this report: ✗ 2 of \
                  5 failed.\n✓ 1  the stack starts\n✗ 2  publish · not reached (needs 3)\n✗ 3  \
                  /manage/ after login\n     got 500\nAlso: the media library was not \
                  tested.\nThe full report is in `.ryter/tests/2026-10-01-cms.md`. The tester \
                  worked in a conversation of its own and changed nothing.";
    s.push_message(ryter_core::Message {
        role: "user".into(),
        content: report.into(),
        tool_call_id: None,
        tool_calls: None,
    })
    .unwrap();
    let mut v = ledger();
    crate::run::fill_view_from_session(&mut v, &s);
    assert_eq!(v.shown, Thread::Main);
    assert_eq!(
        chat_bodies(&v),
        [
            "test · kimi-k3 · ✗ 2 of 5 failed\n✓ 1  the stack starts\n✗ 2  publish · not reached \
          (needs 3)\n✗ 3  /manage/ after login\n     got 500\nfull report  \
          .ryter/tests/2026-10-01-cms.md"
        ]
    );
    assert!(matches!(
        v.messages[0].kind,
        crate::chat::MessageKind::System {
            level: crate::chat::SystemLevel::Report { failed: true }
        }
    ));
    assert_eq!((v.test_runs, v.retest), (1, Some(3)));
}
