//! Chat transcript: message model and per-message rendering (`R-CHAT-*`).
//!
//! `Message` replaces the old `LogLine`. Every message opens with a speaker
//! header row; bodies are left-aligned for every kind (`R-CHAT-09`).

pub mod cache;
pub mod highlight;
pub mod layout;
pub mod markdown;
pub mod toolview;
pub mod wrap;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ryter_core::format_usd;

use crate::theme::Theme;

/// Last path segment of a model id (`anthropic/claude-sonnet-4.6` → `claude-sonnet-4.6`).
pub fn short_model(id: &str) -> &str {
    id.rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(id)
}

/// Wall-clock instant with a fixed UTC offset, enough for `HH:MM` headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OffsetTimestamp {
    /// Seconds since the Unix epoch.
    pub epoch_secs: u64,
    /// Local offset from UTC in seconds.
    pub offset_secs: i32,
}

impl OffsetTimestamp {
    /// Now, at the given offset.
    pub fn now(offset_secs: i32) -> Self {
        let epoch_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            epoch_secs,
            offset_secs,
        }
    }

    /// From a unix-millisecond string (session metadata).
    pub fn from_millis(ms: u64, offset_secs: i32) -> Self {
        Self {
            epoch_secs: ms / 1000,
            offset_secs,
        }
    }

    /// `HH:MM` in local time.
    pub fn hhmm(self) -> String {
        let local = i64::try_from(self.epoch_secs).unwrap_or(0) + i64::from(self.offset_secs);
        let day = local.rem_euclid(86_400);
        format!("{:02}:{:02}", day / 3600, (day % 3600) / 60)
    }
}

/// Parse `+0530` / `-0400` (`date +%z`) into seconds.
pub fn parse_tz_offset(s: &str) -> Option<i32> {
    let s = s.trim();
    let (sign, digits) = match s.chars().next()? {
        '+' => (1, &s[1..]),
        '-' => (-1, &s[1..]),
        _ => (1, s),
    };
    if digits.len() != 4 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let h: i32 = digits[0..2].parse().ok()?;
    let m: i32 = digits[2..4].parse().ok()?;
    Some(sign * (h * 3600 + m * 60))
}

/// Outcome of a tool call for its row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    /// In flight.
    Running,
    /// Returned normally.
    Ok,
    /// Returned an error payload.
    Error,
}

/// Severity of a system message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemLevel {
    /// Neutral.
    Info,
    /// Attention.
    Warn,
    /// Failure.
    Error,
    /// A dim horizontal rule with the body centred in it (`R-EVT-04`).
    Rule,
}

/// Who or what produced a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageKind {
    /// The person at the keyboard.
    User,
    /// Orchestrator model output.
    Assistant {
        /// Model id.
        model: String,
    },
    /// Specialist output (display only; not orchestrator context).
    Specialist {
        /// `planner` / `architect` / `builder` / `auditor`.
        role: String,
        /// Model id if known.
        model: String,
    },
    /// Collapsed tool row.
    Tool {
        /// Tool name.
        name: String,
        /// Outcome.
        status: ToolStatus,
    },
    /// Merge notice after a builder finished.
    Merge,
    /// TUI / slash output.
    System {
        /// Severity.
        level: SystemLevel,
    },
}

/// Metadata for the header's right side.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MessageMeta {
    /// USD for this message when known. Never print `$0.00` for unknown.
    pub cost: Option<f64>,
    /// Elapsed for tools / turns.
    pub duration_ms: Option<u64>,
    /// What a tool step came to: `new · 48 lines`, `✓ 12 passed`, `✗ exit 1`.
    pub detail: Option<String>,
    /// Secondary label: specialist task, tool summary.
    pub label: Option<String>,
    /// Provider tool-call id (to match `ToolResult`).
    pub tool_id: Option<String>,
}

/// One transcript entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    /// Monotonic, stable across re-render.
    pub id: u64,
    /// Groups a user message with everything it caused.
    pub turn: u64,
    /// Speaker.
    pub kind: MessageKind,
    /// Raw markdown / text, mutated during streaming.
    pub body: String,
    /// Wall clock for the header.
    pub at: OffsetTimestamp,
    /// Cost, duration, labels.
    pub meta: MessageMeta,
    /// Bumped on every mutation; render-cache key.
    pub rev: u64,
}

impl Message {
    /// Append streamed text (`R-CHAT-03`).
    pub fn append(&mut self, text: &str) {
        self.body.push_str(text);
        self.rev += 1;
    }

    /// Replace the body.
    pub fn set_body(&mut self, text: String) {
        self.body = text;
        self.rev += 1;
    }

    /// Mutate metadata and invalidate.
    pub fn touch(&mut self) {
        self.rev += 1;
    }

