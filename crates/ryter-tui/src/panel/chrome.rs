//! Shared popout chrome: rounded border, title, status, legend, scrollbar (`R-POP-03`).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::chat::wrap;
use crate::theme::Theme;

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

/// Paint the frame and return the inner rect.
pub fn draw_frame(frame: &mut Frame, area: Rect, ch: &Chrome, theme: Theme) -> Rect {
    if area.width < 4 || area.height < 3 {
        return area;
    }
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

/// Scrollbar in the right border (`R-POP-06`).
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
            c.set_symbol("│");
            c.set_style(Style::default().fg(theme.dim).bg(theme.panel_bg));
        }
    }
}
