//! Composer rendering (`R-COMP-01..08`, `R-COMP-16`).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_segmentation::UnicodeSegmentation;

use super::{COUNTER_AT, MAX_ROWS, Mode, group_thousands};
use crate::chat::wrap;
use crate::theme::Theme;
use crate::view::View;

/// Rows the composer needs at `width` (border included).
pub fn height(view: &View, width: u16) -> u16 {
    if !view.ui.classic() {
        // A rule above the prompt, no box.
        let col = width.min(crate::draw::LEDGER_COLUMN);
        let inner = usize::from(col.saturating_sub(3));
        let rows = view.composer.rows(inner).clamp(1, MAX_ROWS);
        return u16::try_from(rows).unwrap_or(1) + 1;
    }
    let inner = usize::from(width.saturating_sub(4));
    let rows = view.composer.rows(inner).clamp(1, MAX_ROWS);
    u16::try_from(rows).unwrap_or(1) + 2
}

/// Border color per `R-COMP-01`.
pub fn border_color(view: &View, theme: Theme) -> Color {
    if view.composer.rejected {
        return theme.error;
    }
    match &view.composer.mode {
        Mode::Secret { .. } => return theme.warn,
        Mode::Field { .. } => return theme.accent,
        Mode::Normal => {}
    }
    // The mode's color, always: it is how the user knows, at the point of
    // typing, which hat gets this message.
    theme.mode(view.mode)
}

/// Border title per `R-COMP-02`.
pub fn title(view: &View) -> String {
    match &view.composer.mode {
        Mode::Normal => view.username.clone(),
        Mode::Secret { .. } => "api key".into(),
        Mode::Field { label } => label.clone(),
    }
}

fn glyph(view: &View) -> &'static str {
    match view.composer.mode {
        Mode::Normal | Mode::Field { .. } => "›",
        Mode::Secret { .. } => "key",
    }
}

fn placeholder(view: &View) -> String {
    match &view.composer.mode {
        Mode::Normal if view.busy => "type to queue the next message, / for commands".into(),
        Mode::Normal => match view.mode {
            ryter_core::Role::SoloBuild | ryter_core::Role::Crew => {
                "what should change? · Tab: plan · / for commands".into()
            }
            ryter_core::Role::SoloPlan => {
                "what are we planning? nothing changes here · Tab: review".into()
            }
            ryter_core::Role::SoloReview => {
                "what should be reviewed? · /audit reviews what changed · Tab: test".into()
            }
            ryter_core::Role::SoloTest => match view.retest {
                Some(n) => format!("ask the tester, or: retest {n} · Tab: build"),
                None => "ask the tester · Tab: build · / for commands".into(),
            },
        },
        Mode::Secret { connection } => format!("paste the API key for {connection}"),
        Mode::Field { label } => format!("type {label}…"),
    }
}

/// The ledger's composer: a rule, then the prompt. The rule takes the
/// mode's color, so where the message goes is still in sight as you type;
/// `queued` and a long message's count sit at its right end.
/// Notes for the prompt's edge: a message waiting its turn, and the length
/// of a long one.
fn edge_notes(view: &View, theme: Theme) -> Vec<Span<'static>> {
    let mut right: Vec<Span<'static>> = Vec::new();
    if view.queued_prompt.is_some() {
        right.push(Span::styled(
            " queued ",
            Style::default()
                .fg(theme.warn)
                .bg(theme.bg)
                .add_modifier(Modifier::BOLD),
        ));
    }
    let count = view.composer.char_count();
    if count > COUNTER_AT && !matches!(view.composer.mode, Mode::Secret { .. }) {
        right.push(Span::styled(
            format!(" {} chars ", group_thousands(count)),
            Style::default().fg(theme.dim).bg(theme.bg),
        ));
    }
    right
}

/// Rows the boxed prompt needs at `width`, its box included.
pub fn boxed_height(view: &View, width: u16) -> u16 {
    let inner = usize::from(width.saturating_sub(4));
    let rows = view.composer.rows(inner).clamp(1, MAX_ROWS);
    u16::try_from(rows).unwrap_or(1) + 2
}

/// The prompt in a box of the hat's color (the solo screen's rail layout):
/// the notes on its top edge, `keys` on its lower one. Returns the cursor
/// cell, if visible.
pub fn draw_boxed(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    theme: Theme,
    keys: Line<'static>,
) -> Option<(u16, u16)> {
    if area.height < 3 || area.width < 8 {
        return None;
    }
    let b = ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(border_color(view, theme)).bg(theme.bg))
        .title_top(Line::from(edge_notes(view, theme)).right_aligned())
        .title_bottom(keys.right_aligned())
        .style(Style::default().bg(theme.bg));
    let inner = b.inner(area);
    frame.render_widget(b, area);
    let inner = Rect {
        x: inner.x + 1,
        width: inner.width.saturating_sub(2),
        ..inner
    };
    body(frame, inner, view, theme, theme.bg)
}

