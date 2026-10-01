//! Crate-level tests (§15): golden snapshots, scroll invariants, secret
//! redaction, help ↔ keymap parity, and the redraw budget.
//!
//! Snapshots live in `crates/ryter-tui/snapshots/`. Regenerate with
//! `UPDATE_SNAPSHOTS=1 cargo test -p ryter-tui`.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ryter_core::{AgentEvent, Phase, Role};

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
    let panels: [(&str, PanelId); 20] = [
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
        PanelId::Crew,
        PanelId::CrewBuilder,
        PanelId::Agents,
        PanelId::Sessions(SessionsMode::Browse),
        PanelId::Spend,
        PanelId::Budget,
        PanelId::Settings,
        PanelId::Theme,
        PanelId::Tools,
        PanelId::Auditor,
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

/// The screen says who gets the next message: the hat in solo mode (its
/// badge on the composer), the lead in crew mode. Never "orchestrator" or a
/// phase.
#[test]
fn the_screen_shows_the_mode() {
    for mode in [
        ryter_core::Role::SoloBuild,
        ryter_core::Role::SoloPlan,
        ryter_core::Role::SoloReview,
        ryter_core::Role::Orchestrator,
    ] {
        for base in [idle(), mid_stream(ActivityMode::Collapsed)] {
            let mut view = base;
            view.mode = mode;
            for (w, h) in SIZES {
                let frame = render_to_string(&view, w, h);
                let badge = format!(" {} ", view.mode_label().to_ascii_uppercase());
                assert!(frame.contains(&badge), "{w}x{h} no {badge:?}:\n{frame}");
                if mode == ryter_core::Role::Orchestrator {
                    assert!(frame.contains("crew · lead"), "{w}x{h}:\n{frame}");
                }
                for gone in ["orchestrator", "handoff", "phase"] {
                    assert!(!frame.contains(gone), "{w}x{h} shows {gone:?}:\n{frame}");
                }
            }
        }
    }
}

/// Solo mode has no crew, so the crew's cards only appear in crew mode.
#[test]
fn crew_cards_only_in_crew_mode() {
    let mut view = mid_stream(ActivityMode::Collapsed);
    view.crew.push(crate::view::CrewRow {
        id: "01".into(),
        role: "builder".into(),
        label: "add a flag".into(),
        spend: None,
        status: "working".into(),
        started_ms: 0,
        acting: "builder".into(),
        live: None,
        tools: 0,
    });
    let frame = render_to_string(&view, 160, 50);
    assert!(
        !frame.contains("╭─ crew"),
        "solo mode shows the crew card:\n{frame}"
    );
    view.mode = ryter_core::Role::Orchestrator;
    let frame = render_to_string(&view, 160, 50);
    assert!(
        frame.contains("╭─ crew"),
        "crew mode hides the crew card:\n{frame}"
    );
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
    crate::run_events_apply(&mut v, AgentEvent::TurnStarted { turn: 1 });
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
        "plan · review   tab switch",
        "MODEL",
        "24k of 200k tokens",
        "SPEND",
        "this turn",
        "session",
        "budget      off",
        "CHANGED",
        "app/server.js",
        "chat  changes ^t  crew /crew",
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
        text.contains(" PLAN") && text.contains("build · review"),
        "{text}"
    );
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
            !text.contains("R Y T E R") && text.contains("crew board"),
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

/// The rail and the live lanes draw at every size without losing the
/// prompt, and the rail steps aside below its width.
#[test]
fn rail_and_lanes_fit_every_size() {
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
        let text = render_to_string(&crew_live(), w, h);
        assert!(text.contains("LANES") || h < 24, "{w}x{h}:\n{text}");
    }
}

#[test]
fn snapshot_ledger() {
    all_sizes("ledger", &ledger());
}