    /// True when `other` continues this speaker in the same turn (`R-CHAT-08`).
    pub fn same_speaker(&self, other: &Self) -> bool {
        if self.turn != other.turn {
            return false;
        }
        match (&self.kind, &other.kind) {
            (MessageKind::User, MessageKind::User) => true,
            (MessageKind::Assistant { model: a }, MessageKind::Assistant { model: b }) => a == b,
            (MessageKind::Specialist { role: a, .. }, MessageKind::Specialist { role: b, .. }) => {
                a == b
            }
            _ => false,
        }
    }

    /// Display name for the header (`R-CHAT-06`).
    pub fn speaker(&self, username: &str) -> String {
        match &self.kind {
            MessageKind::User => wrap::truncate(username, 20),
            MessageKind::Assistant { model } => short_model(model).to_string(),
            MessageKind::Specialist { role, .. } => role.clone(),
            MessageKind::Tool { name, .. } => name.clone(),
            MessageKind::Merge => "merge".into(),
            MessageKind::System { level } => match level {
                SystemLevel::Info => "system".into(),
                SystemLevel::Warn => "warning".into(),
                SystemLevel::Error => "error".into(),
                SystemLevel::Rule => String::new(),
            },
        }
    }

    /// Leading glyph. Non-color companion for every colored state (`R-THEME-09`).
    pub fn glyph(&self) -> &'static str {
        match &self.kind {
            MessageKind::User => "",
            MessageKind::Assistant { .. } => "",
            MessageKind::Specialist { .. } => "⇢ ",
            MessageKind::Tool { status, .. } => match status {
                ToolStatus::Running => "◌ ",
                ToolStatus::Ok => "· ",
                ToolStatus::Error => "! ",
            },
            MessageKind::Merge => "⇄ ",
            MessageKind::System { level } => match level {
                SystemLevel::Info => "· ",
                SystemLevel::Warn => "! ",
                SystemLevel::Error => "✕ ",
                SystemLevel::Rule => "",
            },
        }
    }

    /// Accent color for the speaker name.
    pub fn accent(&self, theme: Theme) -> Color {
        match &self.kind {
            MessageKind::User => theme.user,
            MessageKind::Assistant { .. } => theme.assistant,
            MessageKind::Specialist { role, .. } => theme.role(role),
            MessageKind::Tool { status, .. } => match status {
                ToolStatus::Error => theme.error,
                _ => theme.tool,
            },
            MessageKind::Merge => theme.build,
            MessageKind::System { level } => match level {
                SystemLevel::Info | SystemLevel::Rule => theme.dim,
                SystemLevel::Warn => theme.warn,
                SystemLevel::Error => theme.error,
            },
        }
    }

    /// One-line collapsed form for the sticky header (`R-SCROLL-04`).
    pub fn collapsed(&self, width: usize) -> String {
        let first = self
            .body
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        wrap::truncate(first, width)
    }

    /// Right-side metadata (`R-CHAT-07`).
    pub fn meta_text(&self, timestamps: bool) -> String {
        let mut parts: Vec<String> = Vec::new();
        match &self.kind {
            MessageKind::Tool { .. } => {
                if let Some(d) = self.meta.detail.as_ref().filter(|d| !d.is_empty()) {
                    parts.push(d.clone());
                }
                if let Some(ms) = self.meta.duration_ms {
                    parts.push(fmt_duration(ms));
                }
            }
            MessageKind::User | MessageKind::System { .. } => {
                if timestamps {
                    parts.push(self.at.hhmm());
                }
            }
            MessageKind::Assistant { .. } | MessageKind::Specialist { .. } | MessageKind::Merge => {
                if timestamps {
                    parts.push(self.at.hhmm());
                }
                if let Some(c) = self.meta.cost {
                    parts.push(format_usd(Some(c)));
                }
            }
        }
        parts.join("  ")
    }
}

/// `0.2s` / `12s` / `1:04` for tool durations.
pub fn fmt_duration(ms: u64) -> String {
    if ms < 10_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else if ms < 60_000 {
        format!("{}s", ms / 1000)
    } else {
        fmt_elapsed(ms / 1000)
    }
}

/// `M:SS` from seconds (`R-ACT-06`).
pub fn fmt_elapsed(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// Humanized token count: `842`, `1.2k`, `192k`, `1.5M` (`R-ACT-07`).
pub fn humanize(n: u64) -> String {
    if n < 1000 {
        n.to_string()
    } else if n < 10_000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else if n < 1_000_000 {
        format!("{}k", n / 1000)
    } else {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    }
}

/// Rendering knobs for one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderOpts {
    /// Pane width in columns.
    pub width: usize,
    /// Show `HH:MM`.
    pub timestamps: bool,
    /// Line numbers in code blocks.
    pub line_numbers: bool,
    /// Resolved username.
    pub username: String,
    /// File extension guessed from the previous tool row (`R-SYN-04`).
    pub lang_hint: Option<String>,
    /// Omit the header (continuation of the same speaker).
    pub continuation: bool,
}

