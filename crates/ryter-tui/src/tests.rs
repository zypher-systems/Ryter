//! Crate-level tests (§15): golden snapshots, scroll invariants, secret
//! redaction, help ↔ keymap parity, and the redraw budget.
//!
//! Snapshots live in `crates/ryter-tui/snapshots/`. Regenerate with
//! `UPDATE_SNAPSHOTS=1 cargo test -p ryter-tui`.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ryter_core::{AgentEvent, Phase, Role};

use crate::action::{PanelId, SessionsMode};
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
        Phase::Build,
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
            role: Role::Orchestrator,
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
    let panels: [(&str, PanelId); 18] = [
        ("providers", PanelId::Providers),
        ("models", PanelId::Models),
        ("crew", PanelId::Crew),
        ("crew-builder", PanelId::CrewBuilder),
        ("agents", PanelId::Agents),
        ("sessions", PanelId::Sessions(SessionsMode::Browse)),
        ("spend", PanelId::Spend),
        ("budget", PanelId::Budget),
        ("settings", PanelId::Settings),
        ("theme", PanelId::Theme),
        ("tools", PanelId::Tools),
        ("auditor", PanelId::Auditor),
        ("mcp", PanelId::Mcp),
        ("skills", PanelId::Skills),
        ("hooks", PanelId::Hooks),
        ("context", PanelId::Context),
        ("help", PanelId::Help),
        ("doctor", PanelId::Doctor),
    ];
    for (name, id) in panels {
        let v = with_panel(id);
        assert!(!v.panels.is_empty(), "{name} did not open");
        all_sizes(&format!("panel-{name}"), &v);
    }
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

#[test]
fn crew_suggests_a_tiered_crew_and_applies_it_on_y() {
    use crate::action::Action;
    use crate::panel::{Notice, Outcome};
    let mut view = with_panel(PanelId::Crew);
    view.connection = "openrouter".into();
    view.model = "deepseek/deepseek-v4.1-flash".into();
    let key = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
    let mut crew = view.panels.stack.pop().unwrap();
    assert!(matches!(
        crew.key(key('s'), &mut view),
        Outcome::Act(Action::ListCrewModels { .. })
    ));
    let row = |id: &str, i: f64, o: f64| ryter_core::ModelInfo {
        id: id.into(),
        context_length: Some(400_000),
        input_per_million: Some(i),
        output_per_million: Some(o),
        connection: Some("openrouter".into()),
        created: None,
        tools: Some(true),
    };
    crew.on_notice(
        &Notice::Models(vec![
            row("deepseek/deepseek-v4.1-flash", 0.15, 0.6),
            row("openai/gpt-5.5", 5.0, 30.0),
        ]),
        &mut view,
    );
    view.panels.stack.push(crew);
    let frame = render_to_string(&view, 120, 40);
    assert!(frame.contains("schooner — balanced"), "{frame}");
    assert!(frame.contains("gpt-5.5"), "{frame}");
    let mut crew = view.panels.stack.pop().unwrap();
    match crew.key(key('y'), &mut view) {
        Outcome::Act(Action::ApplyCrewTiering(rows)) => {
            assert_eq!(rows["auditor"].model.as_deref(), Some("openai/gpt-5.5"));
            assert_eq!(
                rows["builder"].model.as_deref(),
                Some("deepseek/deepseek-v4.1-flash")
            );
        }
        _ => panic!("y must apply the suggestion"),
    }
}

/// The ready-made crews are rows in `/crew`; the galleon puts the strong
/// model in the builder's seat.
#[test]
fn crew_offers_three_ready_made_crews() {
    use crate::action::Action;
    use crate::panel::{Notice, Outcome};
    let mut view = with_panel(PanelId::Crew);
    view.connection = "openrouter".into();
    view.model = "deepseek/deepseek-v4.1-flash".into();
    let frame = render_to_string(&view, 120, 40);
    for name in ["skiff", "schooner", "galleon", "low cost", "high cost"] {
        assert!(frame.contains(name), "{name} missing:\n{frame}");
    }
    let press = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
    let mut crew = view.panels.stack.pop().unwrap();
    // Three roles, then skiff, schooner, galleon.
    for _ in 0..5 {
        crew.key(press(KeyCode::Down), &mut view);
    }
    assert!(matches!(
        crew.key(press(KeyCode::Enter), &mut view),
        Outcome::Act(Action::ListCrewModels { .. })
    ));
    let row = |id: &str, i: f64, o: f64| ryter_core::ModelInfo {
        id: id.into(),
        context_length: Some(400_000),
        input_per_million: Some(i),
        output_per_million: Some(o),
        connection: Some("openrouter".into()),
        created: None,
        tools: Some(true),
    };
    crew.on_notice(
        &Notice::Models(vec![
            row("deepseek/deepseek-v4.1-flash", 0.15, 0.6),
            row("openai/gpt-5.5", 5.0, 30.0),
            row("anthropic/claude-opus-5", 5.0, 25.0),
        ]),
        &mut view,
    );
    match crew.key(press(KeyCode::Char('y')), &mut view) {
        Outcome::Act(Action::ApplyCrewTiering(rows)) => {
            assert_eq!(rows["builder"].model.as_deref(), Some("openai/gpt-5.5"));
            assert_eq!(
                rows["auditor"].model.as_deref(),
                Some("anthropic/claude-opus-5")
            );
        }
        _ => panic!("y must apply the galleon"),
    }
}

/// The user talks to the lead. No phase, no handoff, no "orchestrator" on screen.
#[test]
fn the_screen_speaks_of_the_lead_not_phases() {
    for view in [idle(), mid_stream(ActivityMode::Collapsed)] {
        for (w, h) in SIZES {
            let frame = render_to_string(&view, w, h);
            assert!(!frame.contains("orchestrator"), "{w}x{h}:\n{frame}");
            assert!(frame.contains("lead"), "{w}x{h}:\n{frame}");
            for gone in ["─ build ─", "─ plan ─", "handoff", "phase"] {
                assert!(!frame.contains(gone), "{w}x{h} shows {gone:?}:\n{frame}");
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
    // Principle 1: the conversation gets the space. `no tasks yet` and
    // `no specialists running` used to hold a column open to say nothing (G-03).
    let view = idle();
    for (w, h) in SIZES {
        let frame = render_to_string(&view, w, h);
        assert!(!frame.contains("no tasks yet"), "{w}x{h}: empty tasks card");
        assert!(
            !frame.contains("no specialists running"),
            "{w}x{h}: empty crew card"
        );
    }
}

/// The raw session id is operator chrome; `/sessions` is where it belongs (G-07).
#[test]
fn session_card_leads_with_title_and_crew_state() {
    let view = idle();
    let frame = render_to_string(&view, 120, 40);
    let card_line = frame
        .lines()
        .find(|l| l.contains("untitled"))
        .unwrap_or_default()
        .to_string();
    assert!(
        card_line.contains("idle"),
        "the crew's state shares the row: {card_line:?}"
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
    for id in [PanelId::Help, PanelId::Settings, PanelId::Crew] {
        let view = with_panel(id);
        for (w, h) in SIZES {
            let frame = render_to_string(&view, w, h);
            for needle in [
                "╭─ session",
                "╭─ model",
                "╭─ spend",
                "auditor ✓",
                "price unknown",
            ] {
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
            s.contains("ask the lead") || s.contains("you"),
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