/// Mission control: crew mode with a plan puts the board above the lead's
/// chat, drawn from the queue snapshot and the live lanes.
fn crew_board() -> View {
    let mut v = ledger();
    v.mode = Role::Orchestrator;
    v.budget_usd = 3.0;
    v.spend = Some(0.10);
    v.turn_calls = 43;
    v.now_ms = 64_000;
    v.crew_started_ms = Some(0);
    let t = |id: &str, role: &str, status: &str, by: &str, waits: &[&str], reason: &str| {
        ryter_core::queue::TaskView {
            id: id.into(),
            title: format!("{id} task"),
            role: role.into(),
            status: status.into(),
            by: by.into(),
            waits_on: waits.iter().map(|s| s.to_string()).collect(),
            reason: reason.into(),
            retries: 0,
            rejections: 0,
        }
    };
    // First snapshot: everything waits on the scaffold.
    v.set_tasks(
        vec![
            t("design", "architect", "done", "orchestrator", &[], ""),
            t("scaffold", "builder", "running", "architect", &[], ""),
            t(
                "greet",
                "builder",
                "pending",
                "architect",
                &["scaffold"],
                "waits on scaffold",
            ),
            t(
                "count",
                "builder",
                "pending",
                "architect",
                &["scaffold"],
                "waits on scaffold",
            ),
        ],
        None,
    );
    // Later: the scaffold landed; greet builds, count is in audit.
    v.set_tasks(
        vec![
            t("design", "architect", "done", "orchestrator", &[], ""),
            t("scaffold", "builder", "done", "architect", &[], ""),
            t("greet", "builder", "running", "architect", &[], ""),
            t("count", "builder", "running", "architect", &[], ""),
        ],
        Some(ryter_core::queue::PatchView {
            branch: "ryter/patch-01a0e642-1".into(),
            target: "main".into(),
            tasks: vec!["scaffold".into(), "greet".into(), "count".into()],
            landed: vec!["scaffold".into()],
        }),
    );
    for (id, title, acting, status, started) in [
        ("s1", "greet task", "builder", "edit src/greet.rs", 46_000),
        ("s2", "count task", "auditor", "reviewing (glm-5.3)", 59_000),
    ] {
        v.crew.push(crate::view::CrewRow {
            id: id.into(),
            role: "builder".into(),
            label: title.into(),
            spend: None,
            status: status.into(),
            started_ms: started,
            acting: acting.into(),
            live: None,
            tools: 0,
        });
    }
    v
}

#[test]
fn snapshot_crew_board() {
    let v = crew_board();
    let text = render_to_string(&v, 140, 42);
    check_snapshot("crew-board-140x42", &text);
    for want in [
        "SPEND",
        "$0.10 of $3.00",
        "1 of 3 landed",
        "patch-1 → main",
        "lead grok-4.6 · builders same · auditor grok-4.6",
        "1:04",
        "43 calls · 0 retries",
        "│ ✓ design",
        "───▶│ ✓ scaffold",
        "─┬─▶│ ◐ greet",
        "└─▶│ ◑ count",
        "│  landed",
        "│  building 0:18",
        "│  in audit 0:05",
        "patch ▸ main  lands when greet, count land",
        "builder",
        "auditor",
        "reviewing (glm-5.3)",
        "CREW · LEAD",
    ] {
        assert!(text.contains(want), "missing {want:?}:\n{text}");
    }
    // Solo mode, or no plan: no board.
    let mut v = crew_board();
    v.mode = Role::SoloBuild;
    assert!(!render_to_string(&v, 140, 42).contains("PLAN"));
}