/// Render one message to rows. The first row is the speaker header unless
/// `opts.continuation`.
pub fn render_message(msg: &Message, opts: &RenderOpts, theme: Theme) -> Vec<Line<'static>> {
    let width = opts.width.max(12);
    let mut out: Vec<Line<'static>> = Vec::new();
    if let MessageKind::System {
        level: SystemLevel::Rule,
    } = msg.kind
    {
        // `── transcript compacted · 180k → 42k tokens ──` (`R-EVT-04`).
        let label = wrap::truncate(msg.body.trim(), width.saturating_sub(6));
        let fill = width.saturating_sub(wrap::width(&label) + 2);
        let left = fill / 2;
        let right = fill - left;
        let text = format!("{} {label} {}", "─".repeat(left), "─".repeat(right));
        out.push(Line::from(Span::styled(text, theme.muted())));
        return out;
    }
    if !opts.continuation {
        out.push(header_row(msg, opts, theme, width));
    }
    let is_tool = matches!(msg.kind, MessageKind::Tool { .. });
    if is_tool && msg.body.trim().is_empty() {
        return out;
    }
    // Bodies indent one column; user bodies carry a `▎` rule (`R-CHAT-10`).
    let (gutter, gutter_style) = match msg.kind {
        MessageKind::User => ("▎", Style::default().fg(theme.user).bg(theme.bg)),
        _ => (" ", theme.body()),
    };
    let inner = width.saturating_sub(2).max(8);
    let body_rows: Vec<Line<'static>> = match &msg.kind {
        MessageKind::User => markdown::render_inline_only(&msg.body, inner, theme),
        MessageKind::Assistant { .. } | MessageKind::Specialist { .. } | MessageKind::Merge => {
            let md = markdown::MdOptions {
                width: inner,
                line_numbers: opts.line_numbers,
                lang_hint: opts.lang_hint.clone(),
            };
            markdown::render(&msg.body, &md, theme)
        }
        // Tool bodies: `- ` removed, `+ ` added, `! ` failure output, else plain.
        MessageKind::Tool { .. } => msg
            .body
            .lines()
            .flat_map(|line| {
                let (text, style) = if let Some(rest) = line.strip_prefix("! ") {
                    (rest.to_string(), theme.on_bg(theme.error))
                } else if line.starts_with("- ") {
                    (line.to_string(), theme.on_bg(theme.error))
                } else if line.starts_with("+ ") {
                    (line.to_string(), theme.on_bg(theme.success))
                } else {
                    (line.to_string(), theme.muted())
                };
                wrap::wrap_plain(&text, inner)
                    .into_iter()
                    .map(move |l| Line::from(Span::styled(l, style)))
                    .collect::<Vec<_>>()
            })
            .collect(),
        MessageKind::System { .. } => {
            let style = match &msg.kind {
                MessageKind::System {
                    level: SystemLevel::Error,
                } => theme.on_bg(theme.error),
                MessageKind::System {
                    level: SystemLevel::Warn,
                } => theme.on_bg(theme.warn),
                _ => theme.muted(),
            };
            wrap::wrap_plain(&msg.body, inner)
                .into_iter()
                .map(|l| Line::from(Span::styled(l, style)))
                .collect()
        }
    };
    for row in body_rows {
        let mut spans = vec![
            Span::styled(gutter, gutter_style),
            Span::styled(" ", theme.body()),
        ];
        spans.extend(row.spans);
        out.push(Line::from(spans));
    }
    out
}

fn header_row(msg: &Message, opts: &RenderOpts, theme: Theme, width: usize) -> Line<'static> {
    let accent = msg.accent(theme);
    let name_style = Style::default()
        .fg(accent)
        .bg(theme.bg)
        .add_modifier(Modifier::BOLD);
    let glyph = msg.glyph();
    let name = msg.speaker(&opts.username);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    if !glyph.is_empty() {
        spans.push(Span::styled(
            glyph,
            Style::default().fg(accent).bg(theme.bg),
        ));
        used += wrap::width(glyph);
    }
    spans.push(Span::styled(name.clone(), name_style));
    used += wrap::width(&name);
    let meta = msg.meta_text(opts.timestamps);
    let meta_w = wrap::width(&meta);
    let mut label = msg.meta.label.clone().unwrap_or_default();
    if !label.is_empty() {
        let room = width.saturating_sub(used + meta_w + 4);
        label = wrap::truncate(&label, room);
        if !label.is_empty() {
            spans.push(Span::styled(format!("  {label}"), theme.muted()));
            used += 2 + wrap::width(&label);
        }
    }
    if meta_w > 0 && used + meta_w < width {
        let pad = width - used - meta_w;
        spans.push(Span::styled(" ".repeat(pad), theme.body()));
        spans.push(Span::styled(meta, theme.muted()));
    }
    Line::from(spans)
}

