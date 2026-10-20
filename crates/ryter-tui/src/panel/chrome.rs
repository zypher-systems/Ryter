//! Shared popout chrome (`R-POP-03`): a slab with cut corners where the
//! screen has a tint to lift it on, a rounded line box where it has not.
//!
//! The slab is the panel's own background with a lid and a foot of half
//! blocks, their corners quarter blocks, so nothing on it is a right
//! angle; the title is its first row, in the border's colour, the status
//! at that row's right. In sixteen colours the panel's background is the
//! screen's, so a slab would vanish: the line box stays there, and
//! without colour.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::chat::wrap;
use crate::theme::{ColorMode, Theme};

/// Border decorations.
#[derive(Debug, Clone)]
pub struct Chrome {
    /// Top-left.
    pub title: String,
    /// Top-right.
    pub status: String,
    /// Bottom.
    pub legend: String,
    /// Border color.
    pub border: Color,
    /// Heavier top border for modal interrupts (`R-POP-75`).
    pub heavy: bool,
}

/// Whether the theme lifts a slab: a panel tint of its own, in colour.
pub fn slab(theme: Theme) -> bool {
    !matches!(theme.mode, ColorMode::Mono | ColorMode::Ansi16) && theme.panel_bg != theme.bg
}

/// Rows the frame takes around a panel's content: the lid, the title row
/// and the foot of a slab; the two border rows of a line box.
pub fn frame_rows(theme: Theme) -> u16 {
    if slab(theme) { 3 } else { 2 }
}

/// Paint the frame and return the inner rect.
pub fn draw_frame(frame: &mut Frame, area: Rect, ch: &Chrome, theme: Theme) -> Rect {
    if area.width < 4 || area.height < 3 {
        return area;
    }
    if slab(theme) {
        return draw_slab(frame, area, ch, theme);
    }
    draw_box(frame, area, ch, theme)
}

fn draw_slab(frame: &mut Frame, area: Rect, ch: &Chrome, theme: Theme) -> Rect {
    frame.render_widget(Clear, area);
    let under = Style::default().fg(theme.panel_bg).bg(theme.bg);
    let w = usize::from(area.width);
    let lid = Line::from(vec![
        Span::styled("▗", under),
        Span::styled("▄".repeat(w.saturating_sub(2)), under),
        Span::styled("▖", under),
    ]);
    frame.render_widget(Paragraph::new(lid), Rect { height: 1, ..area });
    let body = Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(2),
        ..area
    };
    frame.render_widget(Block::default().style(theme.panel()), body);
    let foot = Line::from(vec![
        Span::styled("▝", under),
        Span::styled("▀".repeat(w.saturating_sub(2)), under),
        Span::styled("▘", under),
    ]);
    frame.render_widget(
        Paragraph::new(foot),
        Rect {
            y: area.y + area.height - 1,
            height: 1,
            ..area
        },
    );
    // The title row: the title in the border's colour, the status at the
    // right in the panel's quiet text.
    let inner_w = w.saturating_sub(2);
    let title_style = Style::default()
        .fg(ch.border)
        .bg(theme.panel_bg)
        .add_modifier(Modifier::BOLD);
    let title = if ch.title.is_empty() {
        String::new()
    } else {
        format!(" {}", wrap::truncate(&ch.title, inner_w.saturating_sub(2)))
    };
    let status = if ch.status.is_empty() {
        String::new()
    } else {
        format!(
            "{} ",
            wrap::truncate(&ch.status, inner_w.saturating_sub(wrap::width(&title) + 2))
        )
    };
    let fill = inner_w.saturating_sub(wrap::width(&title) + wrap::width(&status));
    if !title.is_empty() || !status.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(title, title_style),
                Span::styled(" ".repeat(fill), theme.panel()),
                Span::styled(status, theme.panel_muted()),
            ])),
            Rect {
                x: area.x + 1,
                y: area.y + 1,
                width: area.width.saturating_sub(2),
                height: 1,
            },
        );
    }
    if !ch.legend.is_empty() && area.height > 4 {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(" {}", wrap::truncate(&ch.legend, inner_w.saturating_sub(1))),
                theme.panel_muted(),
            ))),
            Rect {
                x: area.x + 1,
                y: area.y + area.height - 2,
                width: area.width.saturating_sub(2),
                height: 1,
            },
        );
    }
    Rect {
        x: area.x + 1,
        y: area.y + 2,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(3),
    }
}