fn ledger_frame(frame: &mut Frame, area: Rect, view: &View, theme: Theme) -> Rect {
    let w = area.width as usize;
    let rule = Style::default().fg(theme.dim).bg(theme.bg);
    let right = edge_notes(view, theme);
    let lead = Style::default().fg(border_color(view, theme)).bg(theme.bg);
    let right_w: usize = right.iter().map(|s| wrap::width(&s.content)).sum();
    let mut spans = vec![Span::styled("──", lead)];
    spans.push(Span::styled(
        "─".repeat(w.saturating_sub(2 + right_w + 2)),
        rule,
    ));
    spans.extend(right);
    spans.push(Span::styled("──", rule));
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect { height: 1, ..area },
    );
    Rect {
        y: area.y + 1,
        height: area.height - 1,
        ..area
    }
}

/// Draw the composer into `area` and return the cursor cell, if visible.
pub fn draw(frame: &mut Frame, area: Rect, view: &View, theme: Theme) -> Option<(u16, u16)> {
    let ledger = !view.ui.classic();
    if area.height < if ledger { 2 } else { 3 } || area.width < 8 {
        return None;
    }
    let border = border_color(view, theme);
    let bg = if ledger { theme.bg } else { theme.composer_bg };
    let bs = Style::default().fg(border).bg(bg);
    let w = area.width as usize;
    frame.render_widget(Paragraph::new("").style(Style::default().bg(bg)), area);
    let inner = if ledger {
        ledger_frame(frame, area, view, theme)
    } else {
        classic_frame(frame, area, view, theme, bs, bg, w)
    };
    body(frame, inner, view, theme, bg)
}

fn classic_frame(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    theme: Theme,
    bs: Style,
    bg: Color,
    w: usize,
) -> Rect {
    let border = border_color(view, theme);
    // Top border: ╭─ title ──────── queued ─╮ (no phase: there are none).
    let t = format!(" {} ", wrap::truncate(&title(view), w.saturating_sub(12)));
    let mut right = String::new();
    let mut right_style = Style::default().fg(theme.dim).bg(bg);
    if view.queued_prompt.is_some() {
        right = " queued ".into();
        right_style = Style::default()
            .fg(theme.warn)
            .bg(bg)
            .add_modifier(Modifier::BOLD);
    }
    let badge = if matches!(view.composer.mode, Mode::Normal) {
        format!(" {} ", view.mode_label().to_ascii_uppercase())
    } else {
        String::new()
    };
    let fill =
        w.saturating_sub(2 + 1 + wrap::width(&badge) + wrap::width(&t) + wrap::width(&right) + 1);
    let top = Line::from(vec![
        Span::styled("╭", bs),
        Span::styled("─", bs),
        Span::styled(
            badge,
            Style::default()
                .fg(theme.composer_bg)
                .bg(theme.mode(view.mode))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            t,
            Style::default()
                .fg(border)
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("─".repeat(fill), bs),
        Span::styled(right, right_style),
        Span::styled("─", bs),
        Span::styled("╮", bs),
    ]);
    frame.render_widget(Paragraph::new(top), Rect { height: 1, ..area });
    // Bottom border with optional counter (R-COMP-08).
    let count = view.composer.char_count();
    let counter = if count > COUNTER_AT && !matches!(view.composer.mode, Mode::Secret { .. }) {
        format!(" {} chars ", group_thousands(count))
    } else {
        String::new()
    };
    let fill = w.saturating_sub(2 + wrap::width(&counter) + 1);
    let bottom = Line::from(vec![
        Span::styled("╰", bs),
        Span::styled("─".repeat(fill), bs),
        Span::styled(counter, Style::default().fg(theme.dim).bg(bg)),
        Span::styled("─", bs),
        Span::styled("╯", bs),
    ]);
    frame.render_widget(
        Paragraph::new(bottom),
        Rect {
            y: area.y + area.height - 1,
            height: 1,
            ..area
        },
    );
    // Sides.
    {
        let buf = frame.buffer_mut();
        for y in (area.y + 1)..(area.y + area.height - 1) {
            for x in [area.x, area.x + area.width - 1] {
                if x < buf.area.width && y < buf.area.height {
                    let c = &mut buf[(x, y)];
                    c.set_symbol("│");
                    c.set_style(bs);
                }
            }
        }
    }
    Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width - 2,
        height: area.height - 2,
    }
}