/// Best-effort file extension from a tool summary like `read src/main.rs`.
pub fn lang_hint_from_tool(msg: &Message) -> Option<String> {
    if !matches!(msg.kind, MessageKind::Tool { .. }) {
        return None;
    }
    let label = msg.meta.label.as_deref()?;
    label
        .split_whitespace()
        .rev()
        .find_map(highlight::lang_from_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn msg(kind: MessageKind, body: &str) -> Message {
        Message {
            id: 1,
            turn: 1,
            kind,
            body: body.into(),
            at: OffsetTimestamp {
                epoch_secs: 70_920,
                offset_secs: 0,
            },
            meta: MessageMeta::default(),
            rev: 0,
        }
    }

    fn opts(width: usize) -> RenderOpts {
        RenderOpts {
            width,
            timestamps: true,
            line_numbers: true,
            username: "Dusty".into(),
            lang_hint: None,
            continuation: false,
        }
    }

    #[test]
    fn timestamps_and_offsets() {
        let t = OffsetTimestamp {
            epoch_secs: 0,
            offset_secs: -4 * 3600,
        };
        assert_eq!(t.hhmm(), "20:00");
        assert_eq!(parse_tz_offset("+0530"), Some(19_800));
        assert_eq!(parse_tz_offset("-0400"), Some(-14_400));
        assert_eq!(parse_tz_offset("garbage"), None);
    }

    #[test]
    fn user_header_is_left_aligned_with_rule() {
        let m = msg(MessageKind::User, "add a --json flag");
        let rows = render_message(&m, &opts(60), Theme::truecolor_dark());
        assert!(
            rows[0].spans[0].content.starts_with("Dusty"),
            "{:?}",
            text(&rows[0])
        );
        assert!(text(&rows[0]).ends_with("19:42"));
        assert!(text(&rows[1]).starts_with("▎ add a --json flag"));
        // A user typing `# TODO` does not get a heading (R-MD-12).
        let m = msg(MessageKind::User, "# TODO");
        let rows = render_message(&m, &opts(60), Theme::truecolor_dark());
        assert!(text(&rows[1]).contains("# TODO"));
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn assistant_header_omits_unknown_cost() {
        let mut m = msg(
            MessageKind::Assistant {
                model: "x/grok-4.6".into(),
            },
            "hi",
        );
        let rows = render_message(&m, &opts(60), Theme::truecolor_dark());
        let h = text(&rows[0]);
        assert!(h.starts_with("grok-4.6"));
        assert!(!h.contains('$'), "{h}");
        m.meta.cost = Some(0.012);
        let rows = render_message(&m, &opts(60), Theme::truecolor_dark());
        assert!(
            text(&rows[0]).ends_with("19:42  $0.01"),
            "{}",
            text(&rows[0])
        );
    }

    #[test]
    fn tool_row_is_a_single_header() {
        let mut m = msg(
            MessageKind::Tool {
                name: "read".into(),
                status: ToolStatus::Ok,
            },
            "",
        );
        m.meta.label = Some("read Cargo.toml".into());
        m.meta.duration_ms = Some(200);
        let rows = render_message(&m, &opts(60), Theme::truecolor_dark());
        assert_eq!(rows.len(), 1);
        let h = text(&rows[0]);
        assert!(h.starts_with("· read  read Cargo.toml"), "{h}");
        assert!(h.ends_with("0.2s"), "{h}");
        assert_eq!(lang_hint_from_tool(&m).as_deref(), Some("toml"));
    }

    #[test]
    fn same_speaker_collapses_within_turn() {
        let a = msg(MessageKind::Assistant { model: "m".into() }, "a");
        let mut b = a.clone();
        b.id = 2;
        assert!(a.same_speaker(&b));
        b.turn = 2;
        assert!(!a.same_speaker(&b));
        let rows = render_message(
            &a,
            &RenderOpts {
                continuation: true,
                ..opts(40)
            },
            Theme::truecolor_dark(),
        );
        assert_eq!(text(&rows[0]).trim(), "a");
    }

    #[test]
    fn humanize_and_durations() {
        assert_eq!(humanize(842), "842");
        assert_eq!(humanize(1234), "1.2k");
        assert_eq!(humanize(192_000), "192k");
        assert_eq!(humanize(1_500_000), "1.5M");
        assert_eq!(fmt_duration(200), "0.2s");
        assert_eq!(fmt_duration(12_000), "12s");
        assert_eq!(fmt_elapsed(94), "1:34");
    }
}