/// Design C1: each lane is a live card. What the worker is doing (writing a
/// file, thinking, running a command) shows as it streams, with the file so
/// far, its reasoning, or the command's output; the PULSE tile says how fast
/// the crew is going and when it was last heard from.
fn crew_live() -> View {
    let mut v = crew_board();
    v.budget_usd = 0.0;
    v.task_spend.insert("greet".into(), 0.09);
    v.task_spend.insert("count".into(), 0.14);
    v.crew.push(crate::view::CrewRow {
        id: "s3".into(),
        role: "builder".into(),
        label: "design task".into(),
        spend: None,
        status: "read src/app.rs".into(),
        started_ms: 20_000,
        acting: "builder".into(),
        live: None,
        tools: 0,
    });
    let live = |id: &str, role: Role, phase, target: &str, tokens, lines, tail: &[&str]| {
        AgentEvent::SubagentLive {
            id: ryter_core::SubagentId::new(id),
            role,
            phase,
            target: target.into(),
            tokens,
            lines,
            tail: tail.iter().map(|t| t.to_string()).collect(),
        }
    };
    use ryter_core::LivePhase::{Running, Thinking, Writing};
    v.now_ms = 60_000;
    for (id, role, phase, target, tokens, lines, tail) in [
        (
            "s1",
            Role::Builder,
            Writing,
            "edit src/greet.rs",
            1200,
            180,
            &[
                "fn greet(name: &str) -> String {",
                "    let who = name.trim();",
            ][..],
        ),
        (
            "s2",
            Role::Auditor,
            Running,
            "bash cargo test",
            0,
            0,
            &[
                "test greet::tests::trims ... ok",
                "     Running unittests src/main.rs",
            ][..],
        ),
        (
            "s3",
            Role::Builder,
            Thinking,
            "",
            3000,
            0,
            &[
                "…repeat lives in queue.rs, not in App, so the bar needs a read-only view of the queue. I'll add Queue::mode()",
            ][..],
        ),
    ] {
        crate::run_events_apply(&mut v, live(id, role, phase, target, tokens, lines, tail));
    }
    v.now_ms = 62_000;
    for (id, role, phase, target, tokens, lines, tail) in [
        (
            "s1",
            Role::Builder,
            Writing,
            "edit src/greet.rs",
            1280,
            212,
            &[
                "fn greet(name: &str) -> String {",
                "    let who = name.trim();",
                "    format!(\"hello, {who}\")",
            ][..],
        ),
        (
            "s3",
            Role::Builder,
            Thinking,
            "",
            3056,
            0,
            &[
                "…repeat lives in queue.rs, not in App, so the bar needs a read-only view of the queue. I'll add Queue::mode() instead of passing the queue in",
            ][..],
        ),
    ] {
        crate::run_events_apply(&mut v, live(id, role, phase, target, tokens, lines, tail));
    }
    v.crew[0].tools = 9;
    v.crew[1].tools = 4;
    if let Some(t) = v.tasks.iter_mut().find(|t| t.id == "count") {
        t.rejections = 2;
    }
    v.spend_log.push((50_000, 0.02));
    v.tick(62_000);
    v.now_ms = 62_300;
    v
}

#[test]
fn live_lanes_show_what_each_worker_is_doing() {
    let v = crew_live();
    let text = render_to_string(&v, 160, 48);
    if std::env::var_os("SHOW").is_some() {
        println!("{text}");
    }
    for want in [
        "PULSE",
        "tok/s · 3 working",
        "last byte",
        "+$0.020 in the last minute",
        " WRITING  edit src/greet.rs",
        "212 lines so far · ~1.3k tok · 40 tok/s · task $0.090 · 9 tools so far",
        "+ fn greet(name: &str) -> String {",
        "+     format!(\"hello, {who}\")▌",
        " RUNNING  $ cargo test",
        "task $0.14 · 4 tools so far · rejected 2 times",
        "test greet::tests::trims ... ok",
        " THINKING ",
        "I'll add Queue::mode()",
    ] {
        assert!(text.contains(want), "missing {want:?}:\n{text}");
    }
    assert!(
        text.contains("live · newest output at the bottom of each card"),
        "{text}"
    );
    // ^r hides the reasoning, and shows it again.
    assert!(text.contains("^r hide reasoning"), "{text}");
    let mut v = v;
    let key = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL);
    crate::run::keys::handle(&mut v, key);
    let hidden = render_to_string(&v, 160, 48);
    assert!(
        hidden.contains("reasoning hidden · ^r shows it")
            && !hidden.contains("Queue::mode()")
            && hidden.contains("+ fn greet"),
        "{hidden}"
    );
    crate::run::keys::handle(&mut v, key);
    assert!(render_to_string(&v, 160, 48).contains("Queue::mode()"));
    // Shorter: the cards drop their output, then become one row a worker.
    let text = render_to_string(&v, 160, 26);
    assert!(
        text.contains("lines so far") && !text.contains("+ fn greet"),
        "{text}"
    );
    let text = render_to_string(&v, 160, 20);
    assert!(
        text.contains("LANES") && !text.contains("lines so far"),
        "{text}"
    );
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

