//! Popout panel framework: stack, geometry, chrome (`R-POP-01..09`, `R-MOD-02`).
//!
//! Each panel module owns its state, key handling, and rendering. The
//! framework draws the shared chrome, dims what is behind, and drops a shadow.

pub mod agents;
pub mod chrome;
pub mod context;
pub mod crew;
pub mod doctor;
pub mod help;
pub mod hooks;
pub mod mcp;
pub mod modal;
pub mod models;
pub mod phase;
pub mod providers;
pub mod sessions;
pub mod settings;
pub mod skills;
pub mod spend;
pub mod theme;
pub mod toggles;
pub mod widgets;

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ryter_core::{AgentEvent, SandboxProfile};

use crate::action::{Action, PanelId};
use crate::theme::Theme;
use crate::view::View;

/// Result of a key press inside a panel.
pub enum Outcome {
    /// Consumed; nothing else.
    Stay,
    /// Pop this panel.
    Close,
    /// Open a child on top (`R-POP-08`).
    Push(Box<dyn Panel>),
    /// Hand an action to the loop; panel stays.
    Act(Action),
    /// Pop and act.
    CloseAct(Action),
    /// Push a child and act (e.g. `/crew` → `/models` + fetch).
    PushAct(Box<dyn Panel>, Action),
}

/// Rendered body.
#[derive(Debug, Default)]
pub struct Body {
    /// Rows to paint (already windowed to the height given).
    pub lines: Vec<Line<'static>>,
    /// `(first visible row, total rows)` for the scrollbar (`R-POP-06`).
    pub scroll: Option<(usize, usize)>,
}

/// Modal styling (`R-POP-75`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalKind {
    /// Permission: `warn` border.
    Permission,
    /// Ask: `accent` border.
    Ask,
}

/// Out-of-band results from background work (connectivity tests, doctor).
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    /// `t` in `/provider`.
    ConnTest {
        /// Connection.
        name: String,
        /// Latency or error.
        result: Result<u64, String>,
    },
    /// `/doctor` finished.
    Doctor(Vec<(String, String, String)>),
    /// A file export finished (`/spend e`, `/doctor c`).
    Exported(String),
    /// Models arrived for a specific picker (`/models`, `/crew`).
    Models(Vec<ryter_core::ModelInfo>),
    /// Crew presets on disk changed.
    PresetsChanged(Vec<String>),
    /// Sessions list refreshed (after rename/delete).
    SessionsChanged,
}

/// A popout.
pub trait Panel {
    /// Stable kind name (tests, Debug).
    fn kind(&self) -> &'static str;
    /// Title in the top-left border.
    fn title(&self, view: &View) -> String;
    /// Counter / status in the top-right border.
    fn status(&self, _view: &View) -> String {
        String::new()
    }
    /// Key legend in the bottom border.
    fn legend(&self, view: &View) -> String;
    /// `(preferred width, content rows)`.
    fn size(&self, view: &View) -> (u16, u16);
    /// When `Some`, the composer is this panel's input field (`R-COMP-17`).
    fn input(&self, _view: &View) -> Option<String> {
        None
    }
    /// Render the body for `width × height`.
    fn render(&self, view: &View, width: u16, height: u16, theme: Theme) -> Body;
    /// Handle a key.
    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome;
    /// Agent event hook.
    fn on_event(&mut self, _ev: &AgentEvent, _view: &mut View) {}
    /// Background notice hook.
    fn on_notice(&mut self, _n: &Notice, _view: &mut View) {}
    /// Modal interrupts render with heavier chrome.
    fn modal(&self) -> Option<ModalKind> {
        None
    }
    /// Clone into a box (`View: Clone`).
    fn box_clone(&self) -> Box<dyn Panel>;
}

/// Stack of open panels; the last is focused.
#[derive(Default)]
pub struct PanelStack {
    /// Bottom to top.
    pub stack: Vec<Box<dyn Panel>>,
    /// `?` pressed: legend shows universal keys.
    pub show_keys: bool,
}