/// The prompt and the text, inside whatever frame the layout drew.
fn body(
    frame: &mut Frame,
    inner: Rect,
    view: &View,
    theme: Theme,
    bg: Color,
) -> Option<(u16, u16)> {
    let g = glyph(view);
    let gw = wrap::width(g);
    let text_x = inner.x + gw as u16 + 1;
    let text_w = usize::from(inner.width).saturating_sub(gw + 2);
    let gstyle = Style::default()
        .fg(theme.prompt)
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    let text_style = Style::default().fg(theme.fg).bg(bg);
    let prefix = |first: bool| {
        if first {
            Span::styled(format!("{g} "), gstyle)
        } else {
            Span::styled(" ".repeat(gw + 1), Style::default().bg(bg))
        }
    };
    // Typing goes to the search row inside the panel: say so, rather than
    // echo it a screen away from the list it filters.
    if view.panels.inline_input(view) {
        let line = Line::from(vec![
            prefix(true),
            Span::styled(
                "typing goes to the search in the panel above",
                Style::default().fg(theme.dim).bg(bg),
            ),
        ]);
        frame.render_widget(Paragraph::new(line), Rect { height: 1, ..inner });
        return None;
    }
    // Secret mode: bullets only (R-COMP-07).
    if matches!(view.composer.mode, Mode::Secret { .. }) {
        let n = view.composer.secret_len();
        let dots = "•".repeat(n.min(text_w.saturating_sub(1)));
        let line = if n == 0 {
            Line::from(vec![
                prefix(true),
                Span::styled(placeholder(view), Style::default().fg(theme.dim).bg(bg)),
            ])
        } else {
            Line::from(vec![prefix(true), Span::styled(dots.clone(), text_style)])
        };
        frame.render_widget(Paragraph::new(line), Rect { height: 1, ..inner });
        let cx = text_x + wrap::width(&dots) as u16;
        return Some((cx.min(inner.x + inner.width - 1), inner.y));
    }
    if view.composer.is_empty() {
        let line = Line::from(vec![
            prefix(true),
            Span::styled(
                wrap::truncate(&placeholder(view), text_w),
                Style::default().fg(theme.dim).bg(bg),
            ),
        ]);
        frame.render_widget(Paragraph::new(line), Rect { height: 1, ..inner });
        return Some((text_x, inner.y));
    }
    let (rows, (crow, ccol)) = view.composer.layout(text_w);
    let visible = usize::from(inner.height);
    // Keep the cursor row on screen (R-COMP-09).
    let mut first = view
        .composer
        .scroll_row
        .min(rows.len().saturating_sub(visible));
    if crow < first {
        first = crow;
    } else if crow >= first + visible {
        first = crow + 1 - visible;
    }
    let text = view.composer.text();
    let mut lines = Vec::with_capacity(visible);
    for (i, (a, b)) in rows.iter().enumerate().skip(first).take(visible) {
        let seg: String = text[*a..*b]
            .graphemes(true)
            .filter(|g| *g != "\n")
            .collect();
        let seg = wrap::expand_tabs(&seg);
        lines.push(Line::from(vec![
            prefix(i == 0),
            Span::styled(seg, text_style),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
    if rows.len() > visible {
        // Overflow marker in the right column.
        let mark = if first + visible < rows.len() {
            "↓"
        } else {
            "↑"
        };
        let buf = frame.buffer_mut();
        let x = inner.x + inner.width - 1;
        let y = inner.y + inner.height - 1;
        if x < buf.area.width && y < buf.area.height {
            let c = &mut buf[(x, y)];
            c.set_symbol(mark);
            c.set_style(Style::default().fg(theme.dim).bg(bg));
        }
    }
    let cy = inner.y + (crow - first) as u16;
    let cx = text_x + ccol.min(text_w) as u16;
    Some((cx.min(inner.x + inner.width - 1), cy))
}

/// Paint the block cursor (`R-COMP-06`).
pub fn paint_cursor(frame: &mut Frame, at: Option<(u16, u16)>, theme: Theme) {
    let Some((x, y)) = at else {
        return;
    };
    let buf = frame.buffer_mut();
    if x >= buf.area.width || y >= buf.area.height {
        return;
    }
    let c = &mut buf[(x, y)];
    let sym = c.symbol().to_string();
    if sym.trim().is_empty() {
        c.set_symbol("█");
        c.set_style(Style::default().fg(theme.prompt).bg(theme.composer_bg));
    } else {
        c.set_style(
            Style::default()
                .fg(theme.composer_bg)
                .bg(theme.prompt)
                .add_modifier(Modifier::BOLD),
        );
    }
}
