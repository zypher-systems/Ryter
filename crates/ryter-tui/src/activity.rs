//! Activity strip: what the model is doing right now (`R-ACT-*`).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::chat::{fmt_elapsed, humanize, wrap};
use crate::theme::Theme;
use crate::view::View;

/// Braille spinner frames at 80 ms (`R-ACT-03`).
pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Per-turn reasoning cap (`R-ACT-12`).
pub const REASONING_CAP: usize = 64 * 1024;
/// Turns of reasoning retained.
pub const REASONING_TURNS: usize = 20;
/// With nothing received for this long, a busy turn is waiting for the
/// model: one that thinks on the server and streams nothing, or a slow
/// provider.
pub const WAIT_AFTER_MS: u64 = 3000;
/// How much of the latest text the ticker shows.
const TICKER_TAIL: usize = 48;

/// Startup / toggle state (`R-ACT-14`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// One-line ticker.
    Collapsed,
    /// Scrollable pane.
    Expanded,
    /// Strip hidden; `Reasoning` events discarded.
    Off,
}

impl Mode {
    /// From `[ui] reasoning`.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "expanded" => Mode::Expanded,
            "off" | "none" | "false" => Mode::Off,
            _ => Mode::Collapsed,
        }
    }
}

/// Phase verb (`R-ACT-04`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verb {
    /// No turn yet.
    Idle,
    /// Reasoning stream.
    Thinking,
    /// Token stream.
    Writing,
    /// Tool call.
    Tool(String),
    /// Permission / ask outstanding: waiting for the user.
    Waiting,
    /// Nothing has arrived for [`WAIT_AFTER_MS`]: waiting for the model.
    WaitingModel,
    /// Cancel requested.
    Cancelling,
    /// Turn finished normally.
    Done,
    /// Turn cancelled.
    Stopped,
    /// Turn errored.
    Failed,
}

impl Verb {
    /// The strip's word for the phase.
    pub fn label(&self) -> String {
        match self {
            Verb::Idle => "idle".into(),
            Verb::Thinking => "thinking".into(),
            Verb::Writing => "writing".into(),
            Verb::Tool(t) => t.clone(),
            Verb::Waiting => "waiting for you".into(),
            Verb::WaitingModel => "waiting for the model".into(),
            Verb::Cancelling => "cancelling".into(),
            Verb::Done => "done".into(),
            Verb::Stopped => "stopped".into(),
            Verb::Failed => "failed".into(),
        }
    }

    /// Terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Verb::Done | Verb::Stopped | Verb::Failed | Verb::Idle)
    }
}

/// Strip state.
#[derive(Debug, Clone, PartialEq)]
pub struct Activity {
    /// Collapsed / expanded / off.
    pub mode: Mode,
    /// Most recent verb.
    pub verb: Verb,
    /// Current activity detail (tool summary).
    pub current: String,
    /// `now_ms` when the turn started.
    pub started_ms: Option<u64>,
    /// Elapsed, frozen at turn end.
    pub elapsed_ms: u64,
    /// Output tokens this turn.
    pub tokens: u64,
    /// True until a `Spend` event confirms the count (`R-ACT-07`).
    pub tokens_estimated: bool,
    /// Spinner frame.
    pub frame: usize,
    /// Tool calls this turn.
    pub tools: u32,
    /// USD this turn when known.
    pub cost: Option<f64>,
    /// Some turn was unpriced.
    pub cost_unknown: bool,
    /// Reasoning pane scroll; `None` follows the tail (`R-ACT-11`).
    pub scroll: Option<usize>,
    /// Turn whose reasoning the pane shows.
    pub turn: u64,
    /// Whether any turn has ever run (height 0 otherwise, `R-ACT-01`).
    pub has_history: bool,
    /// `now_ms` of the last delta of any kind: a token, a thought, a tool
    /// call or its result. Nothing for [`WAIT_AFTER_MS`] is waiting.
    pub last_delta_ms: Option<u64>,
    /// The tail of the latest text, single-spaced, for the ticker.
    tail: String,
    /// What the open question asks, while one is open: `allow?`, `plan?`.
    pub ask: Option<String>,
}

impl Default for Activity {
    fn default() -> Self {
        Self::new(Mode::Collapsed)
    }
}

