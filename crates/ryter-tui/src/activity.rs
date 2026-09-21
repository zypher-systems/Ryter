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
    /// Permission / ask outstanding.
    Waiting,
    /// Builder result being merged.
    Merging,
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
    fn label(&self) -> String {
        match self {
            Verb::Idle => "idle".into(),
            Verb::Thinking => "thinking".into(),
            Verb::Writing => "writing".into(),
            Verb::Tool(t) => t.clone(),
            Verb::Waiting => "waiting".into(),
            Verb::Merging => "merging".into(),
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
        }
    }

    /// Turn began.
    pub fn start(&mut self, turn: u64, now_ms: u64) {
        self.verb = Verb::Thinking;
        self.current.clear();
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

    /// Advance the spinner and elapsed clock.
    pub fn tick(&mut self, now_ms: u64) {
        self.frame = (now_ms / 80) as usize % SPINNER.len();
        if let Some(s) = self.started_ms {
            self.elapsed_ms = now_ms.saturating_sub(s);
        }
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
    match a.mode {
        Mode::Collapsed => 1,
        Mode::Expanded => {
            let text = view
                .reasoning
                .get(&a.turn)
                .map(String::as_str)
                .unwrap_or("");
            let rows = if text.trim().is_empty() {
                1
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
    let rows: Vec<String> = if text.trim().is_empty() {
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