fn draw_box(frame: &mut Frame, area: Rect, ch: &Chrome, theme: Theme) -> Rect {
    // Reset glyphs first: a styled `Block` alone leaves the underlying frame's
    // characters in place wherever the body does not write (`R-POP-03`).
    frame.render_widget(Clear, area);
    frame.render_widget(Block::default().style(theme.panel()), area);
    let bs = Style::default().fg(ch.border).bg(theme.panel_bg);
    let title_style = Style::default()
        .fg(theme.panel_title)
        .bg(theme.panel_bg)
        .add_modifier(Modifier::BOLD);
    let w = area.width as usize;
    let hz = if ch.heavy { "━" } else { "─" };
    // Top: ╭─ title ───── status ─╮
    let title = if ch.title.is_empty() {
        String::new()
    } else {
        format!(" {} ", wrap::truncate(&ch.title, w.saturating_sub(8)))
    };
    let status = if ch.status.is_empty() {
        String::new()
    } else {
        format!(
            " {} ",
            wrap::truncate(&ch.status, w.saturating_sub(title.len() + 8))
        )
    };
    let fill = w.saturating_sub(2 + 1 + wrap::width(&title) + wrap::width(&status) + 1);
    let top = Line::from(vec![
        Span::styled(if ch.heavy { "┏" } else { "╭" }, bs),
        Span::styled(hz, bs),
        Span::styled(title, title_style),
        Span::styled(hz.repeat(fill), bs),
        Span::styled(status, Style::default().fg(theme.dim).bg(theme.panel_bg)),
        Span::styled(hz, bs),
        Span::styled(if ch.heavy { "┓" } else { "╮" }, bs),
    ]);
    frame.render_widget(Paragraph::new(top), Rect { height: 1, ..area });
    // Sides.
    for y in (area.y + 1)..(area.y + area.height - 1) {
        let buf = frame.buffer_mut();
        for x in [area.x, area.x + area.width - 1] {
            if x < buf.area.width && y < buf.area.height {
                let c = &mut buf[(x, y)];
                c.set_symbol("│");
                c.set_style(bs);
            }
        }
    }
    // Bottom: ╰─ legend ────╯
    let legend = if ch.legend.is_empty() {
        String::new()
    } else {
        format!(" {} ", wrap::truncate(&ch.legend, w.saturating_sub(6)))
    };
    let fill = w.saturating_sub(2 + 1 + wrap::width(&legend));
    let bottom = Line::from(vec![
        Span::styled("╰", bs),
        Span::styled("─", bs),
        Span::styled(legend, Style::default().fg(theme.dim).bg(theme.panel_bg)),
        Span::styled("─".repeat(fill), bs),
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
    Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

/// Scrollbar down the right edge (`R-POP-06`): in the border of a line
/// box, in the slab's right pad.
pub fn scrollbar(
    frame: &mut Frame,
    area: Rect,
    first: usize,
    total: usize,
    visible: usize,
    theme: Theme,
) {
    if total <= visible || visible == 0 || area.height < 4 {
        return;
    }
    let track_h = (area.height - 2) as usize;
    let thumb_h = ((visible * track_h) / total).max(1);
    let max_first = total.saturating_sub(visible).max(1);
    let thumb_y = (first.min(max_first) * (track_h - thumb_h)) / max_first;
    let x = area.x + area.width - 1;
    let track = if slab(theme) { "┆" } else { "│" };
    let buf = frame.buffer_mut();
    for i in 0..track_h {
        let y = area.y + 1 + i as u16;
        if x >= buf.area.width || y >= buf.area.height {
            continue;
        }
        let c = &mut buf[(x, y)];
        if i >= thumb_y && i < thumb_y + thumb_h {
            c.set_symbol("┃");
            c.set_style(Style::default().fg(theme.accent).bg(theme.panel_bg));
        } else {
            c.set_symbol(track);
            c.set_style(Style::default().fg(theme.dim).bg(theme.panel_bg));
        }
    }
}