impl Clone for PanelStack {
    fn clone(&self) -> Self {
        Self {
            stack: self.stack.iter().map(|p| p.box_clone()).collect(),
            show_keys: self.show_keys,
        }
    }
}

impl std::fmt::Debug for PanelStack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.stack.iter().map(|p| p.kind()))
            .finish()
    }
}

impl PanelStack {
    /// Open a panel on top.
    pub fn push(&mut self, p: Box<dyn Panel>) {
        self.show_keys = false;
        self.stack.push(p);
    }

    /// Close the top panel.
    pub fn pop(&mut self) -> Option<Box<dyn Panel>> {
        self.show_keys = false;
        self.stack.pop()
    }

    /// Close everything.
    pub fn clear(&mut self) {
        self.stack.clear();
        self.show_keys = false;
    }

    /// Nothing open.
    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// Focused panel.
    pub fn top(&self) -> Option<&dyn Panel> {
        self.stack.last().map(|b| b.as_ref())
    }

    /// Kinds, bottom to top.
    pub fn kinds(&self) -> Vec<&'static str> {
        self.stack.iter().map(|p| p.kind()).collect()
    }

    /// True when a modal interrupt is on top (`R-POP-79`).
    pub fn has_modal(&self) -> bool {
        self.stack.iter().any(|p| p.modal().is_some())
    }

    /// True when the focused panel owns the composer.
    pub fn wants_input(&self, view: &View) -> Option<String> {
        self.top().and_then(|p| p.input(view))
    }
}

/// Dispatch a key to the focused panel.
pub fn handle_key(view: &mut View, key: KeyEvent) -> Action {
    let Some(mut top) = view.panels.stack.pop() else {
        return Action::None;
    };
    if key.code == KeyCode::Char('?') && top.input(view).is_none() {
        view.panels.stack.push(top);
        view.panels.show_keys = !view.panels.show_keys;
        return Action::None;
    }
    let out = top.key(key, view);
    match out {
        Outcome::Stay => {
            view.panels.stack.push(top);
            Action::None
        }
        Outcome::Close => {
            view.panels.show_keys = false;
            Action::None
        }
        Outcome::Push(child) => {
            view.panels.stack.push(top);
            view.panels.push(child);
            Action::None
        }
        Outcome::Act(a) => {
            view.panels.stack.push(top);
            a
        }
        Outcome::CloseAct(a) => {
            view.panels.show_keys = false;
            a
        }
        Outcome::PushAct(child, a) => {
            view.panels.stack.push(top);
            view.panels.push(child);
            a
        }
    }
}

/// Broadcast an agent event to every open panel.
pub fn on_event(view: &mut View, ev: &AgentEvent) {
    let mut stack = std::mem::take(&mut view.panels.stack);
    for p in &mut stack {
        p.on_event(ev, view);
    }
    // A panel may have pushed while we held the stack; keep any additions.
    let added = std::mem::take(&mut view.panels.stack);
    stack.extend(added);
    view.panels.stack = stack;
}

/// Broadcast a notice to every open panel.
pub fn on_notice(view: &mut View, n: &Notice) {
    let mut stack = std::mem::take(&mut view.panels.stack);
    for p in &mut stack {
        p.on_notice(n, view);
    }
    let added = std::mem::take(&mut view.panels.stack);
    stack.extend(added);
    view.panels.stack = stack;
}

/// Keep the composer's field mode in step with the focused panel.
pub fn sync_composer(view: &mut View) {
    let want = view.panels.wants_input(view);
    match (&view.composer.mode, want) {
        (crate::composer::Mode::Field { label }, Some(l)) if *label == l => {}
        (crate::composer::Mode::Field { .. }, Some(l)) => {
            view.composer.mode = crate::composer::Mode::Field { label: l };
        }
        (_, Some(l)) => view.composer.begin_field(l, ""),
        (crate::composer::Mode::Field { .. }, None) => view.composer.end_special(),
        _ => {}
    }
}