impl Activity {
    /// Fresh strip.
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            verb: Verb::Idle,
            current: String::new(),
            started_ms: None,
            elapsed_ms: 0,
            tokens: 0,
            tokens_estimated: true,
            frame: 0,
            tools: 0,
            cost: None,
            cost_unknown: false,
            scroll: None,
            turn: 0,
            has_history: false,
            last_delta_ms: None,
            tail: String::new(),
            ask: None,
        }
    }

    /// Turn began.
    pub fn start(&mut self, turn: u64, now_ms: u64) {
        self.verb = Verb::Thinking;
        self.current.clear();
        self.tail.clear();
        self.last_delta_ms = Some(now_ms);
        self.started_ms = Some(now_ms);
        self.elapsed_ms = 0;
        self.tokens = 0;
        self.tokens_estimated = true;
        self.tools = 0;
        self.cost = None;
        self.cost_unknown = false;
        self.scroll = None;
        self.turn = turn;
        self.has_history = true;
    }

    /// Turn ended (`R-ACT-08`).
    pub fn finish(&mut self, verb: Verb, tools: Option<u32>, duration_ms: Option<u64>) {
        self.verb = verb;
        if let Some(t) = tools {
            self.tools = t;
        }
        if let Some(d) = duration_ms {
            self.elapsed_ms = d;
        }
        self.started_ms = None;
        self.current.clear();
    }

    /// Advance the spinner and elapsed clock. A turn that has received
    /// nothing for [`WAIT_AFTER_MS`] while it was thinking or writing is
    /// waiting for the model; a tool that is running is not, however long
    /// it takes.
    pub fn tick(&mut self, now_ms: u64) {
        self.frame = (now_ms / 80) as usize % SPINNER.len();
        if let Some(s) = self.started_ms {
            self.elapsed_ms = now_ms.saturating_sub(s);
            let since = now_ms.saturating_sub(self.last_delta_ms.unwrap_or(s));
            if since >= WAIT_AFTER_MS
                && matches!(
                    self.verb,
                    Verb::Thinking | Verb::Writing | Verb::WaitingModel
                )
            {
                self.verb = Verb::WaitingModel;
            }
        }
    }

    /// Something arrived from the model: a token, a thought, a tool call
    /// or its result. The caller sets the verb; this ends any waiting.
    pub fn note_delta(&mut self, now_ms: u64) {
        self.last_delta_ms = Some(now_ms);
        self.ask = None;
    }

    /// A question for the user is open: what it asks, in a word or two
    /// (`allow?`, `plan?`, `question`), for the status row.
    pub fn note_ask(&mut self, ask: &str) {
        self.verb = Verb::Waiting;
        self.ask = Some(ask.to_string());
    }

    /// The latest text, for the ticker: its tail, single-spaced.
    pub fn note_text(&mut self, text: &str) {
        for w in text.split_whitespace() {
            if !self.tail.is_empty() {
                self.tail.push(' ');
            }
            self.tail.push_str(w);
        }
        let n = self.tail.chars().count();
        if n > TICKER_TAIL {
            let cut: String = self.tail.chars().skip(n - TICKER_TAIL).collect();
            self.tail = cut;
            self.current = format!("…{}", self.tail);
        } else {
            self.current.clone_from(&self.tail);
        }
    }

    /// A tool's label took the ticker: the next text starts it afresh.
    pub fn note_tool(&mut self, label: &str) {
        self.tail.clear();
        self.current = wrap::truncate(label, 48);
    }

    /// The status row's words for a busy turn: what it is doing, for how
    /// long, and how much it has produced. Narrow, the tokens go first,
    /// then the time.
    pub fn status(&self, width: usize) -> String {
        let label = match (&self.verb, &self.ask) {
            (Verb::Tool(t), _) => format!("running {t}"),
            (Verb::Waiting, Some(ask)) => format!("waiting for you · {ask}"),
            (v, _) => v.label(),
        };
        let mut parts = vec![label];
        if width >= 32 {
            parts.push(fmt_elapsed(self.elapsed_ms / 1000));
        }
        if width >= 48 && self.tokens > 0 {
            parts.push(format!("{} tokens", humanize(self.tokens)));
        }
        parts.join(" · ")
    }

    /// Whether a turn is in flight from the strip's point of view.
    pub fn busy(&self) -> bool {
        self.started_ms.is_some()
    }

    /// Toggle collapsed ↔ expanded (`Ctrl+R`). `Off` becomes collapsed.
    pub fn toggle(&mut self) {
        self.mode = match self.mode {
            Mode::Collapsed | Mode::Off => Mode::Expanded,
            Mode::Expanded => Mode::Collapsed,
        };
    }

    /// Terminal summary text.
    fn summary(&self, spend_unknown: bool) -> String {
        let glyph = match self.verb {
            Verb::Done => "✓",
            Verb::Stopped => "⊘",
            Verb::Failed => "✕",
            _ => "·",
        };
        let mut parts = vec![format!("{glyph} {}", self.verb.label())];
        parts.push(format!(
            "{} tool{}",
            self.tools,
            if self.tools == 1 { "" } else { "s" }
        ));
        parts.push(fmt_elapsed(self.elapsed_ms / 1000));
        if let Some(c) = self.cost {
            parts.push(ryter_core::format_usd(Some(c)));
        } else if spend_unknown || self.cost_unknown {
            parts.push(ryter_core::format_usd(None));
        }
        parts.join(" · ")
    }
}

