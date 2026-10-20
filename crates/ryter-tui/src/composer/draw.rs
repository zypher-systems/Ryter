//! Composer rendering (`R-COMP-01..08`, `R-COMP-16`).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use unicode_segmentation::UnicodeSegmentation;

use super::{COUNTER_AT, MAX_ROWS, Mode, group_thousands};
use crate::chat::wrap;
use crate::theme::Theme;
use crate::view::View;

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

fn glyph(view: &View) -> &'static str {
    match view.composer.mode {
        Mode::Normal | Mode::Field { .. } => "›",
        Mode::Secret { .. } => "key",
    }
}

fn placeholder(view: &View) -> String {
    match &view.composer.mode {
        Mode::Normal if view.busy => "type to queue the next message, / for commands".into(),
        // The keys are on the hint bar; the prompt asks its question and
        // no more.
        Mode::Normal => match view.mode {
            ryter_core::Role::Build | ryter_core::Role::Crew => "what should change?".into(),
            ryter_core::Role::Plan => "what should we plan?".into(),
            ryter_core::Role::Audit => "ask about the audit".into(),
            ryter_core::Role::Scribe => "what should be documented?".into(),
        },
        Mode::Secret { connection } => format!("paste the API key for {connection}"),
        Mode::Field { label } => format!("type {label}…"),
    }
}

/// The hat's chip at the head of the prompt (` Build `). None while the
/// prompt is taking something other than a message.
fn chip(view: &View) -> Option<String> {
    matches!(view.composer.mode, Mode::Normal)
        .then(|| format!(" {} ", crate::glyphs::hat_label(view.mode)))
}

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

/// Rows the workspace prompt needs: its border, text, and breathing row.
pub fn workspace_height(view: &View, width: u16) -> u16 {
    let rows = view
        .composer
        .rows(workspace_text_width(view, width))
        .clamp(1, MAX_ROWS);
    u16::try_from(rows).unwrap_or(1) + 3
}

fn workspace_text_width(view: &View, width: u16) -> usize {
    let chip_w = chip(view).map_or(0, |c| wrap::width(&c) + 2);
    usize::from(width).saturating_sub(6 + chip_w)
}

/// The workspace prompt: a quiet border and a hat chip at the point of
/// typing. Queued state and long-message counts sit on the top edge.
/// Returns the cursor cell.
pub fn draw_workspace(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    theme: Theme,
) -> Option<(u16, u16)> {
    if area.height < 4 || area.width < 16 {
        return None;
    }
    let box_area = Rect {
        x: area.x + 1,
        y: area.y,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(1),
    };
    let bg = theme.composer_bg;
    let accent = border_color(view, theme);
    let edge = Style::default().fg(theme.rule).bg(bg);
    let top_edge = if view.composer.rejected || !matches!(view.composer.mode, Mode::Normal) {
        Style::default().fg(accent).bg(bg)
    } else {
        edge
    };
    frame.render_widget(Block::default().style(Style::default().bg(bg)), box_area);
    let w = usize::from(box_area.width);
    let notes = edge_notes(view, Theme { bg, ..theme });
    let notes_w: usize = notes.iter().map(|s| wrap::width(&s.content)).sum();
    let fits = notes_w + 3 <= w;
    let mut top = vec![
        Span::styled("╭", top_edge),
        Span::styled(
            "─".repeat(w.saturating_sub(if fits { notes_w + 3 } else { 3 })),
            top_edge,
        ),
    ];
    if fits {
        top.extend(notes);
    }
    top.push(Span::styled("─╮", top_edge));
    frame.render_widget(
        Paragraph::new(Line::from(top)),
        Rect {
            height: 1,
            ..box_area
        },
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("╰", edge),
            Span::styled("─".repeat(w.saturating_sub(2)), edge),
            Span::styled("╯", edge),
        ])),
        Rect {
            y: box_area.y + box_area.height - 1,
            height: 1,
            ..box_area
        },
    );
    {
        let buf = frame.buffer_mut();
        for y in (box_area.y + 1)..(box_area.y + box_area.height - 1) {
            for x in [box_area.x, box_area.x + box_area.width - 1] {
                if x < buf.area.width && y < buf.area.height {
                    let c = &mut buf[(x, y)];
                    c.set_symbol("│");
                    c.set_style(edge);
                }
            }
        }
    }
    let mut inner = Rect {
        x: box_area.x + 1,
        y: box_area.y + 1,
        width: box_area.width.saturating_sub(2),
        height: box_area.height.saturating_sub(2),
    };
    if let Some(c) = chip(view) {
        let fill = theme.mode(view.mode);
        let on = theme.bg;
        let spans = pill(&c, on, fill, bg, theme, crate::glyphs::round_caps(&view.ui));
        let span_w: u16 = spans.iter().map(|s| wrap::width(&s.content) as u16).sum();
        let chip_rect = Rect {
            width: span_w.min(inner.width),
            height: 1,
            ..inner
        };
        frame.render_widget(Paragraph::new(Line::from(spans)), chip_rect);
        view.marks.borrow_mut().chip = Some(chip_rect);
        let step = span_w.saturating_add(1).min(inner.width);
        inner.x += step;
        inner.width = inner.width.saturating_sub(step);
    }
    body(frame, inner, view, theme, bg, accent)
}

/// The hat's chip: the label on a fill, with round caps where the font
/// has them (`glyphs::pill`).
fn pill(
    label: &str,
    fg: Color,
    fill: Color,
    under: Color,
    theme: Theme,
    round: bool,
) -> Vec<Span<'static>> {
    // Without colour there is no fill: reverse video marks the chip.
    if theme.mode == crate::theme::ColorMode::Mono {
        return vec![Span::styled(
            label.to_string(),
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
        )];
    }
    crate::glyphs::pill(label.trim(), fg, fill, under, round)
}

/// The prompt and the text, inside whatever frame the layout drew. `mark`
/// is the prompt mark's color.
fn body(
    frame: &mut Frame,
    inner: Rect,
    view: &View,
    theme: Theme,
    bg: Color,
    mark: Color,
) -> Option<(u16, u16)> {
    let g = glyph(view);
    let gw = wrap::width(g);
    let text_x = inner.x + gw as u16 + 1;
    let text_w = usize::from(inner.width).saturating_sub(gw + 2);
    let gstyle = Style::default()
        .fg(mark)
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