/// Filesystem context for panels that read disk on open.
#[derive(Debug, Clone)]
pub struct PanelEnv {
    /// `~/.ryter`.
    pub home: PathBuf,
    /// Project cwd.
    pub cwd: PathBuf,
    /// Trusted project.
    pub trusted: bool,
    /// Sandbox profile in effect.
    pub sandbox: SandboxProfile,
}

/// Construct and push the panel for `id`.
pub fn open(view: &mut View, id: PanelId, env: &PanelEnv) -> Action {
    view.palette = None;
    let p: Box<dyn Panel> = match id {
        PanelId::Providers => Box::new(providers::Providers::new(view)),
        PanelId::Models => Box::new(models::Models::new(view, None)),
        PanelId::Crew => Box::new(crew::Crew::new(env)),
        PanelId::Agents => Box::new(agents::Agents::default()),
        PanelId::Sessions(mode) => Box::new(sessions::Sessions::new(view, env, mode)),
        PanelId::Spend => Box::new(spend::Spend::default()),
        PanelId::Settings => Box::new(settings::Settings::new(view)),
        PanelId::Theme => Box::new(theme::ThemePicker::new(view)),
        PanelId::Tools => Box::new(toggles::Toggles::tools(view)),
        PanelId::Auditor => Box::new(toggles::Toggles::auditor(view)),
        PanelId::Mcp => Box::new(mcp::Mcp::default()),
        PanelId::Skills => Box::new(skills::Skills::default()),
        PanelId::Hooks => Box::new(hooks::Hooks::default()),
        PanelId::Phase => Box::new(phase::PhasePicker::new(view)),
        PanelId::Context => Box::new(context::Context::default()),
        PanelId::Help => Box::new(help::Help::default()),
        PanelId::Doctor => Box::new(doctor::Doctor::new(env)),
    };
    view.panels.push(p);
    Action::None
}