/// Strip height for the frame layout (`R-ACT-01`, `R-ACT-09`).
pub fn height(view: &View, body_h: u16) -> u16 {
    let a = &view.activity;
    if a.mode == Mode::Off || !a.has_history {
        return 0;
    }
    // On the ledger a finished turn's closing line says what the strip
    // would: it shows while a turn runs, or when its reasoning is opened.
    if !view.ui.classic() && !a.busy() && a.mode == Mode::Collapsed {
        return 0;
    }
    match a.mode {
        Mode::Collapsed => 1,
        Mode::Expanded => {
            let text = view
                .reasoning
                .get(&a.turn)
                .map(String::as_str)
                .unwrap_or("");
            let rows = if text.trim().is_empty() {
                if a.busy() { 2 } else { 1 }
            } else {
                wrap::wrap_plain(text, 76).len()
            };
            let cap = (body_h / 3).clamp(3, 12);
            u16::try_from(rows + 2).unwrap_or(cap).clamp(3, cap)
        }
        Mode::Off => 0,
    }
}

/// Draw the strip in `area`.
pub fn draw(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    if area.height == 0 {
        return;
    }
    frame.render_widget(Block::default().style(theme.body()), area);
    let a = &view.activity;
    let width = area.width as usize;
    let spinner_style = if a.busy() {
        Style::default().fg(theme.accent).bg(theme.bg)
    } else {
        theme.muted()
    };
    let spinner = if a.busy() {
        SPINNER[a.frame % SPINNER.len()]
    } else {
        "·"
    };
    if a.mode == Mode::Collapsed {
        let left = if a.busy() {
            let mut parts = vec![a.verb.label()];
            if !a.current.is_empty() {
                parts.push(wrap::truncate(&a.current, 40));
            }
            parts.push(fmt_elapsed(a.elapsed_ms / 1000));
            let tok = if a.tokens_estimated {
                format!("~{} tok", humanize(a.tokens))
            } else {
                format!("{} tok", humanize(a.tokens))
            };
            parts.push(tok);
            parts.join("  ·  ")
        } else if a.verb.is_terminal() && a.verb != Verb::Idle {
            a.summary(view.spend_unknown)
        } else {
            a.verb.label()
        };
        let right = "^r  reasoning  ▾";
        let rw = wrap::width(right);
        let avail = width.saturating_sub(3 + rw + 2);
        let left = wrap::truncate(&left, avail);
        let pad = width.saturating_sub(3 + wrap::width(&left) + rw + 1);
        let line = Line::from(vec![
            Span::styled(" ", theme.body()),
            Span::styled(spinner, spinner_style),
            Span::styled(" ", theme.body()),
            Span::styled(left, theme.muted()),
            Span::styled(" ".repeat(pad), theme.body()),
            Span::styled(right, theme.muted()),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }
    // Expanded (`R-ACT-09..11`).
    let mut lines: Vec<Line> = Vec::new();
    let head_right = format!("{}   ^r close ▴", fmt_elapsed(a.elapsed_ms / 1000));
    let verb = if a.busy() {
        a.verb.label()
    } else {
        a.summary(view.spend_unknown)
    };
    let pad = width.saturating_sub(3 + wrap::width(&verb) + wrap::width(&head_right) + 1);
    lines.push(Line::from(vec![
        Span::styled(" ", theme.body()),
        Span::styled(spinner, spinner_style),
        Span::styled(" ", theme.body()),
        Span::styled(
            verb,
            Style::default()
                .fg(theme.fg)
                .bg(theme.bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ".repeat(pad), theme.body()),
        Span::styled(head_right, theme.muted()),
    ]));
    let body_h = area.height.saturating_sub(1) as usize;
    let text = view
        .reasoning
        .get(&a.turn)
        .map(String::as_str)
        .unwrap_or("");
    let rows: Vec<String> = if text.trim().is_empty() && a.busy() {
        vec![
            format!(
                "waiting for the model · nothing streamed yet · {}",
                fmt_elapsed(a.elapsed_ms / 1000)
            ),
            "some models think on the server and send nothing until they answer".into(),
        ]
    } else if text.trim().is_empty() {
        vec!["no reasoning stream from this model".into()]
    } else {
        wrap::wrap_plain(text, width.saturating_sub(2).max(8))
    };
    let start = match a.scroll {
        Some(s) => s.min(rows.len().saturating_sub(body_h)),
        None => rows.len().saturating_sub(body_h),
    };
    let dim_italic = Style::default()
        .fg(theme.dim)
        .bg(theme.bg)
        .add_modifier(Modifier::ITALIC);
    for r in rows.iter().skip(start).take(body_h) {
        lines.push(Line::from(vec![
            Span::styled(" ", theme.body()),
            Span::styled(r.clone(), dim_italic),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// Append reasoning for `turn`, honoring the caps (`R-ACT-12`).
pub fn push_reasoning(view: &mut View, turn: u64, text: &str) {
    let buf = view.reasoning.entry(turn).or_default();
    if buf.len() < REASONING_CAP {
        let room = REASONING_CAP - buf.len();
        let take: String = text.chars().take(room).collect();
        buf.push_str(&take);
    }
    while view.reasoning.len() > REASONING_TURNS {
        let Some(&oldest) = view.reasoning.keys().next() else {
            break;
        };
        view.reasoning.remove(&oldest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nothing received for three seconds while thinking is waiting for
    /// the model; a delta ends it; a tool that runs long is not waiting.
    #[test]
    fn waiting_for_the_model_after_three_quiet_seconds() {
        let mut a = Activity::new(Mode::Collapsed);
        a.start(1, 1000);
        a.tick(3_900);
        assert_eq!(a.verb, Verb::Thinking);
        a.tick(4_000);
        assert_eq!(a.verb, Verb::WaitingModel);
        assert_eq!(a.status(80), "waiting for the model · 0:03");
        a.note_delta(4_100);
        a.verb = Verb::Writing;
        a.tokens = 1200;
        a.tick(6_000);
        assert_eq!(a.verb, Verb::Writing);
        assert_eq!(a.status(80), "writing · 0:05 · 1.2k tokens");
        assert_eq!(a.status(40), "writing · 0:05");
        assert_eq!(a.status(30), "writing");
        a.verb = Verb::Tool("cargo test".into());
        a.tick(60_000);
        assert_eq!(
            a.verb,
            Verb::Tool("cargo test".into()),
            "a long tool is not waiting"
        );
        assert_eq!(a.status(80), "running cargo test · 0:59 · 1.2k tokens");
        a.verb = Verb::Waiting;
        assert_eq!(a.status(80), "waiting for you · 0:59 · 1.2k tokens");
    }

    /// The ticker shows the tail of the latest text, single-spaced, and a
    /// tool's label takes it over until the next text.
    #[test]
    fn the_ticker_follows_the_latest_text() {
        let mut a = Activity::new(Mode::Collapsed);
        a.start(1, 0);
        a.note_text("The user wants\nthe loop   wired.");
        assert_eq!(a.current, "The user wants the loop wired.");
        a.note_text(" I should read run.rs first, then the tests, then write.");
        assert!(a.current.starts_with('…'), "{}", a.current);
        assert!(
            a.current.ends_with("then the tests, then write."),
            "{}",
            a.current
        );
        assert!(a.current.chars().count() <= 49);
        a.note_tool("read src/run.rs");
        assert_eq!(a.current, "read src/run.rs");
        a.note_text("Now the tests.");
        assert_eq!(a.current, "Now the tests.");
    }

    #[test]
    fn summary_and_modes() {
        let mut a = Activity::new(Mode::parse("expanded"));
        assert_eq!(a.mode, Mode::Expanded);
        a.start(1, 1000);
        a.tick(35_000);
        assert_eq!(a.elapsed_ms, 34_000);
        assert!(a.busy());
        a.cost = Some(0.012);
        a.finish(Verb::Done, Some(4), Some(41_000));
        assert!(!a.busy());
        assert_eq!(a.summary(false), "✓ done · 4 tools · 0:41 · $0.01");
        a.cost = None;
        assert_eq!(a.summary(true), "✓ done · 4 tools · 0:41 · $?.??");
        a.toggle();
        assert_eq!(a.mode, Mode::Collapsed);
        assert_eq!(Mode::parse("off"), Mode::Off);
    }
}