/// Lanes: `tab` picks one, `⏎` opens its transcript, and each shows what
/// its task has cost.
#[test]
fn a_lane_opens_its_transcript() {
    let mut v = crew_board();
    v.task_spend.insert("greet".into(), 0.003);
    v.lane_logs.insert(
        "s1".into(),
        (
            "greet task".into(),
            vec![
                "13:47  builder  read src/lib.rs".into(),
                "13:48  builder  edit src/greet.rs".into(),
            ],
        ),
    );
    let text = render_to_string(&v, 140, 42);
    assert!(
        text.contains("task $0.003"),
        "the lane shows its task's cost:\n{text}"
    );
    let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
    crate::run::keys::handle(&mut v, key(KeyCode::Tab));
    assert_eq!(v.lane_selected, Some(0));
    assert!(render_to_string(&v, 140, 42).contains("on a lane: its transcript"));
    crate::run::keys::handle(&mut v, key(KeyCode::Enter));
    assert_eq!(v.panels.top().map(|p| p.kind()), Some("lane"));
    let text = render_to_string(&v, 140, 42);
    assert!(
        text.contains("edit src/greet.rs") && text.contains("lane · greet task"),
        "{text}"
    );
    // Tab past the last lane lets go.
    v.panels.clear();
    crate::run::keys::handle(&mut v, key(KeyCode::Tab));
    crate::run::keys::handle(&mut v, key(KeyCode::Tab));
    assert_eq!(v.lane_selected, None);
}

