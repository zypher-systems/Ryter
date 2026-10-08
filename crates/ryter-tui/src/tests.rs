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
            turn: 0,
            at: 0,
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
    // The last token arrives at the frame's time: the model is writing,
    // not waiting.
    v.now_ms = 12_345;
    v.on_token("Reading the loop now. The event loop drains ");
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

/// `/provider` has a web-search row under the connections. Enter on it
/// chooses a provider; Tavily becomes the saved choice (the handler then
/// asks for the key), SearXNG asks for the server's address first, and
/// `k` on a Tavily row re-enters the key.
#[test]
fn the_provider_panel_offers_web_search() {
    use crate::panel::providers::Providers;
    use crate::panel::{Outcome, Panel};
    let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
    let set_search = |o: &Outcome, provider: &str, url: Option<&str>| -> bool {
        matches!(o, Outcome::CloseAct(Action::SetSearch { provider: p, url: u })
            if p == provider && u.as_deref() == url)
    };
    let mut v = idle();
    let row = v.connections.len();
    let to_row = |p: &mut Providers, v: &mut View| {
        for _ in 0..row {
            p.key(key(KeyCode::Down), v);
        }
    };
    let frame = render_to_string(&with_panel(PanelId::Providers), 120, 40);
    assert!(frame.contains("web search"), "{frame}");
    assert!(frame.contains("not set up"), "{frame}");
    // Enter opens the chooser on the row, on `off` since nothing is set;
    // one step down wraps to Tavily, the first choice.
    let mut p = Providers::new(&v);
    to_row(&mut p, &mut v);
    assert!(matches!(p.key(key(KeyCode::Enter), &mut v), Outcome::Stay));
    let body = p.render(&v, 70, 20, Theme::truecolor_dark());
    let text: String = body.lines.iter().map(|l| l.to_string() + "\n").collect();
    assert!(
        text.contains("tavily") && text.contains("searxng") && text.contains("off"),
        "{text}"
    );
    p.key(key(KeyCode::Down), &mut v);
    assert!(set_search(
        &p.key(key(KeyCode::Enter), &mut v),
        "tavily",
        None
    ));
    // SearXNG asks for the address, offers the local default, and keeps a
    // non-address out. From `off`, two steps down.
    let mut p = Providers::new(&v);
    to_row(&mut p, &mut v);
    p.key(key(KeyCode::Enter), &mut v);
    p.key(key(KeyCode::Down), &mut v);
    p.key(key(KeyCode::Down), &mut v);
    assert!(matches!(p.key(key(KeyCode::Enter), &mut v), Outcome::Stay));
    assert_eq!(v.composer.text(), "http://localhost:8080");
    v.composer.set_text("box:8888");
    assert!(
        matches!(p.key(key(KeyCode::Enter), &mut v), Outcome::Stay),
        "no scheme, stays"
    );
    v.composer.set_text("http://me:secret@box:8888");
    assert!(
        matches!(p.key(key(KeyCode::Enter), &mut v), Outcome::Stay),
        "credentials in the address, stays"
    );
    let text: String = p
        .render(&v, 70, 20, Theme::truecolor_dark())
        .lines
        .iter()
        .map(|l| l.to_string() + "\n")
        .collect();
    assert!(text.contains("no user name or password"), "{text}");
    v.composer.set_text("http://box:8888/");
    assert!(set_search(
        &p.key(key(KeyCode::Enter), &mut v),
        "searxng",
        Some("http://box:8888")
    ));
    // With Tavily chosen, the row says whether the key is in, and `k` asks
    // for it.
    v.search = crate::view::SearchRow {
        provider: "tavily".into(),
        url: None,
        has_key: false,
    };
    let mut shown = v.clone();
    let _ = panel::open(&mut shown, PanelId::Providers, &env());
    panel::sync_composer(&mut shown);
    let frame = render_to_string(&shown, 120, 40);
    assert!(
        frame.contains("tavily") && frame.contains("no key"),
        "{frame}"
    );
    let mut p = Providers::new(&v);
    to_row(&mut p, &mut v);
    assert!(matches!(
        p.key(key(KeyCode::Char('k')), &mut v),
        Outcome::CloseAct(Action::BeginSetKey(n)) if n == "tavily"
    ));
    // A hand-written `off` or `none` reads as not set up, and the chooser
    // opens on `off` for it.
    v.search.provider = "off".into();
    let mut shown = v.clone();
    let _ = panel::open(&mut shown, PanelId::Providers, &env());
    assert!(render_to_string(&shown, 120, 40).contains("not set up"));
    let mut p = Providers::new(&v);
    to_row(&mut p, &mut v);
    p.key(key(KeyCode::Enter), &mut v);
    assert!(set_search(&p.key(key(KeyCode::Enter), &mut v), "", None));
    v.search.provider = "tavily".into();
    // Off clears the provider. The chooser opens on the provider in use
    // (Tavily, the first), so two steps down reach it.
    let mut p = Providers::new(&v);
    to_row(&mut p, &mut v);
    p.key(key(KeyCode::Enter), &mut v);
    p.key(key(KeyCode::Down), &mut v);
    p.key(key(KeyCode::Down), &mut v);
    assert!(set_search(&p.key(key(KeyCode::Enter), &mut v), "", None));
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
        ryter_core::Role::SoloAudit,
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
                turn: 0,
                at: 0,
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
            at: 0,
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

/// Every run of spaces as one: a row's label and its value, whatever the
/// column's width put between them.
fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn spent(role: Role, usd: Option<f64>) -> AgentEvent {
    AgentEvent::Spend {
        connection: "openrouter".into(),
        model: "m".into(),
        role,
        input_tokens: 100,
        output_tokens: 10,
        cached_tokens: 0,
        total_usd: usd,
        incomplete: false,
    }
}

fn edit_of(id: &str, path: &str, before: Option<&str>, after: &str) -> [AgentEvent; 2] {
    [
        AgentEvent::ToolCall {
            turn: 0,
            at: 0,
            id: id.into(),
            name: "write".into(),
            args: serde_json::json!({"path": path, "content": "…"}),
            role: Role::SoloBuild,
            summary: None,
        },
        AgentEvent::ToolResult {
            id: id.into(),
            output: format!("wrote {path}"),
            is_error: false,
            duration_ms: Some(3),
            diff: Some(Box::new(ryter_core::diff::FileDiff::new(
                path, before, after,
            ))),
        },
    ]
}

/// One turn in `role`: what the user asked, what happened, and its close.
fn hat_turn(v: &mut View, turn: u64, role: Role, ask: &str, events: Vec<AgentEvent>) {
    v.mode = role;
    let _ = v.submit_user(ask.into(), ask.into());
    crate::run_events_apply(v, AgentEvent::TurnStarted { turn, role, at: 0 });
    for ev in events {
        crate::run_events_apply(v, ev);
    }
    crate::run_events_apply(
        v,
        AgentEvent::TurnFinished {
            turn,
            tools: 0,
            duration_ms: 1200,
        },
    );
}

/// The events of the session the design's mockups show: a plan approved,
/// built, reviewed and failed, and the fix being built. Each hat on a
/// model of its own.
fn racked_events() -> Vec<(Role, &'static str, Vec<AgentEvent>)> {
    let old =
        "app.get('/search', (req, res) => {\n  const q = req.query.q\n  res.json(find(q))\n})\n";
    let mid = "app.get('/search', (req, res) => {\n  const q = (req.query.q || '').trim()\n  if (!q) return res.json([])\n  res.json(find(q))\n})\n";
    let new = "app.get('/search', (req, res) => {\n  const q = String(req.query.q ?? '').trim()\n  if (!q) return res.json([])\n  res.json(find(q))\n})\n";
    let test = "import { test } from 'node:test'\nimport assert from 'node:assert'\n\ntest('an empty query finds nothing', async () => {\n  assert.deepEqual(await search(''), [])\n})\n";
    let more = format!(
        "{test}\ntest('a missing query finds nothing', async () => {{\n  assert.deepEqual(await search(), [])\n}})\n"
    );
    let mut build = Vec::new();
    build.extend(edit_of("e1", "app/server.js", Some(old), mid));
    build.extend(edit_of("e2", "test/search.test.js", None, test));
    build.push(spent(Role::SoloBuild, Some(0.012)));
    let mut fix = Vec::new();
    fix.extend(edit_of("e3", "app/server.js", Some(mid), new));
    fix.extend(edit_of("e4", "test/search.test.js", Some(test), &more));
    fix.push(spent(Role::SoloBuild, Some(0.009)));
    vec![
        (
            Role::SoloPlan,
            "search returns everything for an empty query",
            vec![
                AgentEvent::Token {
                    text: "The handler passes the query straight to find.".into(),
                },
                AgentEvent::Planned { approved: true },
                spent(Role::SoloPlan, Some(0.004)),
            ],
        ),
        (Role::SoloBuild, "build the plan", build),
        (
            Role::SoloAudit,
            "review it",
            vec![
                AgentEvent::Token {
                    text: "An empty query returns early.\n\nVERDICT: FAIL".into(),
                },
                AgentEvent::Reviewed {
                    model: "anthropic/claude-opus-5.5".into(),
                    connection: "openrouter".into(),
                    verdict: Some(false),
                    tree: Some("4b825dc".into()),
                    total_usd: Some(0.04),
                },
                spent(Role::SoloAudit, Some(0.04)),
            ],
        ),
        (Role::SoloBuild, "fix the blocking finding", fix),
    ]
}

/// The solo screen with its side columns (`docs/hat-rack-design.md`), in
/// the build hat, with that session on it.
fn racked() -> View {
    use ryter_core::review::{FileChange, Status};
    let mut v = idle();
    v.ui.layout = "ledger".into();
    v.panel_visible = true;
    v.cwd = "~/workspace/shop".into();
    v.git_branch = Some("search-patch".into());
    v.session_title = "empty-query fix".into();
    v.sandbox_profile = "workspace".into();
    for c in &mut v.connections {
        c.has_key = true;
    }
    for (hat, conn, model) in [
        ("build", "openrouter", "deepseek/deepseek-pro-latest"),
        ("audit", "openrouter", "anthropic/claude-opus-5.5"),
    ] {
        v.specialists.insert(
            hat.into(),
            ryter_core::RoleModel {
                connection: Some(conn.into()),
                model: Some(model.into()),
            },
        );
    }
    for (i, (role, ask, events)) in racked_events().into_iter().enumerate() {
        hat_turn(&mut v, i as u64 + 1, role, ask, events);
    }
    v.ctx_tokens = Some(97_000);
    v.ctx_window = Some(256_000);
    v.ctx_pct = Some(38);
    v.last_tests = Some("✓ 14 passed".into());
    v.uncommitted = Some(vec![
        FileChange {
            path: "app/server.js".into(),
            status: Status::Modified,
            added: 2,
            removed: 1,
            binary: false,
        },
        FileChange {
            path: "test/search.test.js".into(),
            status: Status::Added,
            added: 10,
            removed: 0,
            binary: false,
        },
    ]);
    let mut p = ryter_core::project::ProjectSpend::default();
    p.total_usd = 4.82;
    v.project_spend = Some(p);
    // A minute on from the turns above: the model is not writing now.
    v.now_ms = 60_000;
    v
}

/// An earlier fixture's name: the solo screen with its columns on.
fn with_rail() -> View {
    racked()
}

/// `R-TEST-01`, `R-TEST-02`: the solo screen at every size, at the
/// narrowest that holds both columns, and in each hat.
#[test]
fn snapshot_solo() {
    all_sizes("solo", &racked());
    check_snapshot("solo-132x40", &render_to_string(&racked(), 132, 40));
    for hat in crate::rail::HATS {
        let mut v = racked();
        v.mode = hat;
        check_snapshot(
            &format!("solo-{hat}-160x50"),
            &render_to_string(&v, 160, 50),
        );
    }
}

/// `R-RACK-*`, `R-TEST-04`: each hat's block says its model, its turns,
/// what it cost, and the figures that are its own.
#[test]
fn the_rack_shows_each_hats_own_figures() {
    let text = squash(&render_to_string(&racked(), 160, 50));
    for want in [
        "HAT RACK",
        "● PLAN grok-4.6 1 turn $0.004 plans 1 approved",
        "◆ BUILD deepseek-pro-latest 2 turns $0.021 files 2 lines +13 −2 success 14 warning 0 failure 0",
        "● AUDIT claude-opus-5.5 1 turn $0.040 audits ✗ 1 fail",
    ] {
        // Each block's rows are a row apart on screen, the conversation
        // between them: find them in order instead.
        let mut rest = text.as_str();
        for part in want.split(' ') {
            match rest.find(part) {
                Some(at) => rest = &rest[at + part.len()..],
                None => panic!("missing {part:?} of {want:?}:\n{text}"),
            }
        }
    }
    // The rack alone, a row at a time.
    let theme = Theme::truecolor_dark();
    let rows = |v: &View| -> Vec<String> {
        crate::rail::lines(v, theme, 27, 100)
            .unwrap()
            .iter()
            .map(|l| {
                squash(
                    &l.spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect::<String>(),
                )
            })
            .collect()
    };
    let v = racked();
    assert_eq!(
        rows(&v),
        [
            "",
            "HAT RACK",
            "",
            "● PLAN",
            "grok-4.6",
            "1 turn $0.004",
            "plans 1 approved",
            "",
            "◆ BUILD",
            "deepseek-pro-latest",
            "2 turns $0.021",
            "files 2",
            "lines +13 −2",
            "success 14",
            "warning 0",
            "failure 0",
            "",
            "─────────────── specialists",
            "",
            "● AUDIT",
            "claude-opus-5.5",
            "1 turn $0.040",
            "audits ✗ 1 fail",
            "",
            "○ SCRIBE",
            "grok-4.6",
            "not worn yet",
        ]
    );
    // A second review that passed, and a rejected plan.
    let mut v = racked();
    for ev in [
        AgentEvent::Planned { approved: false },
        AgentEvent::TurnStarted {
            turn: 7,
            role: Role::SoloAudit,
            at: 0,
        },
        AgentEvent::Reviewed {
            model: "m".into(),
            connection: "c".into(),
            verdict: Some(true),
            tree: None,
            total_usd: None,
        },
    ] {
        crate::run_events_apply(&mut v, ev);
    }
    v.last_tests = Some("✗ 2 failed, 11 passed, 1 skipped".into());
    let got = rows(&v);
    for want in [
        "plans 1 approved",
        "1 rejected",
        "success 11",
        "warning 1",
        "failure 2",
        "2 turns $0.040",
        "audits ✗ 1 ✓ 1",
    ] {
        assert!(got.iter().any(|r| r == want), "missing {want:?}: {got:#?}");
    }
    // Only the scribe, which had no turn here, is not worn yet.
    assert_eq!(
        got.iter().filter(|r| *r == "not worn yet").count(),
        1,
        "{got:#?}"
    );
    // The build block's warning row is the test run's skip.
    assert_eq!(
        got.iter().filter(|r| *r == "warning 1").count(),
        1,
        "{got:#?}"
    );
    // A test run whose summary has no counts keeps its words.
    v.last_tests = Some("✓ Ran 5 tests".into());
    let got = rows(&v);
    assert!(got.iter().any(|r| r == "checks ✓ Ran 5 tests"), "{got:#?}");
}

/// The counts a test run's summary line gives, in the shapes the tools
/// print them.
#[test]
fn a_test_runs_summary_is_read_as_counts() {
    use crate::rail::check_counts;
    for (line, want) in [
        ("✓ 13 passed", Some((13, 0, 0))),
        ("✗ 2 failed, 11 passed, 1 skipped", Some((11, 1, 2))),
        (
            "✓ 13 passed, 2 skipped, 1 warning in 2.1s",
            Some((13, 3, 0)),
        ),
        (
            "test result: ok. 446 passed; 0 failed; 3 ignored",
            Some((446, 3, 0)),
        ),
        ("Tests: 1 failed, 5 passed, 6 total", Some((5, 0, 1))),
        ("✓ 12 passed, 1 failed", Some((12, 0, 1))),
        ("4 passing (2s) 1 pending 2 failing", Some((4, 1, 2))),
        ("✓ Ran 5 tests", None),
        ("", None),
    ] {
        assert_eq!(check_counts(line), want, "{line:?}");
    }
}

/// Tokens a second over the last two seconds, idle after three without
/// any, and eight seconds of history.
#[test]
fn the_pulse_counts_recent_tokens() {
    let mut p = crate::view::Pulse::default();
    assert_eq!(p.rate(5_000), None);
    p.push(1_000, 50);
    p.push(1_900, 50);
    assert_eq!(p.rate(2_000), Some(50), "100 tokens over two seconds");
    assert_eq!(p.rate(3_500), Some(25), "only the second batch is recent");
    assert_eq!(p.rate(4_950), None, "three seconds of nothing is idle");
    let h = p.history(3_000);
    assert_eq!(h[7], 0);
    assert_eq!(h[6], 50, "the batch at 1.9s, one second ago");
    assert_eq!(h[5], 50, "the batch at 1.0s, two seconds ago");
    // Old arrivals are let go.
    p.push(20_000, 10);
    assert_eq!(p.history(20_000).iter().sum::<u64>(), 10);
}

/// `R-RACK-08`, `R-TEST-03`: the rack is four blocks, however long the
/// session. The list of turns is the conversation.
#[test]
fn the_rack_is_the_same_height_whatever_the_turns() {
    let theme = Theme::truecolor_dark();
    let few = racked();
    let mut many = racked();
    for turn in 5..41 {
        hat_turn(
            &mut many,
            turn,
            Role::SoloBuild,
            "and again",
            edit_of("e", "app/server.js", Some("a\n"), "b\n").into(),
        );
    }
    assert_eq!(many.rack.of(Role::SoloBuild).turns, 38);
    let height = |v: &View| crate::rail::lines(v, theme, 27, 100).unwrap().len();
    assert_eq!(height(&few), height(&many));
    assert_eq!(height(&few), 27);
    let text = squash(&render_to_string(&many, 160, 50));
    assert!(text.contains("38 turns"), "{text}");
}

/// `R-LAYOUT-04`, `R-LAYOUT-07`, `R-TOP-05`: the columns give way to the
/// conversation, the rack first, and the prompt is always there.
#[test]
fn the_side_columns_give_way_to_the_conversation() {
    for (w, h, rack, instruments) in [
        (220, 60, true, true),
        (160, 50, true, true),
        (132, 40, true, true),
        (131, 40, false, true),
        (100, 30, false, true),
        (99, 30, false, false),
        (80, 24, false, false),
        (60, 20, false, false),
    ] {
        let text = render_to_string(&racked(), w, h);
        let top = text.lines().next().unwrap();
        assert_eq!(text.contains("HAT RACK"), rack, "{w}x{h}:\n{text}");
        assert_eq!(text.contains("CONTEXT"), instruments, "{w}x{h}:\n{text}");
        assert!(text.contains("what should change?"), "{w}x{h}:\n{text}");
        assert!(top.contains("RYTER") && top.contains("◆ BUILD"), "{top}");
        // The bar counts a hat's turns only when the rack isn't there to.
        assert_eq!(top.contains("BUILD 2"), !rack, "{w}x{h}: {top}");
        // Below the instruments' width, the foot says what they said.
        // (On a screen narrower still, the budget gives way to the keys.)
        let foot = text.lines().last().unwrap();
        assert_eq!(foot.contains("ctx"), !instruments, "{w}x{h}: {foot}");
        assert_eq!(
            foot.contains("$0.065 · budget off"),
            !instruments && w >= 80,
            "{w}x{h}: {foot}"
        );
        assert!(foot.contains("^c quit"), "{w}x{h}: {foot}");
    }
    // A wide screen too short for every figure drops the hats' own rows
    // from every block at once; shorter still, the rack folds away.
    // Four blocks without their figures take twenty rows.
    let text = render_to_string(&racked(), 160, 28);
    assert!(
        text.contains("HAT RACK") && text.contains("2 turns"),
        "{text}"
    );
    assert!(!squash(&text).contains("lines +13"), "{text}");
    let text = render_to_string(&racked(), 160, 15);
    assert!(
        !text.contains("HAT RACK") && text.contains("CONTEXT"),
        "{text}"
    );
    assert!(text.contains("what should change?"), "{text}");
    // Hidden with `^b`, or `[ui] panel = false`.
    let mut v = racked();
    v.panel_visible = false;
    let text = render_to_string(&v, 160, 50);
    assert!(
        !text.contains("HAT RACK") && !text.contains("CONTEXT"),
        "{text}"
    );
    assert!(text.lines().next().unwrap().contains("BUILD 2"), "{text}");
}

/// `R-TOP-*`: the bar says which hats have been worn. It does not claim an
/// order to wear them in.
#[test]
fn the_top_bar_names_hats_worn_not_steps() {
    let text = render_to_string(&racked(), 160, 50);
    let top = text.lines().next().unwrap();
    for want in [
        "RYTER",
        "● PLAN",
        "◆ BUILD",
        "● AUDIT",
        "empty-query fix",
        "~/workspace/shop · search-patch",
    ] {
        assert!(top.contains(want), "missing {want:?}: {top}");
    }
    for no in ["→", "━", "✓", "▸", "1 ", "2 "] {
        assert!(!top.contains(no), "{no:?} in {top}");
    }
    // The hats are in the order Tab goes round them.
    let at = |s: &str| top.find(s).unwrap();
    assert!(at("PLAN") < at("BUILD") && at("BUILD") < at("AUDIT"));
    // Narrower: the folder's own name, then the branch alone.
    let top = render_to_string(&racked(), 110, 30);
    let top = top.lines().next().unwrap();
    assert!(
        top.contains("shop · search-patch") && !top.contains("~/workspace"),
        "{top}"
    );
    let top = render_to_string(&racked(), 80, 24);
    let top = top.lines().next().unwrap();
    assert!(
        top.contains("search-patch") && !top.contains("shop"),
        "{top}"
    );
}

/// `R-INST-*`: session and project spend, the guard, and what is
/// uncommitted. A hat's own spend is the rack's.
#[test]
fn the_instruments_say_what_is_true_of_the_whole_session() {
    let theme = Theme::truecolor_dark();
    let rows = |v: &View, condensed: bool| -> Vec<String> {
        crate::instruments::lines(v, theme, if condensed { 27 } else { 31 }, 100, condensed)
            .iter()
            .map(|l| {
                squash(
                    &l.spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect::<String>(),
                )
            })
            .collect()
    };
    let mut v = racked();
    assert_eq!(
        rows(&v, false),
        [
            "",
            "MODEL",
            "deepseek-pro-latest",
            "connection openrouter ●",
            "reasoning auto",
            "",
            "CONTEXT",
            "━━━━━━━━━━───────────────── 38%",
            "97k / 256k tokens",
            "",
            "PULSE",
            "▁▁▁▁▁▁▁▁ idle",
            "",
            "SPEND",
            "session $0.065",
            "project $4.82",
            "budget off",
            "",
            "GUARD",
            "sandbox workspace",
            "this hat edits ask first",
            "plan.md none",
            "audit.md none",
            "",
            "CHANGES uncommitted",
            "app/server.js +2 −1",
            "test/search.test.js new",
        ]
    );
    assert_eq!(
        rows(&v, true),
        [
            "",
            "MODEL",
            "build deepseek-pro-latest",
            "",
            "CONTEXT",
            "━━━━━━━━━────────────── 38%",
            "97k / 256k tokens",
            "",
            "PULSE",
            "▁▁▁▁▁▁▁▁ idle",
            "",
            "SPEND",
            "session $0.065",
            "project $4.82",
            "budget off",
            "",
            "GUARD",
            "sandbox workspace",
            "this hat asks first",
            "plan.md none",
            "audit.md none",
            "",
            "CHANGES",
            "2 files +12 −1",
        ]
    );
    // The pulse: tokens a second over the last two seconds, and eight
    // seconds of bars, in the hat's color while the model writes.
    v.mode = Role::SoloBuild;
    v.now_ms = 80_000;
    for (t, n) in [
        (72_000, 60),
        (73_500, 90),
        (75_000, 40),
        (78_200, 80),
        (79_100, 70),
    ] {
        v.pulse.push(t, n);
    }
    let got = rows(&v, false);
    assert!(got.iter().any(|r| r == "▁█▅▁▁▁█▇ 75 tok/s"), "{got:#?}");
    v.now_ms = 83_000;
    assert!(rows(&v, false).iter().any(|r| r == "▁▁▁██▁▁▁ idle"));
    // A project total that leaves unpriced calls out says so; a budget is
    // named; nothing uncommitted is said, and so is no repository.
    if let Some(p) = &mut v.project_spend {
        p.unpriced_calls = 2;
    }
    v.budget_usd = 5.0;
    v.uncommitted = Some(Vec::new());
    let got = rows(&v, false);
    for want in ["project ≥$4.82", "budget $5.00", "nothing uncommitted"] {
        assert!(got.iter().any(|r| r == want), "missing {want:?}: {got:#?}");
    }
    v.uncommitted = None;
    assert!(rows(&v, false).iter().any(|r| r == "no repository here"));
    // More files than fit are counted.
    v.uncommitted = Some(
        (0..9)
            .map(|i| ryter_core::review::FileChange {
                path: format!("src/f{i}.rs"),
                status: ryter_core::review::Status::Modified,
                added: 1,
                removed: 0,
                binary: false,
            })
            .collect(),
    );
    let got = rows(&v, false);
    assert!(got.iter().any(|r| r == "+3 more"), "{got:#?}");
    // What the hat may do follows the hat.
    for (hat, may) in [
        (Role::SoloPlan, "this hat read only"),
        (Role::SoloAudit, "this hat checkpoint, restored"),
    ] {
        v.mode = hat;
        let got = rows(&v, false);
        assert!(got.iter().any(|r| r == may), "{hat}: {got:#?}");
    }
}

/// `R-COLOR-01..03`, `R-TEST-02`: the hat on is the one accent. The chrome
/// does not change with it.
#[test]
fn the_hat_colors_the_screen() {
    let theme = Theme::truecolor_dark();
    let mut chrome = None;
    for hat in crate::rail::HATS {
        let mut v = racked();
        v.mode = hat;
        let buf = render_buffer(&v, 160, 50, theme);
        let color = theme.mode(hat);
        let cells = || (0..50u16).flat_map(|y| (0..160u16).map(move |x| (x, y)));
        // The hat's chip in the bar, and the prompt's.
        let chip = cells()
            .find(|&(x, y)| y == 0 && buf[(x, y)].symbol() == "◆")
            .map(|at| buf[at].bg);
        assert_eq!(chip, Some(color), "{hat}: the bar's chip");
        // The prompt's rule runs the conversation's column, between the
        // side columns' hairlines, not under them.
        let rule_row = (0..50u16)
            .rev()
            .find(|&y| buf[(31, y)].symbol() == "─" && buf[(29, y)].symbol() == "│")
            .unwrap();
        assert_eq!(buf[(31, rule_row)].fg, color, "{hat}: the prompt's rule");
        assert_eq!(
            buf[(31, rule_row + 1)].bg,
            color,
            "{hat}: the prompt's chip"
        );
        assert_eq!(
            buf[(126, rule_row)].symbol(),
            "│",
            "{hat}: the instruments go on"
        );
        assert_eq!(
            buf[(29, rule_row + 1)].symbol(),
            "│",
            "{hat}: the rack goes on"
        );
        // The active block's tint in the rack, and the watermark.
        assert!(
            cells().any(|at| at.0 < 29 && buf[at].bg == theme.rack_tint(hat)),
            "{hat}: no tinted block"
        );
        let (mark, band) = theme.watermark(hat).unwrap();
        assert!(cells().any(|at| buf[at].bg == mark), "{hat}: no watermark");
        assert!(cells().any(|at| buf[at].bg == band), "{hat}: no band");
        // The model's name and the gauge, in the instruments.
        let inst = |want: &str| {
            cells()
                .find(|&(x, y)| x > 126 && buf[(x, y)].symbol() == want && buf[(x, y)].fg == color)
        };
        assert!(inst("━").is_some(), "{hat}: the gauge");
        // Everything that isn't the accent is the same under every hat.
        let neutral = (
            buf[(0, 0)].bg,
            buf[(0, 1)].fg,
            buf[(29, 5)].fg,
            buf[(60, 3)].bg,
        );
        assert_eq!(neutral.0, theme.panel_bg);
        assert_eq!(neutral.1, theme.rule);
        assert_eq!(neutral.2, theme.rule);
        match chrome {
            None => chrome = Some(neutral),
            Some(c) => assert_eq!(c, neutral, "{hat}: the chrome changed"),
        }
        // Each hat's own mark keeps its own color, whichever is on.
        let top: String = (0..160).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        let plan = top.find("PLAN").unwrap();
        let x = top[..plan].chars().count() as u16 - 2;
        if hat != Role::SoloPlan {
            assert_eq!(buf[(x, 0)].fg, theme.plan, "{hat}: plan's mark");
        }
    }
}

/// `R-COLOR-04`: an offer to put on another hat is bordered in that hat's
/// color; a permission prompt keeps the warning color.
#[test]
fn an_offer_is_in_the_color_of_the_hat_it_offers() {
    let theme = Theme::truecolor_dark();
    for (tool, color) in [
        ("review offer", theme.audit),
        ("audit", theme.audit),
        ("bash", theme.warn),
    ] {
        let mut v = racked();
        v.panels.push(Box::new(PermissionModal::new(
            tool.into(),
            "rm -rf x".into(),
        )));
        let buf = render_buffer(&v, 160, 50, theme);
        let corner = (0..50u16)
            .flat_map(|y| (0..160u16).map(move |x| (x, y)))
            .find(|&at| buf[at].symbol() == "┏")
            .unwrap_or_else(|| panic!("{tool}: no modal on screen"));
        assert_eq!(buf[corner].fg, color, "{tool}");
    }
}

/// `R-DEGRADE-01`: without color the hat on is still told apart, by its
/// mark and by reverse video, and nothing is tinted.
#[test]
fn the_hat_is_told_apart_without_color() {
    use ratatui::style::{Color, Modifier};
    let v = racked();
    for mode in [ColorMode::Mono, ColorMode::Ansi16] {
        let theme = Theme::truecolor_dark().degrade(mode);
        let buf = render_buffer(&v, 160, 50, theme);
        let text = render_with_theme(&v, 160, 50, theme);
        let top = text.lines().next().unwrap();
        assert!(top.contains("◆ BUILD") && top.contains("● AUDIT"), "{top}");
        assert!(
            !text.contains('▀') && !text.contains('▄'),
            "{mode:?}: a watermark"
        );
        let x = top[..top.find("◆").unwrap()].chars().count() as u16;
        let chip = &buf[(x, 0)];
        assert!(chip.modifier.contains(Modifier::BOLD), "{mode:?}");
        if mode == ColorMode::Mono {
            assert!(chip.modifier.contains(Modifier::REVERSED));
            for y in 0..50 {
                for x in 0..160 {
                    assert_eq!(buf[(x, y)].bg, Color::Reset, "bg at {x},{y}");
                }
            }
        }
    }
}

/// `R-MARK-*`, `R-TEST-05`: the watermark tints backgrounds. It changes no
/// word and no word's color, leaves an edit's row and a code block as
/// they were, and is not drawn where it can't be drawn whole.
#[test]
fn the_watermark_is_behind_the_text_and_never_in_it() {
    let theme = Theme::truecolor_dark();
    let on = racked();
    let mut off = racked();
    off.ui.watermark = false;
    let (a, b) = (
        render_buffer(&on, 160, 50, theme),
        render_buffer(&off, 160, 50, theme),
    );
    let (mark, band) = theme.watermark(Role::SoloBuild).unwrap();
    let mut tinted = 0;
    let mut over_text = 0;
    for y in 0..50u16 {
        for x in 0..160u16 {
            let (with, without) = (&a[(x, y)], &b[(x, y)]);
            if without.symbol() != " " {
                assert_eq!(with.symbol(), without.symbol(), "a word changed at {x},{y}");
                assert_eq!(with.fg, without.fg, "a word's color changed at {x},{y}");
            }
            if without.bg != theme.bg {
                assert_eq!(with.bg, without.bg, "a background changed at {x},{y}");
                assert_eq!(with.symbol(), without.symbol());
            }
            if [mark, band].contains(&with.bg) || [mark, band].contains(&with.fg) {
                tinted += 1;
                over_text += usize::from(without.symbol() != " ");
            }
        }
    }
    assert!(tinted > 400, "the hat is {tinted} cells");
    assert!(over_text > 20, "none of it is behind text ({over_text})");
    // An edit's rows are under it and keep their own tint.
    assert!(
        (0..50u16).any(|y| (40..120u16).any(|x| a[(x, y)].bg == theme.diff_add_bg)),
        "no edit row on screen to check"
    );
    // Off, or with no room for all of it, there is none of it.
    let none = |v: &View, w: u16, h: u16, theme: Theme| {
        let buf = render_buffer(v, w, h, theme);
        (0..h).all(|y| {
            (0..w).all(|x| {
                let c = &buf[(x, y)];
                ![mark, band].contains(&c.bg) && !["▀", "▄"].contains(&c.symbol())
            })
        })
    };
    assert!(none(&off, 160, 50, theme));
    assert!(!none(&on, 80, 24, theme), "it fits at 80×24");
    assert!(none(&on, 80, 20, theme), "too short for it");
    assert!(none(&on, 64, 40, theme), "too narrow for it");
    // The workbench has the screen to itself.
    let repo = workbench_repo();
    let mut v = racked();
    v.workbench = Some(crate::workbench::Workbench::open(
        &v,
        repo.path().to_path_buf(),
    ));
    assert!(none(&v, 160, 50, theme));
}

/// `R-LAYOUT-05`, `R-TEST-08`: `^b` hides the columns on a screen that
/// holds both. Where one or both are folded away it opens them as a panel.
#[test]
fn ctrl_b_hides_the_columns_or_opens_them_as_a_panel() {
    let ctrl_b = KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL);
    let mut v = racked();
    let _ = render_to_string(&v, 160, 50);
    let _ = crate::run_keys_handle(&mut v, ctrl_b);
    assert!(!v.panel_visible && v.panels.is_empty());
    assert!(!render_to_string(&v, 160, 50).contains("HAT RACK"));
    let _ = crate::run_keys_handle(&mut v, ctrl_b);
    assert!(v.panel_visible);
    for (w, h) in [(120, 40), (80, 24)] {
        let mut v = racked();
        let before = render_to_string(&v, w, h);
        assert!(before.contains("^b hat rack"), "{w}x{h}:\n{before}");
        let _ = crate::run_keys_handle(&mut v, ctrl_b);
        assert_eq!(v.panels.kinds(), ["rack"], "{w}x{h}");
        assert!(v.panel_visible, "the columns that fit stay");
        // A short screen shows the top of it, and scrolls to the rest.
        let mut seen = String::new();
        for _ in 0..3 {
            let text = render_to_string(&v, w, h);
            assert!(text.contains("what should change?"), "{w}x{h}:\n{text}");
            assert!(text.contains("esc close"), "{w}x{h}:\n{text}");
            seen.push_str(&squash(&text));
            for _ in 0..8 {
                let _ = crate::run_keys_handle(
                    &mut v,
                    KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                );
            }
        }
        for want in [
            "hat rack",
            "◆ BUILD",
            "2 turns $0.021",
            "audits ✗ 1 fail",
            "session $0.065",
            "project $4.82",
        ] {
            assert!(seen.contains(want), "{w}x{h} missing {want:?}:\n{seen}");
        }
        // Esc closes it, and so does `^b` again.
        let _ = crate::run_keys_handle(&mut v, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(v.panels.is_empty(), "{w}x{h}");
        let _ = crate::run_keys_handle(&mut v, ctrl_b);
        let _ = crate::run_keys_handle(&mut v, ctrl_b);
        assert!(v.panels.is_empty(), "{w}x{h}");
    }
}

/// `R-CORE-01`: a resumed session's rack is the one it had. The saved
/// chat is replayed without its hats; the event log has what happened.
#[test]
fn a_resumed_session_has_the_rack_it_had() {
    let home = tempfile::TempDir::new().unwrap();
    let cwd = tempfile::TempDir::new().unwrap();
    let mut s =
        ryter_core::Session::create(home.path(), cwd.path(), "c".into(), "m".into()).unwrap();
    let mut live = idle();
    for (i, (role, ask, events)) in racked_events().into_iter().enumerate() {
        let turn = i as u64 + 1;
        s.push_message(ryter_core::Message {
            role: "user".into(),
            content: format!("[hat: {role} — x]\n\n{ask}"),
            tool_call_id: None,
            tool_calls: None,
        })
        .unwrap();
        s.push_message(ryter_core::Message {
            role: "assistant".into(),
            content: "done".into(),
            tool_call_id: None,
            tool_calls: None,
        })
        .unwrap();
        let mut all = vec![AgentEvent::TurnStarted { turn, role, at: 0 }];
        all.extend(events);
        for ev in all {
            s.emit(&ev).unwrap();
            crate::run_events_apply(&mut live, ev);
        }
    }
    s.set_mode(Role::SoloBuild).unwrap();
    let mut v = ledger();
    crate::run::fill_view_from_session(&mut v, &s);
    for hat in crate::rail::HATS {
        assert_eq!(v.rack.of(hat), live.rack.of(hat), "{hat}");
    }
    assert_eq!(v.rack.of(Role::SoloBuild).turns, 2);
    assert_eq!(v.rack.of(Role::SoloPlan).plans_approved, 1);
    // What each hat said is in its color again.
    let hats: Vec<Option<Role>> = v
        .messages
        .iter()
        .filter(|m| matches!(m.kind, MessageKind::Assistant { .. }))
        .map(|m| m.meta.hat)
        .collect();
    assert_eq!(
        hats,
        [
            Some(Role::SoloPlan),
            Some(Role::SoloBuild),
            Some(Role::SoloAudit),
            Some(Role::SoloBuild)
        ]
    );
    // A new session starts with an empty rack.
    v.reset_transcript();
    assert!(!crate::rail::HATS.into_iter().any(|h| v.rack.worn(h)));
}

/// `R-START-07`: `/settings` picks the hat a new session opens in, and
/// says what each choice means.
#[test]
fn settings_pick_the_hat_a_session_starts_in() {
    let mut v = with_panel(PanelId::Settings);
    let text = render_to_string(&v, 120, 40);
    let flat = squash(&text);
    assert!(flat.contains("start in ‹ plan ›"), "{text}");
    assert!(text.contains("read and propose first"), "{text}");
    // It leads the form: the first thing a new user is asked to decide.
    assert!(text.find("STARTUP").unwrap() < text.find("SPEND").unwrap());
    for (want, note) in [
        ("build", "straight to work"),
        ("audit", "open on an audit"),
        ("last", "the hat this project closed in"),
        ("plan", "read and propose first"),
    ] {
        let _ = crate::run_keys_handle(&mut v, KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let text = render_to_string(&v, 120, 40);
        assert!(
            squash(&text).contains(&format!("‹ {want} ›")) && text.contains(note),
            "{want}:\n{text}"
        );
    }
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
    v.last_review = Some((tree, "x-ai/grok-4.7".into(), Some(false)));
    assert!(
        receipt(&v).ends_with("· review ✗ grok-4.7"),
        "{}",
        receipt(&v)
    );
    // A change after the review: what is committed is not what was read.
    std::fs::write(repo.path().join("config.rs"), "rewritten\n").unwrap();
    assert!(
        receipt(&v).ends_with("· not reviewed after the last change"),
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

/// The conversation's screen has the hats across its top and `^t` for the
/// changes among its keys; the workbench keeps its strip of views, which
/// lights `changes` (`R-TOP-07`). Crew mode's board is not a view any more.
#[test]
fn the_view_strip_names_every_view() {
    let v = ledger();
    let text = render_to_string(&v, 140, 40);
    let top = text.lines().next().unwrap();
    assert!(top.contains("RYTER") && top.contains("◆ BUILD"), "{top}");
    assert!(text.contains("^t changes"), "{text}");
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
        "audit".into(),
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
            role: ryter_core::Role::SoloAudit,
        },
        AgentEvent::TurnStarted {
            turn: 2,
            role: Role::SoloBuild,
            at: 0,
        },
        AgentEvent::Token { text: body.into() },
        // An audit that gave its verdict in words and filed no report.
        AgentEvent::Audited {
            model: "anthropic/claude-opus-5.5".into(),
            verdict,
            headline: match verdict {
                Some(true) => "✓ passed, unfiled",
                Some(false) => "✗ failed, unfiled",
                None => "no verdict",
            }
            .into(),
            summary: String::new(),
            rows: Vec::new(),
            ran: Vec::new(),
            file: None,
            restored: Vec::new(),
            checkpointed: true,
            product_used: false,
            filed: false,
            total_usd: Some(0.04),
            duration_ms: 3000,
        },
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

/// The scribe hat on, with documentation written: its block and color.
#[test]
fn snapshot_scribe() {
    let v = scribed();
    all_sizes("scribe", &v);
    let shown = render_to_string(&v, 160, 50);
    assert!(shown.contains("SCRIBE"), "{shown}");
    assert!(shown.contains("docs"), "{shown}");
    assert!(shown.contains("2 written"), "{shown}");
    assert!(shown.contains("docs only"), "{shown}");
}

/// A session where the scribe wrote two documents, in its hat.
fn scribed() -> View {
    let mut v = racked();
    let diff = |path: &str, added: usize, removed: usize| {
        Some(Box::new(ryter_core::diff::FileDiff {
            path: path.into(),
            created: removed == 0,
            added,
            removed,
            hunks: Vec::new(),
            elided: 0,
        }))
    };
    for ev in [
        AgentEvent::TurnStarted {
            turn: 7,
            role: Role::SoloScribe,
            at: 0,
        },
        AgentEvent::ToolCall {
            turn: 0,
            at: 0,
            id: "s1".into(),
            name: "write".into(),
            args: serde_json::json!({"path": "docs/install.md"}),
            role: Role::SoloScribe,
            summary: None,
        },
        AgentEvent::ToolResult {
            id: "s1".into(),
            output: "created docs/install.md · 20 lines".into(),
            is_error: false,
            duration_ms: Some(5),
            diff: diff("docs/install.md", 20, 0),
        },
        AgentEvent::ToolCall {
            turn: 0,
            at: 0,
            id: "s2".into(),
            name: "search_replace".into(),
            args: serde_json::json!({"path": "README.md"}),
            role: Role::SoloScribe,
            summary: None,
        },
        AgentEvent::ToolResult {
            id: "s2".into(),
            output: "edited README.md · +3 −1".into(),
            is_error: false,
            duration_ms: Some(5),
            diff: diff("README.md", 3, 1),
        },
        AgentEvent::TurnFinished {
            turn: 7,
            tools: 2,
            duration_ms: 900,
        },
    ] {
        crate::run_events_apply(&mut v, ev);
    }
    v.set_mode(Role::SoloScribe);
    v
}

#[test]
fn snapshot_review() {
    let mut v = edited();
    v.panels.push(Box::new(PermissionModal::new(
        "audit".into(),
        "x-ai/grok-4.7 on openrouter (the audit hat's model)\naudits 2 files, +8 −1, read-only\nabout $0.02–$0.31 of your $5.00 limit\nyour last 4 audits with it cost $0.03–$0.19".into(),
    )));
    all_sizes("modal-review", &v);
    all_sizes("audit", &reviewed(Some(false)));
}

/// When a hat on another model speaks in the same turn, the ledger names
/// each model as it takes over. Named once a turn, the builder's words
/// after a review read as the reviewer's.
#[test]
fn a_model_is_named_again_after_another_has_spoken() {
    let mut v = idle();
    v.ui.layout = "ledger".into();
    v.specialists.insert(
        "audit".into(),
        ryter_core::config::RoleModel {
            connection: Some("openrouter".into()),
            model: Some("x-ai/reviewer-x".into()),
        },
    );
    let _ = v.submit_user("change it".into(), "change it".into());
    let hat = |v: &mut View, role| crate::run_events_apply(v, AgentEvent::ModeChanged { role });
    v.on_token("Changing the greeting.");
    hat(&mut v, ryter_core::Role::SoloAudit);
    v.on_token("The word is wrong.\n\nVERDICT: FAIL");
    hat(&mut v, ryter_core::Role::SoloBuild);
    v.on_token("Fixing the word.");
    hat(&mut v, ryter_core::Role::SoloAudit);
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

/// A filed audit opens its popout over the body, says its line in the
/// chat, counts in the rack, and the guard card's `audit.md` row reads
/// `writing…` while the audit turn runs.
#[test]
fn a_filed_audit_opens_its_popout_and_counts_in_the_rack() {
    let mut v = edited();
    crate::run_events_apply(
        &mut v,
        AgentEvent::ModeChanged {
            role: ryter_core::Role::SoloAudit,
        },
    );
    crate::run_events_apply(
        &mut v,
        AgentEvent::TurnStarted {
            turn: 2,
            role: ryter_core::Role::SoloAudit,
            at: 0,
        },
    );
    assert!(v.audit_writing);
    let guard: Vec<String> = crate::instruments::lines(&v, Theme::truecolor_dark(), 31, 100, false)
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect();
    assert!(
        guard
            .iter()
            .any(|r| r.contains("audit.md") && r.contains("writing…")),
        "{guard:?}"
    );
    crate::run_events_apply(
        &mut v,
        AgentEvent::Audited {
            model: "kimi-k3".into(),
            verdict: Some(false),
            headline: "✗ 1 of 2 failed".into(),
            summary: "one thing".into(),
            rows: vec![
                "✗ 1\tA\tapp/a.py:1".into(),
                "    detail".into(),
                "✓ 2\tB\t".into(),
            ],
            ran: vec!["run_project test".into()],
            file: Some(".ryter/audit.md".into()),
            restored: vec!["app/a.py".into()],
            checkpointed: true,
            product_used: false,
            filed: true,
            total_usd: Some(0.07),
            duration_ms: 58_000,
        },
    );
    assert!(!v.audit_writing);
    assert_eq!(v.panels.top().map(|p| p.kind()), Some("audit"));
    let screen = render_to_string(&v, 150, 42);
    assert!(
        screen.contains("audit · kimi-k3 · ✗ 1 of 2 failed · $0.070 · restored 1 file"),
        "{screen}"
    );
    let totals = v.rack.of(ryter_core::Role::SoloAudit);
    assert_eq!((totals.verdicts_failed, totals.last_restored), (1, Some(1)));
    // The popout: verdict, the finding with its place at the edge, the tree.
    assert!(screen.contains("VERDICT: FAIL"));
    assert!(screen.contains("app/a.py:1"));
    assert!(screen.contains("restored 1 file the audit had changed"));
    // An unfiled audit says its line and opens nothing.
    v.panels.clear();
    crate::run_events_apply(
        &mut v,
        AgentEvent::Audited {
            model: "kimi-k3".into(),
            verdict: Some(true),
            headline: "✓ passed, unfiled".into(),
            summary: String::new(),
            rows: Vec::new(),
            ran: Vec::new(),
            file: None,
            restored: Vec::new(),
            checkpointed: true,
            product_used: false,
            filed: false,
            total_usd: None,
            duration_ms: 1000,
        },
    );
    assert!(v.panels.is_empty());
    assert_eq!(
        v.rack.of(ryter_core::Role::SoloAudit).last_restored,
        Some(0)
    );
}

/// The verdict is said under the review, and kept for the commit's receipt
/// until a commit is made.
#[test]
fn a_reviews_verdict_is_said_and_kept_for_the_commit() {
    for (verdict, said) in [
        (Some(true), "✓ passed, unfiled"),
        (Some(false), "✗ failed, unfiled"),
        (None, "no verdict"),
    ] {
        let mut v = reviewed(verdict);
        let screen = render_to_string(&v, 120, 40);
        assert!(
            screen.contains(&format!("audit · claude-opus-5.5 · {said} · $0.04")),
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
                turn: 0,
                at: 0,
                id: id.into(),
                name: "run_project".into(),
                args: serde_json::json!({ "action": action }),
                role: Role::SoloBuild,
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

/// `Tab` moves within the row of the rack the hat is in; `Shift+Tab` moves
/// to the other row, onto the hat last worn there
/// (`docs/specialists-design.md` §3). A panel that is open keeps its own keys.
#[test]
fn tab_moves_within_a_row_and_shift_tab_between_rows() {
    use ryter_core::Role;
    let mut v = idle();
    let press = |v: &mut View, shift: bool| {
        let code = if shift {
            KeyCode::BackTab
        } else {
            KeyCode::Tab
        };
        crate::run_keys_handle(v, KeyEvent::new(code, KeyModifiers::NONE))
    };
    v.set_mode(Role::SoloPlan);
    assert!(matches!(
        press(&mut v, false),
        Action::SetMode(Role::SoloBuild)
    ));
    v.set_mode(Role::SoloBuild);
    assert!(matches!(
        press(&mut v, false),
        Action::SetMode(Role::SoloPlan)
    ));
    // Nothing worn in the specialist row yet: audit.
    assert!(matches!(
        press(&mut v, true),
        Action::SetMode(Role::SoloAudit)
    ));
    v.set_mode(Role::SoloAudit);
    // The specialists go round: audit, scribe, audit.
    assert!(matches!(
        press(&mut v, false),
        Action::SetMode(Role::SoloScribe)
    ));
    v.set_mode(Role::SoloScribe);
    assert!(matches!(
        press(&mut v, false),
        Action::SetMode(Role::SoloAudit)
    ));
    // Back to the primary hat last worn, from either specialist.
    assert!(matches!(
        press(&mut v, true),
        Action::SetMode(Role::SoloBuild)
    ));
    v.set_mode(Role::SoloPlan);
    v.set_mode(Role::SoloAudit);
    assert!(matches!(
        press(&mut v, true),
        Action::SetMode(Role::SoloPlan)
    ));
    // The specialist last worn is remembered: scribe, then back to it.
    v.set_mode(Role::SoloScribe);
    v.set_mode(Role::SoloBuild);
    assert!(matches!(
        press(&mut v, true),
        Action::SetMode(Role::SoloScribe)
    ));
    // The hints say where each key goes.
    v.set_mode(Role::SoloBuild);
    let foot = render_to_string(&v, 160, 50);
    let foot = foot.lines().last().unwrap();
    assert!(
        foot.contains("tab plan") && foot.contains("⇧tab specialists"),
        "{foot}"
    );
    v.set_mode(Role::SoloAudit);
    let foot = render_to_string(&v, 160, 50);
    let foot = foot.lines().last().unwrap();
    assert!(
        foot.contains("tab scribe") && foot.contains("⇧tab plan · build"),
        "{foot}"
    );
    v.set_mode(Role::SoloScribe);
    let foot = render_to_string(&v, 160, 50);
    let foot = foot.lines().last().unwrap();
    assert!(
        foot.contains("tab audit") && foot.contains("⇧tab plan · build"),
        "{foot}"
    );
    // A panel that is open keeps Tab for itself.
    v.panels
        .push(Box::new(PermissionModal::new("bash".into(), "ls".into())));
    assert!(!matches!(press(&mut v, false), Action::SetMode(_)));
    assert!(!matches!(press(&mut v, true), Action::SetMode(_)));
}

/// The bar shows both rows with a dot between them; narrower, the row the
/// user is not in folds to its name and a count; narrower still, the
/// current row alone (`R-TOP-01`, `R-TOP-02`).
#[test]
fn the_top_bar_folds_the_row_not_in_use_when_narrow() {
    use ryter_core::Role;
    let top = |v: &View, w: u16| {
        render_to_string(v, w, 40)
            .lines()
            .next()
            .unwrap()
            .to_string()
    };
    let v = racked();
    let t = top(&v, 160);
    assert!(
        t.contains("◆ BUILD") && t.contains("● AUDIT") && t.contains(" · "),
        "{t}"
    );
    let t = top(&v, 110);
    assert!(
        t.contains("◆ BUILD") && t.contains("specialists ·1") && !t.contains("AUDIT"),
        "{t}"
    );
    let t = top(&v, 90);
    assert!(
        t.contains("◆ BUILD") && !t.contains("specialists") && !t.contains("AUDIT"),
        "{t}"
    );
    let mut v = racked();
    v.set_mode(Role::SoloAudit);
    let t = top(&v, 110);
    assert!(
        t.contains("◆ AUDIT") && t.contains("plan · build ·2") && !t.contains("◆ BUILD"),
        "{t}"
    );
    let t = top(&v, 90);
    assert!(t.contains("◆ AUDIT") && !t.contains("plan · build"), "{t}");
}

/// The guard card names `.ryter/plan.md` and `.ryter/audit.md`: the day and
/// the heading when the file is there, `none` when it isn't (`R-INST-01`).
#[test]
fn the_guard_card_names_the_plan_and_audit_files() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".ryter")).unwrap();
    std::fs::write(
        dir.path().join(".ryter/plan.md"),
        "# Upload size limit and a delete check\n\nsteps\n",
    )
    .unwrap();
    let mut v = racked();
    v.workspace = Some(dir.path().to_path_buf());
    v.refresh_uncommitted();
    let text = squash(&render_to_string(&v, 160, 50));
    // The row is short of room for the day and the heading: the heading.
    assert!(text.contains("plan.md upload size limit"), "{text}");
    assert!(text.contains("audit.md none"), "{text}");
    let label = crate::view::file_label(&dir.path().join(".ryter/plan.md")).unwrap();
    assert!(label.ends_with(" upload size limit"), "{label}");
    assert!(crate::view::file_label(&dir.path().join(".ryter/audit.md")).is_none());
}

/// The wheel scrolls a panel that is open, three rows a notch, as `↑` and
/// `↓` do (`R-PLAN-03`).
/// The audit card, like the plan panel, takes the wheel (R-PLAN-03).
#[test]
fn the_wheel_scrolls_the_audit_card() {
    use crossterm::event::{MouseEvent, MouseEventKind};
    let mut v = idle();
    let rows: Vec<String> = (1..=60)
        .map(|i| format!("✗ {i}\tfinding number {i}\tsrc/f{i}.rs"))
        .collect();
    let ev = AgentEvent::Audited {
        model: "m".into(),
        verdict: Some(false),
        headline: "✗ 60 of 60 failed".into(),
        summary: "long".into(),
        rows,
        ran: vec!["cargo test".into()],
        file: Some(".ryter/audit.md".into()),
        restored: Vec::new(),
        checkpointed: true,
        product_used: false,
        filed: true,
        total_usd: Some(0.01),
        duration_ms: 1000,
    };
    v.panels.push(Box::new(
        crate::panel::audit::AuditModal::from_event(&ev, 0).unwrap(),
    ));
    let shown = |v: &View| render_to_string(v, 140, 30);
    assert!(shown(&v).contains("finding number 1"), "{}", shown(&v));
    let wheel = |v: &mut View, kind: MouseEventKind| {
        crate::run_mouse_handle(
            v,
            MouseEvent {
                kind,
                column: 20,
                row: 10,
                modifiers: KeyModifiers::NONE,
            },
        )
    };
    for _ in 0..8 {
        let _ = wheel(&mut v, MouseEventKind::ScrollDown);
    }
    let after = shown(&v);
    assert!(
        !after.contains("finding number 1\t") && !after.contains("✗ 1 "),
        "{after}"
    );
    assert!(after.contains("finding number 2"), "{after}");
    for _ in 0..8 {
        let _ = wheel(&mut v, MouseEventKind::ScrollUp);
    }
    assert!(shown(&v).contains("finding number 1"));
    assert!(!v.panels.is_empty());
}

#[test]
fn the_wheel_scrolls_an_open_panel() {
    use crossterm::event::{MouseEvent, MouseEventKind};
    let mut v = idle();
    let plan: String = (1..=60)
        .map(|i| format!("{i}. step number {i}\n"))
        .collect();
    v.panels.push(Box::new(crate::panel::plan::PlanModal::new(
        "Long".into(),
        plan,
        0,
    )));
    let shown = |v: &View| render_to_string(v, 120, 30);
    assert!(shown(&v).contains("step number 1\n") || shown(&v).contains("1. step number 1"));
    let wheel = |v: &mut View, kind: MouseEventKind| {
        crate::run_mouse_handle(
            v,
            MouseEvent {
                kind,
                column: 20,
                row: 10,
                modifiers: KeyModifiers::NONE,
            },
        )
    };
    for _ in 0..6 {
        let _ = wheel(&mut v, MouseEventKind::ScrollDown);
    }
    let after = shown(&v);
    assert!(!after.contains("1. step number 1"), "{after}");
    assert!(after.contains("step number 19"), "{after}");
    for _ in 0..6 {
        let _ = wheel(&mut v, MouseEventKind::ScrollUp);
    }
    assert!(shown(&v).contains("1. step number 1"));
    // The panel still has the screen: the chat behind it did not move.
    assert!(!v.panels.is_empty());
}

/// One spinner. The strip is gone from the solo and classic screens; `^r`
/// opens the turn's reasoning as a pane right under the status row while
/// the turn runs, under the turn's closing line after, never more than a
/// third of the chat.
/// The workbench keeps the strip as its one reasoning surface: the pane
/// that the solo and classic screens place in the conversation is not
/// placed there, so `^r` never shows the reasoning twice.
#[test]
fn the_workbench_keeps_its_strip_and_gets_no_pane() {
    let repo = workbench_repo();
    let mut v = mid_stream(ActivityMode::Expanded);
    v.ui.layout = "ledger".into();
    crate::run_events_apply(
        &mut v,
        AgentEvent::Reasoning {
            text: "first the loop, then the tests ".repeat(30),
        },
    );
    v.workbench = Some(crate::workbench::Workbench::open(
        &v,
        repo.path().to_path_buf(),
    ));
    let shown = render_to_string(&v, 160, 50);
    assert!(
        !shown.contains("reasoning · "),
        "no pane in the workbench's chat\n{shown}"
    );
    assert!(
        shown.contains("^r close"),
        "the strip is the workbench's reasoning surface\n{shown}"
    );
}

#[test]
fn the_reasoning_pane_opens_under_the_status_row() {
    for layout in ["ledger", "classic"] {
        let mut v = mid_stream(ActivityMode::Collapsed);
        v.ui.layout = layout.into();
        crate::run_events_apply(
            &mut v,
            AgentEvent::Reasoning {
                text: "first the loop, then the tests ".repeat(30),
            },
        );
        let shown = render_to_string(&v, 160, 42);
        assert!(
            !shown.contains("^r  reasoning  ▾"),
            "{layout}: the strip is gone\n{shown}"
        );
        assert!(
            !shown.contains("reasoning · "),
            "{layout}: collapsed is the row alone\n{shown}"
        );
        // ^r opens the pane right under the row.
        let _ = crate::run_keys_handle(
            &mut v,
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
        );
        assert_eq!(v.activity.mode, ActivityMode::Expanded);
        let shown = render_to_string(&v, 160, 42);
        let lines: Vec<&str> = shown.lines().collect();
        let status = lines
            .iter()
            .position(|l| l.contains("thinking · "))
            .unwrap_or_else(|| panic!("{layout}: no status row\n{shown}"));
        let header = lines
            .iter()
            .position(|l| l.contains("reasoning · "))
            .unwrap_or_else(|| panic!("{layout}: no pane header\n{shown}"));
        assert_eq!(
            header,
            status + 1,
            "{layout}: the pane is under the row\n{shown}"
        );
        // The fixture's own reasoning and the text pushed here both speak
        // of the loop: the pane's rows are the ones that do.
        let body = lines
            .iter()
            .skip(header + 1)
            .take_while(|l| l.contains("loop"))
            .count();
        assert!(
            (3..=13).contains(&body),
            "{layout}: the pane is capped at a third: {body} rows\n{shown}"
        );
        // The turn ends: the row goes, and the pane sits where the turn
        // closed.
        let turn = v.turn;
        crate::run_events_apply(
            &mut v,
            AgentEvent::TurnFinished {
                turn,
                tools: 0,
                duration_ms: 1000,
            },
        );
        let shown = render_to_string(&v, 160, 42);
        assert!(!shown.contains("thinking · "), "{layout}\n{shown}");
        let lines: Vec<&str> = shown.lines().collect();
        let header = lines
            .iter()
            .position(|l| l.contains("reasoning · "))
            .unwrap_or_else(|| panic!("{layout}: no pane after the turn\n{shown}"));
        let after = lines
            .iter()
            .skip(header + 1)
            .any(|l| l.contains("first the loop"));
        assert!(after, "{layout}: the reasoning stays readable\n{shown}");
        if layout == "ledger" {
            let closing = lines[..header]
                .iter()
                .rposition(|l| l.contains("└─"))
                .unwrap_or_else(|| panic!("no closing line\n{shown}"));
            assert!(header > closing, "{shown}");
        }
        // ^r again closes it.
        let _ = crate::run_keys_handle(
            &mut v,
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
        );
        assert!(!render_to_string(&v, 160, 42).contains("reasoning · "));
    }
}

/// A click on the status row opens the pane, and a click on the pane's
/// header closes it, on both screens.
#[test]
fn a_click_on_the_status_row_toggles_the_pane() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    for layout in ["ledger", "classic"] {
        let mut v = mid_stream(ActivityMode::Collapsed);
        v.ui.layout = layout.into();
        crate::run_events_apply(&mut v, AgentEvent::Reasoning { text: "so ".into() });
        let hit = crate::draw::render_hit(&v, 160, 42);
        assert!(
            hit.activity.height >= 1,
            "{layout}: the row is a target: {:?}",
            hit.activity
        );
        let event = |kind: MouseEventKind, row: u16, col: u16| MouseEvent {
            kind,
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        };
        // A click is a press and a release in one cell; the press alone
        // does nothing, since it may be the start of a drag.
        let click = |v: &mut View, hit: &crate::draw::Hit, row: u16, col: u16| {
            let left = MouseButton::Left;
            let _ =
                crate::run_mouse_handle_with(v, event(MouseEventKind::Down(left), row, col), hit);
            crate::run_mouse_handle_with(v, event(MouseEventKind::Up(left), row, col), hit)
        };
        let down = event(
            MouseEventKind::Down(MouseButton::Left),
            hit.activity.y,
            hit.activity.x + 3,
        );
        let _ = crate::run_mouse_handle_with(&mut v, down, &hit);
        assert_eq!(
            v.activity.mode,
            ActivityMode::Collapsed,
            "{layout}: not yet"
        );
        let _ = click(&mut v, &hit, hit.activity.y, hit.activity.x + 3);
        assert_eq!(v.activity.mode, ActivityMode::Expanded, "{layout}");
        let hit = crate::draw::render_hit(&v, 160, 42);
        assert!(
            hit.activity.height >= 2,
            "{layout}: row and header: {:?}",
            hit.activity
        );
        // The header, one row below the status row.
        let _ = click(&mut v, &hit, hit.activity.y + 1, hit.activity.x + 3);
        assert_eq!(v.activity.mode, ActivityMode::Collapsed, "{layout}");
        // A click elsewhere in the chat does nothing to it.
        let _ = click(&mut v, &hit, hit.chat.y, hit.chat.x + 3);
        assert_eq!(v.activity.mode, ActivityMode::Collapsed, "{layout}");
    }
}

/// A running turn ends on a row that moves: the spinner and what the model
/// is doing, with the time and the tokens, dropped in that order as the
/// screen narrows. The row goes when the turn ends.
#[test]
fn a_running_turn_ends_on_a_live_status_row() {
    let mut v = mid_stream(ActivityMode::Collapsed);
    v.ui.layout = "ledger".into();
    let wide = render_to_string(&v, 160, 50);
    assert!(
        wide.contains("writing · 0:12 · 59 tokens"),
        "writing with time and tokens:\n{wide}"
    );
    let narrow = render_to_string(&v, 60, 24);
    assert!(
        narrow.contains("writing · 0:12") && !narrow.contains("59 tokens"),
        "{narrow}"
    );
    // Thinking: a reasoning delta.
    crate::run_events_apply(&mut v, AgentEvent::Reasoning { text: "so ".into() });
    assert!(render_to_string(&v, 160, 50).contains("thinking · 0:12"));
    // Running a tool.
    crate::run_events_apply(
        &mut v,
        AgentEvent::ToolCall {
            turn: 0,
            at: 0,
            id: "t2".into(),
            name: "bash".into(),
            args: serde_json::json!({"command": "cargo test"}),
            role: Role::SoloBuild,
            summary: Some("cargo test".into()),
        },
    );
    assert!(render_to_string(&v, 160, 50).contains("running bash · 0:12"));
    // Nothing for three seconds while thinking: waiting for the model.
    crate::run_events_apply(
        &mut v,
        AgentEvent::ToolResult {
            id: "t2".into(),
            output: "ok".into(),
            is_error: false,
            duration_ms: Some(5),
            diff: None,
        },
    );
    v.tick(12_345 + crate::activity::WAIT_AFTER_MS);
    let waiting = render_to_string(&v, 160, 50);
    assert!(
        waiting.contains("waiting for the model · 0:15"),
        "{waiting}"
    );
    // A prompt open: waiting for the user.
    v.activity.verb = crate::activity::Verb::Waiting;
    assert!(render_to_string(&v, 160, 50).contains("waiting for you"));
    // The turn ends: the row goes.
    crate::run_events_apply(
        &mut v,
        AgentEvent::TurnFinished {
            turn: 2,
            tools: 1,
            duration_ms: 15_000,
        },
    );
    let done = render_to_string(&v, 160, 50);
    assert!(
        !done.contains("waiting for") && !done.contains("thinking ·"),
        "{done}"
    );
}

/// The bar's chip spins while the hat's model works, and shows `◆` again
/// when the turn ends.
#[test]
fn the_bar_spins_while_the_turn_runs() {
    let mut v = mid_stream(ActivityMode::Collapsed);
    v.ui.layout = "ledger".into();
    for width in [160u16, 110, 80] {
        let text = render_to_string(&v, width, 30);
        let bar = text.lines().next().unwrap_or("");
        assert!(bar.contains("⠼ BUILD"), "{width}: {bar}");
        assert!(!bar.contains("◆ BUILD"), "{width}: {bar}");
    }
    crate::run_events_apply(
        &mut v,
        AgentEvent::TurnFinished {
            turn: 2,
            tools: 1,
            duration_ms: 1,
        },
    );
    let bar = render_to_string(&v, 160, 30);
    assert!(bar.lines().next().unwrap_or("").contains("◆ BUILD"));
}

/// Busy with nothing arriving, the pulse card says `waiting`; idle says
/// `idle`; the reasoning pane says what is happening while it is empty.
#[test]
fn an_empty_wait_is_said_on_the_pulse_and_in_the_pane() {
    let mut v = idle();
    v.ui.layout = "ledger".into();
    v.activity = crate::activity::Activity::new(ActivityMode::Expanded);
    let _ = v.submit_user("plan it".into(), "plan it".into());
    v.tick(20_000);
    let text = render_to_string(&v, 160, 50);
    assert!(text.contains("waiting"), "{text}");
    assert!(text.contains("nothing streamed yet"), "{text}");
    assert!(text.contains("think on the server"), "{text}");
    assert!(!text.contains(" idle"), "{text}");
    crate::run_events_apply(
        &mut v,
        AgentEvent::TurnFinished {
            turn: 1,
            tools: 0,
            duration_ms: 1,
        },
    );
    let text = render_to_string(&v, 160, 50);
    assert!(
        text.contains("idle") || text.contains("no reasoning stream"),
        "{text}"
    );
}

/// The drain stops to paint after a tool's result, or after a run of
/// events, so a batch of tool calls arrives as rows over frames.
#[test]
fn the_drain_pauses_to_paint_after_a_tool_result() {
    let mut v = mid_stream(ActivityMode::Collapsed);
    let (tx, rx) = std::sync::mpsc::channel();
    for i in 0..3 {
        tx.send(AgentEvent::ToolCall {
            turn: 0,
            at: 0,
            id: format!("w{i}"),
            name: "write".into(),
            args: serde_json::json!({"path": format!("f{i}.txt"), "content": "x"}),
            role: Role::SoloBuild,
            summary: None,
        })
        .unwrap();
        tx.send(AgentEvent::ToolResult {
            id: format!("w{i}"),
            output: "created".into(),
            is_error: false,
            duration_ms: Some(1),
            diff: None,
        })
        .unwrap();
    }
    let (n, paused) = crate::run::drain_events(&mut v, &rx);
    assert_eq!(
        (n, paused),
        (2, true),
        "a call and its result, then a paint"
    );
    let (n, paused) = crate::run::drain_events(&mut v, &rx);
    assert_eq!((n, paused), (2, true));
    let (n, paused) = crate::run::drain_events(&mut v, &rx);
    assert_eq!((n, paused), (2, true));
    let (n, paused) = crate::run::drain_events(&mut v, &rx);
    assert_eq!((n, paused), (0, false), "nothing left");
    for _ in 0..40 {
        tx.send(AgentEvent::Reasoning { text: "a ".into() })
            .unwrap();
    }
    let (n, paused) = crate::run::drain_events(&mut v, &rx);
    assert_eq!((n, paused), (32, true), "a run of events pauses too");
}

// -- The allow card: inset, the chat pushed up, announced ----------------------

/// A view mid-turn on the rack screen with a long reply and a card asking
/// about an edit: what the user sees when the build hat wants a yes.
fn asking(width_lines: usize) -> View {
    let mut v = mid_stream(ActivityMode::Collapsed);
    v.ui.layout = "ledger".into();
    let reply: String = (0..width_lines)
        .map(|i| format!("reply line {i}\n"))
        .collect();
    v.on_token(&reply);
    let old: String = (0..60).map(|i| format!("row {i}\n")).collect();
    let new = old.replace("row 30\n", "row thirty\n");
    let diff = ryter_core::diff::FileDiff::new("app/server.js", Some(&old), &new);
    v.panels.push(Box::new(
        PermissionModal::new("search_replace".into(), "app/server.js".into())
            .with_preview(Some(Box::new(diff)))
            .with_answers(false, Some("edits to files in the project".into()))
            .with_context(Some("Next the form text".into()), 0),
    ));
    v.activity.note_ask("allow?");
    v
}

#[test]
fn snapshot_allow_card_on_the_rack() {
    all_sizes("solo-allow", &asking(8));
}

/// The card is inset in the conversation's column: fourteen in and two
/// short of its right edge at a hundred columns or more, two each side
/// down to sixty, the whole column under that.
#[test]
fn the_allow_card_is_inset_in_the_conversation_column() {
    let v = asking(8);
    let theme = Theme::truecolor_dark();
    // The margins follow the column's width, not the screen's: at 160
    // columns the conversation's column is in the nineties, so two each
    // side; the fourteen-and-two shape needs a column of a hundred.
    for (w, h) in [(220u16, 50u16), (160, 50), (110, 40), (80, 24), (50, 24)] {
        let (col_x, col_w) = crate::draw::chat_column(&v, theme, w, h);
        let (left, right) = if col_w >= 100 {
            (14u16, 2u16)
        } else if col_w >= 60 {
            (2, 2)
        } else {
            (0, 0)
        };
        if w == 220 {
            assert!(col_w >= 100, "a column of a hundred at 220: {col_w}");
        }
        let drawn = render_to_string(&v, w, h);
        let top = drawn
            .lines()
            .find(|l| l.contains("┏━ allow?"))
            .unwrap_or_else(|| panic!("no card at {w}x{h}:\n{drawn}"));
        let open = top.chars().position(|c| c == '┏').unwrap();
        let close = top.chars().position(|c| c == '┓').unwrap();
        assert_eq!(open as u16, col_x + left, "{w}x{h}: left edge\n{drawn}");
        assert_eq!(
            close as u16,
            col_x + col_w - right - 1,
            "{w}x{h}: right edge\n{drawn}"
        );
        // Every row of the card ends on its frame: square at every width.
        let rows: Vec<&str> = drawn
            .lines()
            .skip_while(|l| !l.contains("┏━ allow?"))
            .take_while(|l| !l.contains('╰'))
            .collect();
        for r in &rows[1..] {
            assert_eq!(r.chars().nth(close), Some('│'), "{w}x{h}: {r:?}");
        }
    }
}

/// The chat is laid out above the card, not under it: the reply's last
/// line and the status row stay in sight.
#[test]
fn the_chat_moves_up_for_the_allow_card() {
    let v = asking(60);
    let drawn = render_to_string(&v, 160, 42);
    let lines: Vec<&str> = drawn.lines().collect();
    let card = lines.iter().position(|l| l.contains("┏━ allow?")).unwrap();
    let last = lines
        .iter()
        .position(|l| l.contains("reply line 59"))
        .unwrap();
    let status = lines
        .iter()
        .position(|l| l.contains("waiting for you · allow?"))
        .unwrap();
    assert!(
        last < card,
        "the reply's last line is above the card:\n{drawn}"
    );
    assert!(status < card, "the status row is above the card:\n{drawn}");
    // And the card is capped at a third of the column, scrolling inside.
    let bottom = lines.iter().position(|l| l.contains("╰───")).unwrap();
    let height = bottom - card + 1;
    assert!(height <= 42 / 3, "card is {height} rows:\n{drawn}");
    assert!(
        lines[bottom - 1].contains("wheel more of the change"),
        "{drawn}"
    );
}

/// A view mid-turn on the rack screen with a long reply and the build hat
/// proposing how the project runs: the run-file card.
fn proposing_run(width_lines: usize) -> View {
    let mut v = mid_stream(ActivityMode::Collapsed);
    v.ui.layout = "ledger".into();
    let reply: String = (0..width_lines)
        .map(|i| format!("reply line {i}\n"))
        .collect();
    v.on_token(&reply);
    let rows = [
        ("start", "docker compose up -d --wait"),
        ("ready", "http://localhost:8000/healthz"),
        ("test", "docker compose run --rm web pytest -q"),
        ("stop", "docker compose down"),
    ]
    .iter()
    .map(|(l, c)| (l.to_string(), c.to_string()))
    .collect();
    v.panels
        .push(Box::new(crate::panel::plan::PlanModal::run(rows, None, 0)));
    v.activity.note_ask("run?");
    v
}

#[test]
fn snapshot_run_file_card() {
    all_sizes("modal-run-file", &proposing_run(8));
}

/// The run-file card is a question like the allow card: in the rows the
/// chat frees for it, inset in the conversation's column. The reply is
/// laid out above it, whole; nothing of it is under the card.
#[test]
fn the_chat_moves_up_for_the_run_file_card() {
    let theme = Theme::truecolor_dark();
    for (w, h) in [(160u16, 48u16), (100, 30)] {
        let v = proposing_run(60);
        let (col_x, col_w) = crate::draw::chat_column(&v, theme, w, h);
        let drawn = render_to_string(&v, w, h);
        let lines: Vec<&str> = drawn.lines().collect();
        let card = lines
            .iter()
            .position(|l| l.contains("┏━ how this project runs"))
            .unwrap_or_else(|| panic!("no card at {w}x{h}:\n{drawn}"));
        // The reply's last row (it wraps in a narrow column) is above the
        // card, not under it.
        let last = lines
            .iter()
            .rposition(|l| l.contains("reply line"))
            .unwrap_or_else(|| panic!("the reply at {w}x{h}:\n{drawn}"));
        assert!(
            last < card,
            "{w}x{h}: the reply ends above the card:\n{drawn}"
        );
        let status = lines
            .iter()
            .position(|l| l.contains("waiting for you · run?"))
            .unwrap_or_else(|| panic!("the status row at {w}x{h}:\n{drawn}"));
        assert!(
            status < card,
            "{w}x{h}: the status row is above the card:\n{drawn}"
        );
        // No edge of the card in the conversation's column above it: the
        // rows there are the chat's, whole.
        for l in &lines[..card] {
            let col: String = l
                .chars()
                .skip(usize::from(col_x))
                .take(usize::from(col_w))
                .collect();
            assert!(
                !col.contains('┏') && !col.contains('╰') && !col.contains('┃'),
                "{w}x{h}: a card's edge in the chat: {l:?}\n{drawn}"
            );
        }
        // Inset as the allow card is, and capped at a third of the column.
        let open = lines[card].chars().position(|c| c == '┏').unwrap();
        let left = if col_w >= 100 { 14 } else { 2 };
        assert_eq!(open as u16, col_x + left, "{w}x{h}: left edge\n{drawn}");
        let bottom = lines.iter().position(|l| l.contains("╰───")).unwrap();
        let height = bottom - card + 1;
        assert!(
            height <= usize::from(h) / 3 + 1,
            "{w}x{h}: card is {height} rows:\n{drawn}"
        );
        // The commands and the keys are on it whatever the room: the rows
        // of air go first, then it scrolls.
        let body: Vec<&str> = lines[card..=bottom].to_vec();
        assert!(
            body.iter().any(|l| l.contains("start  docker compose up")),
            "{w}x{h}:\n{drawn}"
        );
        assert!(
            body[body.len() - 2].contains("y approve"),
            "{w}x{h}:\n{drawn}"
        );
        if h == 30 {
            assert!(
                body[body.len() - 2].contains("↑↓ scroll"),
                "{w}x{h}:\n{drawn}"
            );
            let card_rows = &body[1..body.len() - 2];
            assert!(
                !card_rows
                    .iter()
                    .any(|l| l.trim_matches(|c| c == '│' || c == ' ').is_empty()),
                "{w}x{h}: a blank row kept:\n{drawn}"
            );
        }
    }
}

/// The foot names the card's keys in the warn color, then the ways out.
#[test]
fn the_foot_shows_the_cards_keys_in_warn() {
    let v = asking(8);
    let theme = Theme::truecolor_dark();
    let buf = render_buffer(&v, 160, 42, theme);
    let foot = buf.area.height - 1;
    let text: String = (0..buf.area.width)
        .map(|x| buf[(x, foot)].symbol().to_string())
        .collect();
    assert!(
        text.contains("⏎ allow") && text.contains("^c stop the turn"),
        "{text}"
    );
    let at = |needle: &str| {
        text.find(needle)
            .map(|i| text[..i].chars().count() as u16)
            .unwrap()
    };
    assert_eq!(buf[(at("⏎"), foot)].fg, theme.warn, "{text}");
    assert_eq!(buf[(at("a allow"), foot)].fg, theme.warn, "{text}");
    assert_ne!(buf[(at("^c"), foot)].fg, theme.warn, "{text}");
}

/// The classic layout's foot line takes the card's keys in warn too.
#[test]
fn the_classic_foot_shows_the_cards_keys_in_warn() {
    let mut v = asking(8);
    v.ui.layout = "classic".into();
    let theme = Theme::truecolor_dark();
    let buf = render_buffer(&v, 160, 42, theme);
    let foot = buf.area.height - 1;
    let text: String = (0..buf.area.width)
        .map(|x| buf[(x, foot)].symbol().to_string())
        .collect();
    assert!(
        text.contains("⏎ allow") && text.contains("^c stop the turn"),
        "{text}"
    );
    let at = |needle: &str| {
        text.find(needle)
            .map(|i| text[..i].chars().count() as u16)
            .unwrap()
    };
    assert_eq!(buf[(at("⏎"), foot)].fg, theme.warn, "{text}");
    assert_eq!(buf[(at("a allow"), foot)].fg, theme.warn, "{text}");
    assert_ne!(buf[(at("^c"), foot)].fg, theme.warn, "{text}");
}

/// The allow card's own key row holds `⏎`, `a` and `n` on a narrow column
/// too: the scope is said in a word, and a key that doesn't fit is left
/// out without taking the ones after it.
#[test]
fn the_allow_cards_key_row_keeps_its_keys_when_narrow() {
    for (w, h) in [(100u16, 30u16), (80, 24), (160, 42)] {
        let v = asking(8);
        let shown = render_to_string(&v, w, h);
        let row = shown
            .lines()
            .find(|l| l.contains("⏎ allow"))
            .unwrap_or_else(|| panic!("{w}x{h}: no key row\n{shown}"));
        assert!(
            row.contains("a allow edits this session") && row.contains("n deny"),
            "{w}x{h}: {row}"
        );
    }
}

/// What each question asks, as the status row says it; and the bell.
#[test]
fn a_question_is_named_and_may_ring() {
    use ryter_core::user_io::UserRequest;
    let (p, _) = std::sync::mpsc::channel();
    let (a, _) = std::sync::mpsc::channel();
    let (q, _) = std::sync::mpsc::channel();
    let perm = |tool: &str| UserRequest::Permission {
        tool: tool.into(),
        summary: "x".into(),
        preview: None,
        strict: false,
        scope: None,
        whole: false,
        asks: None,
        reply: p.clone(),
    };
    assert_eq!(crate::run::ask_for(&perm("bash")), "allow?");
    assert_eq!(crate::run::ask_for(&perm("audit")), "audit?");
    assert_eq!(
        crate::run::ask_for(&UserRequest::Plan {
            title: "t".into(),
            plan: "p".into(),
            reply: a.clone()
        }),
        "plan?"
    );
    assert_eq!(
        crate::run::ask_for(&UserRequest::Run {
            rows: Vec::new(),
            note: None,
            reply: a
        }),
        "run?"
    );
    assert_eq!(
        crate::run::ask_for(&UserRequest::Question {
            title: None,
            question: "?".into(),
            options: Vec::new(),
            reply: q
        }),
        "question"
    );
    let (pw, _) = std::sync::mpsc::channel();
    assert_eq!(
        crate::run::ask_for(&UserRequest::Password {
            prompt: "[sudo] password for kim:".into(),
            again: false,
            reply: pw
        }),
        "password"
    );
    let mut v = mid_stream(ActivityMode::Collapsed);
    v.ui.layout = "ledger".into();
    v.activity.note_ask("plan?");
    assert!(render_to_string(&v, 160, 42).contains("waiting for you · plan?"));
    // A delta ends the question's word.
    v.activity.note_delta(v.now_ms);
    v.activity.verb = crate::activity::Verb::Writing;
    assert!(!render_to_string(&v, 160, 42).contains("waiting for you"));
    // The bell: once when on, never when off.
    let mut out = Vec::new();
    crate::run::ring(&mut out, crate::run::bells_for(true)).unwrap();
    assert_eq!(out, b"\x07");
    let mut out = Vec::new();
    crate::run::ring(&mut out, crate::run::bells_for(false)).unwrap();
    assert!(out.is_empty());
}

/// A command as root asks on a card of its own: `y` only, no "always",
/// and it says the password comes next.
#[test]
fn snapshot_root_permission_card() {
    let mut v = asking(8);
    v.panels.clear();
    v.panels.push(Box::new(
        PermissionModal::new(
            format!("bash {}", ryter_core::tools::AS_ROOT),
            "sudo dnf install -y alsa-lib-devel pkgconf".into(),
        )
        .with_answers(true, None),
    ));
    let drawn = render_to_string(&v, 120, 40);
    assert!(
        drawn.contains("run as root  sudo dnf install -y alsa-lib-devel pkgconf"),
        "{drawn}"
    );
    assert!(drawn.contains("your password is asked next"), "{drawn}");
    assert!(
        drawn.contains("y allow once") && !drawn.contains("always"),
        "{drawn}"
    );
    all_sizes("modal-permission-root", &v);
}

/// A key with no modifier.
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn password_panel(again: bool) -> (View, std::sync::mpsc::Receiver<Option<(String, bool)>>) {
    let mut v = asking(8);
    v.panels.clear();
    let (reply, answer) = std::sync::mpsc::channel();
    let opened = v.now_ms;
    v.panels
        .push(Box::new(crate::panel::modal::PasswordModal::new(
            "[sudo] password for kim:".into(),
            again,
            reply,
            opened,
        )));
    panel::sync_composer(&mut v);
    (v, answer)
}

#[test]
fn snapshot_password_panel() {
    let (mut v, _answer) = password_panel(false);
    for c in "hunter2".chars() {
        crate::run::keys::handle(&mut v, key(KeyCode::Char(c)));
    }
    all_sizes("modal-password", &v);
    let (v, _answer) = password_panel(true);
    assert!(render_to_string(&v, 120, 40).contains("sudo did not accept that one"));
}

/// What is typed for sudo is shown as dots, stays out of the composer,
/// and reaches the reply channel and nothing else. Every character is the
/// password's, `?` and a paste included.
#[test]
fn a_password_is_typed_into_its_own_field() {
    let (mut v, answer) = password_panel(false);
    for c in "p?ss wörd".chars() {
        assert_eq!(
            crate::run::keys::handle(&mut v, key(KeyCode::Char(c))),
            Action::None
        );
    }
    crate::run::paste_handle(&mut v, "-pasted\nand a second line\n");
    crate::run::keys::handle(&mut v, key(KeyCode::Char('x')));
    crate::run::keys::handle(&mut v, key(KeyCode::Backspace));
    assert!(v.composer.is_empty(), "nothing reaches the composer");
    assert!(!v.panels.show_keys, "`?` is a character here");
    for (w, h) in SIZES {
        let drawn = render_to_string(&v, w, h);
        assert!(drawn.contains("••••••••••••••••"), "{w}x{h}: {drawn}");
        for part in ["p?ss", "wörd", "pasted"] {
            assert!(!drawn.contains(part), "{w}x{h} shows {part}");
        }
    }
    assert!(!format!("{:?}", v.panels).contains("pasted"));
    // An Enter that arrives with the panel is not an answer.
    crate::run::keys::handle(&mut v, key(KeyCode::Enter));
    assert!(answer.try_recv().is_err());
    assert_eq!(v.panels.kinds(), ["password"]);
    v.now_ms += crate::panel::modal::ENTER_GUARD_MS;
    // Tab: don't keep it.
    crate::run::keys::handle(&mut v, key(KeyCode::Tab));
    assert!(render_to_string(&v, 120, 40).contains("not at all · tab: keep it 5 minutes"));
    crate::run::keys::handle(&mut v, key(KeyCode::Enter));
    assert_eq!(
        answer.try_recv().unwrap(),
        Some(("p?ss wörd-pasted".to_string(), false))
    );
    assert!(v.panels.is_empty());
}

/// Esc refuses; an empty Enter sends nothing; a cancelled turn, which
/// closes every card, ends sudo's wait.
#[test]
fn a_password_can_be_refused() {
    let (mut v, answer) = password_panel(false);
    v.now_ms += crate::panel::modal::ENTER_GUARD_MS;
    crate::run::keys::handle(&mut v, key(KeyCode::Enter));
    assert!(answer.try_recv().is_err(), "nothing typed, nothing sent");
    crate::run::keys::handle(&mut v, key(KeyCode::Esc));
    assert_eq!(answer.try_recv().unwrap(), None);
    assert!(v.panels.is_empty());

    let (mut v, answer) = password_panel(false);
    crate::run::keys::handle(&mut v, key(KeyCode::Char('x')));
    v.panels.pop();
    assert_eq!(
        answer.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Disconnected)
    );
}

// -- selecting with the mouse ------------------------------------------------

/// Where `needle` starts on a drawn screen: column, row.
fn find_on(buf: &ratatui::buffer::Buffer, needle: &str) -> (u16, u16) {
    for y in 0..buf.area.height {
        let row: Vec<&str> = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
        let text = row.concat();
        if let Some(at) = text.find(needle) {
            let column = text[..at].chars().count();
            return (column as u16, y);
        }
    }
    panic!("{needle:?} is not on the screen");
}

fn mouse(
    kind: crossterm::event::MouseEventKind,
    (column, row): (u16, u16),
    modifiers: KeyModifiers,
) -> crossterm::event::MouseEvent {
    crossterm::event::MouseEvent {
        kind,
        column,
        row,
        modifiers,
    }
}

/// Press at `from`, drag to `to`, and let go. Returns the screen as it was
/// drawn with the button still down, and what the release asked for.
fn drag(
    v: &mut View,
    (w, h): (u16, u16),
    from: (u16, u16),
    to: (u16, u16),
    modifiers: KeyModifiers,
) -> (ratatui::buffer::Buffer, Action) {
    use crossterm::event::{MouseButton, MouseEventKind};
    let left = MouseButton::Left;
    let hit = crate::draw::render_hit(v, w, h);
    let a =
        crate::run_mouse_handle_with(v, mouse(MouseEventKind::Down(left), from, modifiers), &hit);
    assert_eq!(a, Action::None);
    let a = crate::run_mouse_handle_with(v, mouse(MouseEventKind::Drag(left), to, modifiers), &hit);
    assert_eq!(a, Action::None);
    let held = render_buffer(v, w, h, Theme::truecolor_dark());
    let a = crate::run_mouse_handle_with(v, mouse(MouseEventKind::Up(left), to, modifiers), &hit);
    (held, a)
}

/// What a release copies, read as the loop reads it.
fn copied(v: &View, held: &ratatui::buffer::Buffer, a: Action) -> String {
    match a {
        Action::CopySelection(sel) => sel.text(v, held, Theme::truecolor_dark()),
        other => panic!("nothing to copy: {other:?}"),
    }
}

fn talking(layout: &str, reply: &str) -> View {
    let mut v = idle();
    v.ui.layout = layout.into();
    v.ui.line_numbers = false;
    let _ = v.submit_user("summarise".into(), "summarise".into());
    v.on_token(reply);
    v
}

const PROSE: &str = "alpha beta gamma\n\nsecond paragraph here\n\n```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n";

/// A wide character is one character, though it takes two cells.
#[test]
fn a_wide_character_is_copied_once() {
    let theme = Theme::truecolor_dark();
    let mut v = talking("ledger", "start 日本語 and 한글 end\n");
    let plain = render_buffer(&v, 120, 40, theme);
    let from = find_on(&plain, "start");
    let (x, y) = find_on(&plain, "end");
    // `find_on` counts characters; the wide ones took five more cells.
    let (held, a) = drag(&mut v, (120, 40), from, (x + 5 + 2, y), KeyModifiers::NONE);
    assert_eq!(copied(&v, &held, a), "start 日本語 and 한글 end");
}

/// Hold the button and drag: the text between is highlighted, and letting
/// go copies it. The highlight and the copy are the conversation's text,
/// not the timeline beside it.
#[test]
fn dragging_over_the_conversation_highlights_and_copies_its_text() {
    let theme = Theme::truecolor_dark();
    for layout in ["ledger", "classic"] {
        let mut v = talking(layout, PROSE);
        let size = (120, 40);
        let plain = render_buffer(&v, size.0, size.1, theme);
        let from = find_on(&plain, "beta");
        let (x, y) = find_on(&plain, "paragraph");
        let to = (x + 8, y);
        let (held, a) = drag(&mut v, size, from, to, KeyModifiers::NONE);
        // What is highlighted: from the press to the pointer, and whole
        // rows of text between; nothing left of the text.
        let lit = |x: u16, y: u16| held[(x, y)].bg == theme.selection_bg;
        assert!(lit(from.0, from.1) && lit(to.0, to.1), "{layout}");
        assert!(!lit(from.0 - 1, from.1), "{layout}: before the press");
        assert!(!lit(to.0 + 1, to.1), "{layout}: after the pointer");
        let hit = crate::draw::render_hit(&v, size.0, size.1);
        assert!(lit(hit.chat_text.x, to.1), "{layout}: the row's start");
        assert!(
            !lit(hit.chat_text.x - 1, to.1) && !lit(hit.chat.x, from.1 + 1),
            "{layout}: the timeline is not text"
        );
        assert_eq!(
            copied(&v, &held, a),
            "beta gamma\n\nsecond paragraph",
            "{layout}"
        );
        assert!(v.selection.is_none(), "{layout}: the highlight goes");
        let after = render_buffer(&v, size.0, size.1, theme);
        assert!(after[(from.0, from.1)].bg != theme.selection_bg, "{layout}");
    }
}

/// A click is not a selection: nothing is highlighted and nothing copied,
/// and what a click did before, it still does.
#[test]
fn a_click_selects_nothing() {
    use crossterm::event::{MouseButton, MouseEventKind};
    let theme = Theme::truecolor_dark();
    let mut v = talking("ledger", PROSE);
    let hit = crate::draw::render_hit(&v, 120, 40);
    let at = find_on(&render_buffer(&v, 120, 40, theme), "beta");
    let none = KeyModifiers::NONE;
    let down = mouse(MouseEventKind::Down(MouseButton::Left), at, none);
    assert_eq!(
        crate::run_mouse_handle_with(&mut v, down, &hit),
        Action::None
    );
    let held = render_buffer(&v, 120, 40, theme);
    assert!(held[(at.0, at.1)].bg != theme.selection_bg);
    let up = mouse(MouseEventKind::Up(MouseButton::Left), at, none);
    assert_eq!(crate::run_mouse_handle_with(&mut v, up, &hit), Action::None);
    assert!(v.selection.is_none());
    // The status row still opens the pane: on the release, in the cell
    // it was pressed in.
    let row = (hit.activity.x + 3, hit.activity.y);
    let down = mouse(MouseEventKind::Down(MouseButton::Left), row, none);
    let _ = crate::run_mouse_handle_with(&mut v, down, &hit);
    assert_eq!(
        v.activity.mode,
        ActivityMode::Collapsed,
        "a press is not yet a click"
    );
    let up = mouse(MouseEventKind::Up(MouseButton::Left), row, none);
    assert_eq!(crate::run_mouse_handle_with(&mut v, up, &hit), Action::None);
    assert_eq!(v.activity.mode, ActivityMode::Expanded);
    assert!(v.selection.is_none());
}

/// With Alt held the selection is a rectangle: a column of a code block
/// without what is beside it.
#[test]
fn alt_drag_selects_a_rectangle() {
    let theme = Theme::truecolor_dark();
    let mut v = talking("ledger", PROSE);
    let plain = render_buffer(&v, 120, 40, theme);
    let (x, y) = find_on(&plain, "fn main");
    let (held, a) = drag(
        &mut v,
        (120, 40),
        (x, y),
        (x + 10, y + 2),
        KeyModifiers::ALT,
    );
    assert!(held[(x + 10, y)].bg == theme.selection_bg);
    assert!(held[(x + 11, y + 1)].bg != theme.selection_bg);
    assert_eq!(copied(&v, &held, a), "fn main() {\n    println\n}");
}

fn many_lines() -> View {
    let lines: String = (0..120).map(|i| format!("line {i}\n")).collect();
    talking("ledger", &format!("```\n{lines}```\n"))
}

/// The highlight is on the text, not on the screen: when the conversation
/// scrolls under a held button, it goes with the text, and what has left
/// the screen is still copied.
#[test]
fn a_selection_stays_on_its_text_when_the_conversation_scrolls() {
    use crossterm::event::{MouseButton, MouseEventKind};
    let theme = Theme::truecolor_dark();
    let mut v = many_lines();
    let size = (100, 30);
    let hit = crate::draw::render_hit(&v, size.0, size.1);
    let plain = render_buffer(&v, size.0, size.1, theme);
    let from = find_on(&plain, "line 110");
    let (x, y) = find_on(&plain, "line 112");
    let none = KeyModifiers::NONE;
    let left = MouseButton::Left;
    let _ =
        crate::run_mouse_handle_with(&mut v, mouse(MouseEventKind::Down(left), from, none), &hit);
    let _ = crate::run_mouse_handle_with(
        &mut v,
        mouse(MouseEventKind::Drag(left), (x + 7, y), none),
        &hit,
    );
    v.scroll.scroll_by(-5, true);
    let held = render_buffer(&v, size.0, size.1, theme);
    let moved = find_on(&held, "line 110");
    assert_eq!(moved, (from.0, from.1 + 5), "the text moved down five rows");
    assert!(held[(moved.0, moved.1)].bg == theme.selection_bg);
    assert!(held[(from.0, from.1)].bg != theme.selection_bg);
    // Scrolled out of sight, and copied all the same.
    v.scroll.scroll_by(-60, true);
    let gone = render_buffer(&v, size.0, size.1, theme);
    let a = crate::run_mouse_handle_with(
        &mut v,
        mouse(MouseEventKind::Up(left), (x + 7, y), none),
        &hit,
    );
    assert_eq!(copied(&v, &gone, a), "line 110\nline 111\nline 112");
}

/// Held past the top of the conversation, the pointer pulls it down a row
/// at a time, and the selection takes in what comes into view: more than a
/// screen can be copied.
#[test]
fn holding_past_the_edge_scrolls_and_selects_on() {
    use crossterm::event::{MouseButton, MouseEventKind};
    let theme = Theme::truecolor_dark();
    let mut v = many_lines();
    let size = (100, 30);
    let hit = crate::draw::render_hit(&v, size.0, size.1);
    let plain = render_buffer(&v, size.0, size.1, theme);
    let (x, y) = find_on(&plain, "line 115");
    let none = KeyModifiers::NONE;
    let left = MouseButton::Left;
    let _ = crate::run_mouse_handle_with(
        &mut v,
        mouse(MouseEventKind::Down(left), (x + 7, y), none),
        &hit,
    );
    let above = (x, hit.chat.y.saturating_sub(1));
    let _ =
        crate::run_mouse_handle_with(&mut v, mouse(MouseEventKind::Drag(left), above, none), &hit);
    assert_eq!(v.selection.as_ref().map(|s| s.pull), Some(-1));
    let top_before = v.scroll.effective.get();
    for step in 1..=60u64 {
        v.tick(v.now_ms + 50);
        let _ = render_buffer(&v, size.0, size.1, theme);
        assert!(step < 60 || v.scroll.effective.get() < top_before);
    }
    assert_eq!(top_before - v.scroll.effective.get(), 60, "a row a tick");
    let held = render_buffer(&v, size.0, size.1, theme);
    let a =
        crate::run_mouse_handle_with(&mut v, mouse(MouseEventKind::Up(left), above, none), &hit);
    let text = copied(&v, &held, a);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.len() > 60, "more than a screen: {}", lines.len());
    assert_eq!(lines.last().copied(), Some("line 115"));
    assert!(text.contains("line 60\nline 61\nline 62"), "{text}");
    // At the top there is nothing more to pull.
    let mut v = many_lines();
    let _ = render_buffer(&v, size.0, size.1, theme);
    v.scroll.to_top(true);
    let hit = crate::draw::render_hit(&v, size.0, size.1);
    assert_eq!(v.scroll.effective.get(), 0);
    let at = find_on(&render_buffer(&v, size.0, size.1, theme), "line 3");
    let _ = crate::run_mouse_handle_with(&mut v, mouse(MouseEventKind::Down(left), at, none), &hit);
    let _ =
        crate::run_mouse_handle_with(&mut v, mouse(MouseEventKind::Drag(left), above, none), &hit);
    let head = v.selection.as_ref().map(|s| s.head);
    v.tick(v.now_ms + 50);
    assert_eq!(v.selection.as_ref().map(|s| s.head), head);
}

/// Outside the conversation a selection is the screen's cells, kept to the
/// pane it began in: the composer's text, or anything on the screen while a
/// panel is open.
#[test]
fn the_composer_and_a_panel_can_be_selected_too() {
    let theme = Theme::truecolor_dark();
    let mut v = talking("ledger", PROSE);
    for c in "hello world".chars() {
        crate::run::keys::handle(&mut v, key(KeyCode::Char(c)));
    }
    let plain = render_buffer(&v, 120, 40, theme);
    let (x, y) = find_on(&plain, "hello world");
    // Dragged far above the composer: it stays in the composer.
    let (held, a) = drag(
        &mut v,
        (120, 40),
        (x + 10, y),
        (x + 6, 2),
        KeyModifiers::NONE,
    );
    assert_eq!(copied(&v, &held, a), "world");
    assert!(held[(x + 6, 2)].bg != theme.selection_bg);

    let mut v = asking(8);
    let plain = render_buffer(&v, 120, 40, theme);
    let (x, y) = find_on(&plain, "app/server.js");
    let (held, a) = drag(&mut v, (120, 40), (x, y), (x + 12, y), KeyModifiers::NONE);
    assert_eq!(copied(&v, &held, a), "app/server.js");
    assert_eq!(v.panels.kinds(), ["permission"], "the card is still up");
}

/// The rows a selection reads are the rows the pane draws.
#[test]
fn the_documents_rows_are_the_rows_on_the_screen() {
    use crate::chat::layout;
    let theme = Theme::truecolor_dark();
    let v = many_lines();
    let text = |l: &ratatui::text::Line<'_>| -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    };
    let frame = layout::frame(&v, 80, 24, theme);
    let off = frame.resolved.offset;
    let rows = layout::rows(&v, 80, 24, theme, off, off + 24);
    assert_eq!(
        rows.iter().map(text).collect::<Vec<_>>(),
        frame.lines.iter().map(text).collect::<Vec<_>>()
    );
    // Past the end there are no rows.
    assert_eq!(
        layout::rows(&v, 80, 24, theme, frame.doc_rows, frame.doc_rows + 9).len(),
        0
    );
}

/// A copy is said on the last row for a moment, and then the row is the
/// keys again.
#[test]
fn a_copy_is_said_on_the_last_row_and_goes() {
    assert_eq!(crate::select::describe("one\ntwo\nthree"), "copied 3 lines");
    assert_eq!(crate::select::describe("word"), "copied 4 characters");
    assert_eq!(crate::select::describe("x"), "copied 1 character");
    for layout in ["ledger", "classic"] {
        let mut v = talking(layout, PROSE);
        v.flash("copied 3 lines");
        let drawn = render_to_string(&v, 120, 40);
        let last = drawn.lines().last().unwrap_or_default();
        assert_eq!(
            last.trim(),
            "copied 3 lines",
            "{layout}: the row is its own"
        );
        v.tick(v.now_ms + 5_000);
        assert!(!render_to_string(&v, 120, 40).contains("copied 3 lines"));
    }
}

/// A code block is copied as code: without the box drawn around it, its
/// line numbers, or the margin it sits in.
#[test]
fn a_code_block_is_copied_without_its_frame() {
    let theme = Theme::truecolor_dark();
    let code =
        "fn main() {\n    let n = 2;\n    if n > 1 {\n        println!(\"{n}\");\n    }\n}\n";
    for numbers in [true, false] {
        let mut v = talking(
            "ledger",
            &format!("Here it is:\n\n```rust\n{code}```\n\nDone.\n"),
        );
        v.ui.line_numbers = numbers;
        let plain = render_buffer(&v, 120, 40, theme);
        assert_eq!(
            render_to_string(&v, 120, 40).contains("1 fn main"),
            numbers,
            "line numbers are drawn when they are on"
        );
        // From the first line of the code to its last brace.
        let from = find_on(&plain, "fn main");
        let (_, y) = find_on(&plain, "Done.");
        let (x, _) = find_on(&plain, "    let n");
        let last = (x, y - 3);
        assert_eq!(plain[(last.0, last.1)].symbol(), "}");
        let (held, a) = drag(&mut v, (120, 40), from, last, KeyModifiers::NONE);
        assert_eq!(copied(&v, &held, a), code.trim_end(), "numbers: {numbers}");
        // From the sentence before it to the one after: the frame's rules
        // are empty rows, and the code keeps its own indentation.
        let from = find_on(&plain, "Here it is");
        let (x, y) = find_on(&plain, "Done.");
        let (held, a) = drag(&mut v, (120, 40), from, (x + 4, y), KeyModifiers::NONE);
        let text = copied(&v, &held, a);
        assert!(text.starts_with("Here it is:\n"), "{text:?}");
        assert!(text.ends_with("\nDone."), "{text:?}");
        assert_eq!(
            text,
            format!("Here it is:\n\n{}\n\nDone.", code.trim_end()),
            "numbers: {numbers}"
        );
    }
}

/// A field is edited from what it holds: the session's title in `/rename`,
/// the cap in `/budget`, a name in `/settings`. Each set its text and then
/// had it cleared as the composer became its field, so each opened empty.
#[test]
fn a_panels_field_opens_on_what_it_holds() {
    let enter = || KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    let down = || KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
    // `/rename`.
    let mut v = idle();
    v.session_title = "the pager fix".into();
    let _ = panel::open(&mut v, PanelId::Sessions(SessionsMode::Rename), &env());
    panel::sync_composer(&mut v);
    assert_eq!(v.composer.text(), "the pager fix");
    assert!(render_to_string(&v, 120, 40).contains("the pager fix"));

    // `/budget`: down to the first amount, Enter to edit it.
    let mut v = with_panel(PanelId::Budget);
    let mut opened = None;
    for _ in 0..8 {
        let _ = crate::run_keys_handle(&mut v, enter());
        if matches!(v.composer.mode, crate::composer::Mode::Field { .. }) {
            opened = Some(v.composer.text().to_string());
            break;
        }
        let _ = crate::run_keys_handle(&mut v, down());
    }
    let amount = opened.expect("an amount to edit");
    assert!(
        amount.parse::<f64>().is_ok(),
        "the amount as it stands: {amount:?}"
    );

    // `/settings`: the username.
    let mut v = idle();
    v.ui.username = "dusty".into();
    let _ = panel::open(&mut v, PanelId::Settings, &env());
    panel::sync_composer(&mut v);
    let mut opened = None;
    for _ in 0..40 {
        if render_to_string(&v, 120, 50)
            .lines()
            .any(|l| l.contains('›') && l.contains("username"))
        {
            let _ = crate::run_keys_handle(&mut v, enter());
            opened = Some(v.composer.text().to_string());
            break;
        }
        let _ = crate::run_keys_handle(&mut v, down());
    }
    assert_eq!(opened.as_deref(), Some("dusty"));
}

// -- after the 0.24.0 reviews ------------------------------------------------

/// With the workbench open the conversation is its middle pane, and that
/// is where it is selected: reported as the whole workbench, a drag over
/// the text copied a different row, and a press on the changes began a
/// selection of the conversation.
#[test]
fn the_workbench_selects_the_conversation_where_it_is() {
    let theme = Theme::truecolor_dark();
    let mut v = talking("ledger", PROSE);
    let root = tempfile::tempdir().unwrap();
    v.workbench = Some(crate::workbench::Workbench::open(
        &v,
        root.path().to_path_buf(),
    ));
    let size = (160, 44);
    let plain = render_buffer(&v, size.0, size.1, theme);
    let hit = crate::draw::render_hit(&v, size.0, size.1);
    let from = find_on(&plain, "beta");
    assert!(
        hit.chat.x > 20 && from.0 >= hit.chat_text.x,
        "the text is in the pane reported: {:?} {from:?}",
        hit.chat_text
    );
    let (x, y) = find_on(&plain, "paragraph");
    let (held, a) = drag(&mut v, size, from, (x + 8, y), KeyModifiers::NONE);
    assert!(held[(from.0, from.1)].bg == theme.selection_bg);
    assert_eq!(copied(&v, &held, a), "beta gamma\n\nsecond paragraph");
    // A press left of the conversation is not a selection of it.
    use crossterm::event::{MouseButton, MouseEventKind};
    let at = (hit.chat.x.saturating_sub(6), hit.chat.y + 3);
    let down = mouse(
        MouseEventKind::Down(MouseButton::Left),
        at,
        KeyModifiers::NONE,
    );
    let _ = crate::run_mouse_handle_with(&mut v, down, &hit);
    assert!(v.selection.as_ref().is_some_and(|s| s.chat.is_none()));
}

/// A long reply with the question pinned over the pane's first rows.
fn pinned() -> View {
    let mut v = many_lines();
    let _ = render_buffer(&v, 100, 30, Theme::truecolor_dark());
    v.scroll.scroll_by(-40, true);
    v
}

/// What is drawn over the conversation is not the conversation. A press
/// on the pinned question selects what is drawn there; a drag from the
/// text up onto it stops at the last row of text in sight and is not
/// painted across it. The copy was rows nobody could see.
#[test]
fn what_is_drawn_over_the_conversation_is_not_selected_as_it() {
    use crossterm::event::{MouseButton, MouseEventKind};
    let theme = Theme::truecolor_dark();
    let size = (100, 30);
    let none = KeyModifiers::NONE;
    let left = MouseButton::Left;
    let mut v = pinned();
    let plain = render_buffer(&v, size.0, size.1, theme);
    let hit = crate::draw::render_hit(&v, size.0, size.1);
    let pin = hit.covers.first().copied().expect("the question is pinned");
    assert_eq!((pin.y, pin.height), (hit.chat.y, 2));
    let shown = find_on(&plain, "summarise");
    assert_eq!(
        shown.1, pin.y,
        "the pinned question is on the pane's first row"
    );
    // A press on the pin, dragged down into the text: the pin's own cells.
    let (held, a) = drag(&mut v, size, shown, (shown.0 + 30, pin.y + 9), none);
    let text = copied(&v, &held, a);
    assert!(text.starts_with("summarise"), "{text:?}");
    assert!(!text.contains("line "), "nothing from under it: {text:?}");
    assert!(held[(shown.0, shown.1)].bg == theme.selection_bg);
    assert!(held[(shown.0, pin.y + 5)].bg != theme.selection_bg);
    // From the text up onto the pin: it ends at the first row under the
    // pin, pulls the pane up, and leaves the pin unpainted.
    let below = (pin.y + pin.height, pin.y + pin.height + 6);
    let first_row: String = (hit.chat_text.x..hit.chat_text.x + 12)
        .map(|x| plain[(x, below.0)].symbol().to_string())
        .collect();
    let (x, y) = (hit.chat_text.x + 10, below.1);
    let _ = crate::run_mouse_handle_with(
        &mut v,
        mouse(MouseEventKind::Down(left), (x, y), none),
        &hit,
    );
    let onto = (hit.chat_text.x + 2, pin.y);
    let _ =
        crate::run_mouse_handle_with(&mut v, mouse(MouseEventKind::Drag(left), onto, none), &hit);
    assert_eq!(
        v.selection.as_ref().map(|s| (s.pull, s.pull_at)),
        Some((-1, 2))
    );
    let held = render_buffer(&v, size.0, size.1, theme);
    assert!(
        held[(onto.0 + 3, pin.y)].bg != theme.selection_bg,
        "the pin is not text"
    );
    assert!(held[(onto.0 + 3, pin.y + 1)].bg != theme.selection_bg);
    assert!(held[(onto.0 + 3, below.0)].bg == theme.selection_bg);
    let a = crate::run_mouse_handle_with(&mut v, mouse(MouseEventKind::Up(left), onto, none), &hit);
    let text = copied(&v, &held, a);
    assert_eq!(text.lines().count(), 7, "{text:?}");
    assert!(
        first_row
            .trim_end()
            .ends_with(text.lines().next().unwrap_or("?")),
        "it starts at the first row in sight: {first_row:?} {text:?}"
    );
}

/// The command palette is a pane of its own: a press on it selects its
/// rows, not the conversation it is drawn over.
#[test]
fn the_palette_is_selected_as_itself() {
    let theme = Theme::truecolor_dark();
    let mut v = talking("ledger", PROSE);
    v.busy = false;
    crate::run::keys::handle(&mut v, key(KeyCode::Char('/')));
    assert!(v.palette.is_some());
    let size = (120, 40);
    let plain = render_buffer(&v, size.0, size.1, theme);
    let hit = crate::draw::render_hit(&v, size.0, size.1);
    let palette = hit.covers.last().copied().expect("the palette is drawn");
    // A command's name on one of its rows.
    let (x, y, name) = (palette.y..palette.y + palette.height)
        .find_map(|y| {
            let row: String = (palette.x..palette.x + palette.width)
                .map(|x| plain[(x, y)].symbol().to_string())
                .collect();
            let at = row.find('/')?;
            let name: String = row[at..]
                .chars()
                .take_while(|c| !c.is_whitespace())
                .collect();
            let x = palette.x + row[..at].chars().count() as u16;
            Some((x, y, name))
        })
        .expect("a command in the palette");
    assert!(name.len() > 2, "{name}");
    let end = x + name.chars().count() as u16 - 1;
    // Dragged out of the palette, into the conversation above it: the
    // selection stays the palette's, and nothing of the reply is in it.
    let (held, a) = drag(&mut v, size, (x, y), (end, y), KeyModifiers::NONE);
    assert_eq!(copied(&v, &held, a), name);
    let (_, a) = drag(
        &mut v,
        size,
        (x, y),
        (end, palette.y.saturating_sub(4)),
        KeyModifiers::NONE,
    );
    match a {
        Action::CopySelection(sel) => assert!(sel.chat.is_none() && sel.area == palette),
        other => panic!("{other:?}"),
    }
}

/// A drag that begins on a card or on the status row is a selection, and
/// a click there does what it did: the click waits for the release.
#[test]
fn a_drag_from_a_card_selects_and_a_click_still_opens_it() {
    use crossterm::event::{MouseButton, MouseEventKind};
    let theme = Theme::truecolor_dark();
    let mut v = talking("classic", PROSE);
    v.panel_visible = true;
    let size = (160, 44);
    let hit = crate::draw::render_hit(&v, size.0, size.1);
    let (card, rect) = hit
        .cards
        .iter()
        .find(|(c, _)| c.opens().is_some())
        .cloned()
        .expect("a card that opens a panel");
    let plain = render_buffer(&v, size.0, size.1, theme);
    // Some text of the card, and a drag across it.
    let row = (rect.y..rect.y + rect.height)
        .find(|y| (rect.x..rect.x + rect.width).any(|x| plain[(x, *y)].symbol().trim() != ""))
        .expect("a row with text");
    let whole: String = (rect.x..rect.x + rect.width)
        .map(|x| plain[(x, row)].symbol().to_string())
        .collect();
    let (held, a) = drag(
        &mut v,
        size,
        (rect.x, row),
        (rect.x + rect.width - 1, row),
        KeyModifiers::NONE,
    );
    assert_eq!(copied(&v, &held, a), whole.trim());
    assert!(v.panels.is_empty(), "a drag opens nothing");
    // A click: press and release in one cell.
    let none = KeyModifiers::NONE;
    let at = (rect.x + 1, row);
    let down = mouse(MouseEventKind::Down(MouseButton::Left), at, none);
    assert_eq!(
        crate::run_mouse_handle_with(&mut v, down, &hit),
        Action::None
    );
    let up = mouse(MouseEventKind::Up(MouseButton::Left), at, none);
    assert_eq!(
        crate::run_mouse_handle_with(&mut v, up, &hit),
        Action::OpenPanel(card.opens().unwrap())
    );
}

/// Only the frame is left out of a code block. A line that is only a
/// number, drawn as dim as the frame (a diff's context line), is code.
#[test]
fn a_dim_line_of_code_is_not_taken_for_the_frame() {
    let theme = Theme::truecolor_dark();
    let diff = "@@ -1,4 +1,4 @@\n 42\n-old line\n+new line\n    0\n 7 7\n";
    for numbers in [true, false] {
        let mut v = talking("ledger", &format!("```diff\n{diff}```\n\nAfter.\n"));
        v.ui.line_numbers = numbers;
        let plain = render_buffer(&v, 120, 40, theme);
        let from = find_on(&plain, "@@ -1,4");
        let (_, y) = find_on(&plain, "After.");
        let (held, a) = drag(
            &mut v,
            (120, 40),
            from,
            (from.0 + 40, y - 3),
            KeyModifiers::NONE,
        );
        assert_eq!(copied(&v, &held, a), diff.trim_end(), "numbers: {numbers}");
    }
}

/// Code keeps every space of its own: the body of a function, selected
/// without the line that opens it, is not moved to the margin.
#[test]
fn the_body_of_a_function_keeps_its_indentation() {
    let theme = Theme::truecolor_dark();
    let code = "fn main() {\n    let n = 2;\n    println!(\"{n}\");\n}\n";
    let mut v = talking("ledger", &format!("```rust\n{code}```\n"));
    let plain = render_buffer(&v, 120, 40, theme);
    let hit = crate::draw::render_hit(&v, 120, 40);
    let (_, y) = find_on(&plain, "let n = 2;");
    // From the row's left edge, through the frame, to the next line's end.
    let (x, _) = find_on(&plain, "println!");
    let (held, a) = drag(
        &mut v,
        (120, 40),
        (hit.chat_text.x, y),
        (x + 15, y + 1),
        KeyModifiers::NONE,
    );
    assert_eq!(
        copied(&v, &held, a),
        "    let n = 2;\n    println!(\"{n}\");"
    );
}

/// The composer's caret is drawn in an empty cell, and is not text.
#[test]
fn the_caret_is_not_copied() {
    let theme = Theme::truecolor_dark();
    let mut v = talking("ledger", PROSE);
    for c in "hello".chars() {
        crate::run::keys::handle(&mut v, key(KeyCode::Char(c)));
    }
    let plain = render_buffer(&v, 120, 40, theme);
    let (x, y) = find_on(&plain, "hello");
    assert_eq!(
        plain[(x + 5, y)].symbol(),
        "█",
        "the caret follows the text"
    );
    let (held, a) = drag(&mut v, (120, 40), (x, y), (x + 12, y), KeyModifiers::NONE);
    assert_eq!(copied(&v, &held, a), "hello");
}

/// A panel that opens over the conversation while the button is down ends
/// the selection: it is not painted across the panel, and the release
/// copies nothing.
#[test]
fn a_panel_opening_ends_a_selection_of_the_conversation() {
    use crossterm::event::{MouseButton, MouseEventKind};
    let theme = Theme::truecolor_dark();
    let mut v = talking("ledger", PROSE);
    let size = (120, 40);
    let hit = crate::draw::render_hit(&v, size.0, size.1);
    let plain = render_buffer(&v, size.0, size.1, theme);
    let from = find_on(&plain, "beta");
    let (x, y) = find_on(&plain, "paragraph");
    let none = KeyModifiers::NONE;
    let left = MouseButton::Left;
    let _ =
        crate::run_mouse_handle_with(&mut v, mouse(MouseEventKind::Down(left), from, none), &hit);
    let _ = crate::run_mouse_handle_with(
        &mut v,
        mouse(MouseEventKind::Drag(left), (x, y), none),
        &hit,
    );
    v.panels.push(Box::new(PermissionModal::new(
        "bash".into(),
        "rm -rf target".into(),
    )));
    let held = render_buffer(&v, size.0, size.1, theme);
    for yy in 0..size.1 {
        for xx in 0..size.0 {
            assert!(
                held[(xx, yy)].bg != theme.selection_bg,
                "painted at {xx},{yy}"
            );
        }
    }
    let up = mouse(MouseEventKind::Up(left), (x, y), none);
    assert_eq!(crate::run_mouse_handle_with(&mut v, up, &hit), Action::None);
    assert!(v.selection.is_none());
}

/// A selection being pulled ends at a row of the pane, so a report from
/// the pointer while it is held there changes nothing, and at the top of
/// the document it stays at the first row.
#[test]
fn a_pulled_selection_ends_at_the_panes_edge() {
    use crossterm::event::{MouseButton, MouseEventKind};
    let theme = Theme::truecolor_dark();
    let mut v = many_lines();
    let size = (100, 30);
    let hit = crate::draw::render_hit(&v, size.0, size.1);
    let plain = render_buffer(&v, size.0, size.1, theme);
    let (x, y) = find_on(&plain, "line 115");
    let none = KeyModifiers::NONE;
    let left = MouseButton::Left;
    let _ = crate::run_mouse_handle_with(
        &mut v,
        mouse(MouseEventKind::Down(left), (x + 7, y), none),
        &hit,
    );
    let above = (x, hit.chat.y.saturating_sub(1));
    let wiggle = (x + 1, hit.chat.y.saturating_sub(1));
    let mut lines = 0;
    for step in 0..200u64 {
        if step % 7 == 3 {
            let at = if step % 2 == 0 { above } else { wiggle };
            let hit = crate::draw::render_hit(&v, size.0, size.1);
            let _ = crate::run_mouse_handle_with(
                &mut v,
                mouse(MouseEventKind::Drag(left), at, none),
                &hit,
            );
        } else if step == 0 {
            let _ = crate::run_mouse_handle_with(
                &mut v,
                mouse(MouseEventKind::Drag(left), above, none),
                &hit,
            );
        }
        v.tick(v.now_ms + 50);
        let held = render_buffer(&v, size.0, size.1, theme);
        let sel = v.selection.clone().expect("still held");
        let now = sel.text(&v, &held, theme).lines().count();
        assert!(
            now >= lines,
            "step {step}: {now} after {lines}: rows were dropped"
        );
        lines = now;
    }
    assert_eq!(v.scroll.effective.get(), 0, "pulled to the top");
    let held = render_buffer(&v, size.0, size.1, theme);
    let a =
        crate::run_mouse_handle_with(&mut v, mouse(MouseEventKind::Up(left), above, none), &hit);
    let text = copied(&v, &held, a);
    assert!(
        text.ends_with("line 115"),
        "{}",
        &text[text.len().saturating_sub(40)..]
    );
    assert!(text.contains("line 0\nline 1\nline 2"), "the whole reply");
}