/// Panel rectangle (`R-POP-01`, `R-POP-02`). `body` is the area above the
/// composer; `full` is the whole terminal.
pub fn rect(full: Rect, body: Rect, pref_w: u16, content_rows: u16, modal: bool) -> Rect {
    if full.width < 60 || full.height < 20 {
        return body;
    }
    let w = pref_w.clamp(40, full.width.saturating_sub(8));
    // A panel may use the body it now owns. Reserving ten rows of the terminal
    // meant `/help` got 20 rows for 40 rows of keybindings at 100x30 and
    // clipped the first thing a new user reads; two rows of breathing room
    // above and below is enough to still read as floating.
    let max_h = body.height.saturating_sub(2).max(8);
    let h = (content_rows + 2).clamp(8, max_h);
    let x = body.x + body.width.saturating_sub(w) / 2;
    let y = body.y + body.height.saturating_sub(h) / if modal { 4 } else { 3 };
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

/// Dim everything in `area` one step (`R-POP-04`).
pub fn dim_region(frame: &mut Frame, area: Rect, theme: Theme) {
    let buf = frame.buffer_mut();
    for y in area.y..area.y.saturating_add(area.height) {
        for x in area.x..area.x.saturating_add(area.width) {
            if x >= buf.area.width || y >= buf.area.height {
                continue;
            }
            let cell = &mut buf[(x, y)];
            let mut st = cell.style();
            st.fg = Some(theme.dim);
            st = st.remove_modifier(Modifier::BOLD | Modifier::UNDERLINED);
            if theme.mode == crate::theme::ColorMode::Mono {
                st = st.add_modifier(Modifier::DIM);
            }
            cell.set_style(st);
        }
    }
}

/// One-column / one-row shadow (`R-POP-05`).
pub fn shadow(frame: &mut Frame, area: Rect, theme: Theme) {
    let shade = crate::theme::darken(theme.bg, 0.5);
    let buf = frame.buffer_mut();
    let (bw, bh) = (buf.area.width, buf.area.height);
    let right = area.x.saturating_add(area.width);
    let bottom = area.y.saturating_add(area.height);
    let mut paint = |x: u16, y: u16| {
        if x < bw && y < bh {
            let cell = &mut buf[(x, y)];
            cell.set_symbol(" ");
            cell.set_style(Style::default().bg(shade).fg(theme.dim));
        }
    };
    for y in (area.y + 1)..=bottom.min(bh.saturating_sub(1)) {
        paint(right, y);
    }
    for x in (area.x + 1)..=right.min(bw.saturating_sub(1)) {
        paint(x, bottom);
    }
}

/// Draw the focused panel (and the ones beneath it, dimmed) over `body`.
pub fn draw(frame: &mut Frame, full: Rect, body: Rect, view: &View, theme: Theme) {
    if view.panels.is_empty() {
        return;
    }
    dim_region(frame, body, theme);
    let n = view.panels.stack.len();
    for (i, p) in view.panels.stack.iter().enumerate() {
        let focused = i + 1 == n;
        let (pw, rows) = p.size(view);
        let modal = p.modal();
        let area = rect(full, body, pw, rows, modal.is_some());
        if !focused {
            // Under-panels render dimmed with no shadow.
            draw_one(frame, area, p.as_ref(), view, theme, false);
            dim_region(frame, area, theme);
            continue;
        }
        shadow(frame, area, theme);
        draw_one(frame, area, p.as_ref(), view, theme, true);
    }
}

fn draw_one(
    frame: &mut Frame,
    area: Rect,
    p: &dyn Panel,
    view: &View,
    theme: Theme,
    focused: bool,
) {
    let border: Color = match p.modal() {
        Some(ModalKind::Permission) => theme.warn,
        Some(ModalKind::Ask) => theme.accent,
        None if focused => theme.panel_border,
        None => theme.dim,
    };
    let legend = if focused && view.panels.show_keys {
        "↑↓ move · enter activate · tab/⇧tab fields · space toggle · ←→ cycle · ^s save · esc back"
            .to_string()
    } else {
        p.legend(view)
    };
    let ch = chrome::Chrome {
        title: p.title(view),
        status: p.status(view),
        legend,
        border,
        heavy: p.modal().is_some(),
    };
    let inner = chrome::draw_frame(frame, area, &ch, theme);
    let body = p.render(view, inner.width, inner.height, theme);
    frame.render_widget(Paragraph::new(body.lines).style(theme.panel()), inner);
    if let Some((first, total)) = body.scroll {
        chrome::scrollbar(frame, area, first, total, inner.height as usize, theme);
    }
}

/// Route an editing key into the composer while a panel owns it (`R-COMP-17`).
/// Returns true when the key was an edit.
pub fn edit_field(c: &mut crate::composer::Composer, key: KeyEvent) -> bool {
    use crossterm::event::KeyModifiers;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char(ch) if !ctrl && !ch.is_control() => {
            c.insert_char(ch);
            true
        }
        KeyCode::Char('w') if ctrl => {
            c.kill_word_back();
            true
        }
        KeyCode::Char('u') if ctrl => {
            c.kill_to_line_start();
            true
        }
        KeyCode::Char('k') if ctrl => {
            c.kill_to_line_end();
            true
        }
        KeyCode::Char('a') if ctrl => {
            c.home();
            true
        }
        KeyCode::Char('e') if ctrl => {
            c.end();
            true
        }
        KeyCode::Backspace => {
            c.backspace();
            true
        }
        KeyCode::Delete => {
            c.delete();
            true
        }
        KeyCode::Left => {
            if ctrl {
                c.word_left();
            } else {
                c.left();
            }
            true
        }
        KeyCode::Right => {
            if ctrl {
                c.word_right();
            } else {
                c.right();
            }
            true
        }
        KeyCode::Home => {
            c.home();
            true
        }
        KeyCode::End => {
            c.end();
            true
        }
        _ => false,
    }
}

/// Wrap a selection index after `delta` over `n` rows (wrapping at the ends).
pub fn step(selected: usize, delta: i32, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    (selected as i32 + delta).rem_euclid(n as i32) as usize
}

/// First visible row so `selected` stays on screen.
pub fn window(selected: usize, total: usize, height: usize) -> usize {
    if total <= height || height == 0 {
        return 0;
    }
    let half = height / 2;
    selected.saturating_sub(half).min(total - height)
}