/// Last night's run on the board: the scaffold blocked on a missing system
/// package, its reason in plain sight, and what waits on it.
#[test]
fn a_blocked_scaffold_shows_why_and_what_waits() {
    let mut v = crew_board();
    v.crew.clear();
    let t = |id: &str, status: &str, waits: &[&str], reason: &str| ryter_core::queue::TaskView {
        id: id.into(),
        title: format!("{id} task"),
        role: "builder".into(),
        status: status.into(),
        by: "orchestrator".into(),
        waits_on: waits.iter().map(|s| s.to_string()).collect(),
        reason: reason.into(),
        retries: 0,
        rejections: 0,
    };
    v.task_edges.clear();
    v.set_tasks(
        vec![
            t(
                "scaffold",
                "blocked",
                &[],
                "the builder is blocked: sudo dnf install alsa-lib-devel",
            ),
            t("audio", "pending", &["scaffold"], "waits on scaffold"),
            t("ui", "pending", &["scaffold"], "waits on scaffold"),
        ],
        None,
    );
    let text = render_to_string(&v, 140, 42);
    for want in [
        "│ ✕ scaffold",
        "─┬─▶│ ○ audio",
        "└─▶│ ○ ui",
        "│  waits on scaffold",
        // The box can't hold the reason; it is under the drawing, whole.
        "✕ scaffold  blocked: the builder is blocked:",
        "sudo dnf",
        "alsa-lib-devel",
        "0 of 3 landed",
        "no one is working right now",
    ] {
        assert!(text.contains(want), "missing {want:?}:\n{text}");
    }
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

/// The views are in sight: a strip names them with their keys and lights
/// the one on screen, and crew mode shows its board before there is a plan.
#[test]
fn the_view_strip_names_every_view() {
    let v = ledger();
    let text = render_to_string(&v, 140, 40);
    let top = text.lines().next().unwrap();
    assert!(
        top.contains("chat") && top.contains("changes  ^t") && top.contains("crew board  /crew"),
        "{top}"
    );
    // Crew mode, nothing planned yet: the board is there, saying so.
    let mut v = ledger();
    v.mode = Role::Orchestrator;
    let text = render_to_string(&v, 140, 40);
    assert!(
        text.contains("PLAN") && text.contains("no plan yet"),
        "{text}"
    );
    assert!(text.contains("no one is working right now"), "{text}");
    check_snapshot("crew-board-empty-140x40", &text);
    // The workbench lights `changes`.
    let repo = workbench_repo();
    let mut v = ledger();
    v.workbench = Some(crate::workbench::Workbench::open(
        &v,
        repo.path().to_path_buf(),
    ));
    let top = render_to_string(&v, 140, 40)
        .lines()
        .next()
        .unwrap()
        .to_string();
    assert!(top.contains("chat  esc"), "{top}");
    // In crew mode too the workbench takes the screen, not a strip under the board.
    v.mode = Role::Orchestrator;
    let text = render_to_string(&v, 140, 40);
    assert!(text.contains("CHANGES") && !text.contains("PLAN"), "{text}");
}

/// The user's plan from a real run: a design and the two tasks it wrote,
/// drawn as boxes with the design fanning out to both. And a plan too tall to
/// draw falls back to the tree rather than being cut.
#[test]
fn the_plan_is_drawn_and_falls_back_when_it_wont_fit() {
    let t = |id: &str, role: &str, status: &str, by: &str| ryter_core::queue::TaskView {
        id: id.into(),
        title: format!("{id} task"),
        role: role.into(),
        status: status.into(),
        by: by.into(),
        waits_on: Vec::new(),
        reason: String::new(),
        retries: 0,
        rejections: 0,
    };
    let mut v = crew_board();
    v.crew.clear();
    v.task_edges.clear();
    v.patch_view = None;
    v.set_tasks(
        vec![
            t("modern-style", "architect", "done", "orchestrator"),
            t("style-rewrite", "builder", "running", "architect"),
            t("brand-markup", "builder", "running", "architect"),
        ],
        None,
    );
    let text = render_to_string(&v, 140, 42);
    assert!(text.contains("│ ✓ modern-style"), "{text}");
    assert!(text.contains("─┬─▶│ ◐ style-rewrite"), "{text}");
    assert!(text.contains("└─▶│ ◐ brand-markup"), "{text}");
    // Twelve tasks at 30 rows can't be drawn: the tree, in full.
    let mut many = vec![t("design", "architect", "done", "orchestrator")];
    for i in 0..12 {
        many.push(t(&format!("task-{i}"), "builder", "pending", "architect"));
    }
    v.task_edges.clear();
    v.set_tasks(many, None);
    let text = render_to_string(&v, 140, 30);
    assert!(
        text.contains("✓ design") && text.contains("├▶ ○ task-0") && !text.contains("┌──"),
        "{text}"
    );
    // The plan has the screen's height now: all twelve fit.
    assert!(text.contains("└▶ ○ task-11"), "{text}");
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

/// `/second`: the cost prompt, then the review in the chat with its verdict
/// and cost.
#[test]
fn snapshot_second_opinion() {
    let mut v = edited();
    v.panels.push(Box::new(PermissionModal::new(
        "second opinion".into(),
        "x-ai/grok-4.7 on openrouter (your choice)\nreviews 2 files, +8 −1, read-only\nabout $0.02–$0.31 of your $5.00 limit\nyour last 4 reviews with it cost $0.03–$0.19".into(),
    )));
    all_sizes("modal-second-opinion", &v);
    let mut v = edited();
    crate::run_events_apply(
        &mut v,
        AgentEvent::SecondOpinion {
            model: "anthropic/claude-opus-5.5".into(),
            connection: "openrouter".into(),
            verdict: Some(false),
            body: "- **blocking** `app/server.js:6`: an empty query returns early, but `find` still runs on `undefined` when `q` is missing.\n- note: the test covers `\"\"` only.\n\nVERDICT: FAIL".into(),
            total_usd: Some(0.04),
        },
    );
    all_sizes("second-opinion", &v);
}

/// Choosing who gives second opinions: the catalog priced for this review,
/// then the user's limit.
#[test]
fn snapshot_reviewer_chooser() {
    let catalog = |id: &str, i: Option<f64>, o: Option<f64>| ryter_core::ModelInfo {
        id: id.into(),
        context_length: Some(256_000),
        input_per_million: i,
        output_per_million: o,
        connection: Some("openrouter".into()),
        created: None,
        tools: Some(true),
    };
    let models = vec![
        catalog("openai/gpt-5.5", Some(5.0), Some(15.0)),
        catalog("x-ai/grok-4.7", Some(3.0), Some(15.0)),
        catalog("z-ai/glm-5.3", Some(0.4), Some(1.6)),
        catalog("moonshotai/kimi-k3", Some(1.0), Some(4.0)),
        catalog("vendor/no-price", None, None),
    ];
    let mut v = edited();
    let mut chooser = crate::panel::models::Models::for_review(&mut v, 18_000, true);
    chooser.set_models(&v, &models);
    v.panels.push(Box::new(chooser.clone()));
    crate::panel::sync_composer(&mut v);
    all_sizes("reviewer-chooser", &v);
    let mut v = edited();
    let mut c = chooser;
    let _ = crate::panel::Panel::key(
        &mut c,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut v,
    );
    v.panels.push(Box::new(c));
    crate::panel::sync_composer(&mut v);
    all_sizes("reviewer-limit", &v);
}

/// An audit stays in the chat as the audit: every row it shows carries the
/// auditor's rule, and a long one folds so the work it audited stays on
/// screen. `^O` shows it whole.
#[test]
fn an_audit_reads_as_the_audit_and_folds() {
    let t = Theme::truecolor_dark();
    let long: String = (1..=30)
        .map(|i| format!("- **blocking** `app/server.js:{i}`: finding {i}\n"))
        .collect::<String>()
        + "\nVERDICT: FAIL";
    let mut v = edited();
    crate::run_events_apply(&mut v, AgentEvent::TurnStarted { turn: 2 });
    crate::run_events_apply(
        &mut v,
        AgentEvent::SecondOpinion {
            model: "z-ai/glm-5.3".into(),
            connection: "openrouter".into(),
            verdict: Some(false),
            body: long,
            total_usd: Some(0.01),
        },
    );
    crate::run_events_apply(
        &mut v,
        AgentEvent::TurnFinished {
            turn: 2,
            tools: 0,
            duration_ms: 3000,
        },
    );
    let screen = render_to_string(&v, 120, 40);
    assert!(
        screen.contains("… 18 more lines · ^O shows it whole"),
        "{screen}"
    );
    // The work it audited is still on screen above it.
    assert!(
        screen.contains("app/server.js") && screen.contains("trim the query"),
        "{screen}"
    );
    let buf = render_buffer(&v, 120, 40, t);
    let rows: Vec<u16> = (0..buf.area.height)
        .filter(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, *y)].symbol().to_string())
                .collect::<String>()
                .contains("finding")
        })
        .collect();
    assert_eq!(rows.len(), 14);
    for y in rows {
        let cell = &buf[(1, y)];
        assert_eq!((cell.symbol(), cell.fg), ("┃", t.audit), "row {y}");
    }
    v.diffs_expanded = true;
    assert!(!render_to_string(&v, 120, 80).contains("more lines · ^O"));
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
        (
            "chat",
            Box::new(|| {
                let mut v = edited();
                crate::run_events_apply(&mut v, AgentEvent::TurnStarted { turn: 2 });
                crate::run_events_apply(&mut v, AgentEvent::SecondOpinion {
                model: "z-ai/glm-5.3".into(),
                connection: "openrouter".into(),
                verdict: Some(true),
                body: "- **note** `app/server.js:6`: an empty query returns `[]`; the test covers it.\n\nVERDICT: PASS".into(),
                total_usd: Some(0.01),
            });
                crate::run_events_apply(
                    &mut v,
                    AgentEvent::TurnFinished {
                        turn: 2,
                        tools: 0,
                        duration_ms: 3000,
                    },
                );
                v
            }),
        ),
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
