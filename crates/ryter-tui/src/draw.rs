//! Frame assembly: header, body split, activity strip, composer, hint bar, overlays.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::chat::{layout, wrap};
use crate::theme::Theme;
use crate::view::View;

/// Where things landed this frame, for mouse routing.
#[derive(Debug, Clone, Default)]
pub struct Hit {
    /// Chat pane.
    pub chat: Rect,
    /// The part of the chat pane its text is in: right of the timeline.
    pub chat_text: Rect,
    /// What was drawn over the chat pane: the pinned question, the "new
    /// rows" mark, the palette.
    pub covers: Vec<Rect>,
    /// Activity strip.
    pub activity: Rect,
    /// Composer.
    pub composer: Rect,
    /// Workspace chrome that a click activates.
    pub clicks: Vec<(crate::select::WorkspaceClick, Rect)>,
}

/// Paint one frame.
pub fn draw(frame: &mut Frame, view: &View, theme: Theme) -> Hit {
    view.covers.borrow_mut().clear();
    let mut hit = draw_screen(frame, view, theme);
    hit.covers = view.covers.borrow().clone();
    // The conversation's text starts after the pane's margin and its
    // timeline; its last column is the pane's margin too.
    let lead = 1 + layout::GUTTER as u16;
    hit.chat_text = Rect {
        x: hit.chat.x + lead.min(hit.chat.width),
        width: hit.chat.width.saturating_sub(lead + 1),
        ..hit.chat
    };
    let full = frame.area();
    // A panel over the conversation is not the conversation: a selection
    // begun before it opened is not painted across it.
    let hidden = |sel: &crate::select::Selection| sel.chat.is_some() && !view.panels.is_empty();
    if let Some(sel) = view.selection.as_ref().filter(|s| !hidden(s)) {
        sel.paint(
            frame.buffer_mut(),
            view.scroll.effective.get(),
            view.scroll.doc_rows.get(),
            &hit.covers,
            theme.selection_bg,
        );
    }
    if let Some((text, _)) = &view.flash {
        if full.height > 0 {
            let row = Rect {
                y: full.y + full.height - 1,
                height: 1,
                ..full
            };
            // The whole row: the keys under it would show through.
            frame.render_widget(ratatui::widgets::Clear, row);
            frame.render_widget(
                Paragraph::new(Span::styled(
                    format!(" {text}"),
                    Style::default()
                        .fg(theme.accent)
                        .bg(theme.panel_bg)
                        .add_modifier(Modifier::BOLD),
                ))
                .style(Style::default().bg(theme.panel_bg)),
                row,
            );
        }
    }
    hit
}

fn draw_screen(frame: &mut Frame, view: &View, theme: Theme) -> Hit {
    let full = frame.area();
    view.screen.set((full.width, full.height));
    frame.render_widget(Block::default().style(theme.body()), full);
    if full.height < 6 || full.width < 20 {
        return Hit::default();
    }
    crate::workspace::draw(frame, view, theme)
}

pub(crate) fn draw_chat(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    theme: Theme,
) -> layout::ChatFrame {
    let width = usize::from(area.width).saturating_sub(2).max(12);
    let cf = layout::frame(view, width, usize::from(area.height), theme);
    let lines: Vec<Line<'static>> = cf
        .lines
        .iter()
        .map(|l| {
            let mut spans = vec![Span::styled(" ", theme.body())];
            spans.extend(l.spans.iter().cloned());
            Line::from(spans)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).style(theme.body()), area);
    if !cf.sticky.is_empty() && area.height >= 3 {
        let lines: Vec<Line<'static>> = cf
            .sticky
            .iter()
            .map(|l| {
                let mut spans = vec![Span::styled(" ", Style::default().bg(theme.sticky_bg))];
                spans.extend(l.spans.iter().cloned());
                Line::from(spans)
            })
            .collect();
        let pinned = Rect { height: 2, ..area };
        frame.render_widget(Paragraph::new(lines), pinned);
        view.covers.borrow_mut().push(pinned);
    }
    // `↓ N new` pill (R-SCROLL-08).
    if cf.resolved.new_rows > 0 && !view.scroll.follow {
        let text = format!(" ↓ {} new ", cf.resolved.new_rows);
        let tw = wrap::width(&text) as u16;
        if area.width > tw + 2 && area.height > 2 {
            let r = Rect {
                x: area.x + area.width - tw - 1,
                y: area.y + area.height - 1,
                width: tw,
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(Span::styled(
                    text,
                    Style::default()
                        .fg(theme.accent)
                        .bg(theme.panel_bg)
                        .add_modifier(Modifier::BOLD),
                )),
                r,
            );
            view.covers.borrow_mut().push(r);
        }
    }
    cf
}

/// Render a frame to plain text (tests). Secret text never appears (`R-COMP-07`).
pub fn render_to_string(view: &View, width: u16, height: u16) -> String {
    render_with_theme(view, width, height, Theme::truecolor_dark())
}

/// Render, and return where things landed: for tests of the mouse.
#[cfg(test)]
pub fn render_hit(view: &View, width: u16, height: u16) -> Hit {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut hit = Hit::default();
    terminal
        .draw(|f| {
            hit = draw(f, view, Theme::truecolor_dark());
        })
        .expect("draw");
    hit
}

/// Render with a specific theme.
pub fn render_with_theme(view: &View, width: u16, height: u16, theme: Theme) -> String {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|f| {
            draw(f, view, theme);
        })
        .expect("draw");
    buffer_to_string(terminal.backend().buffer())
}

/// Where the live status row and the reasoning pane's header landed, as
/// one rectangle in the chat's area: a click there toggles the pane.
pub(crate) fn status_hit(area: Rect, cf: &layout::ChatFrame) -> Rect {
    let off = cf.resolved.offset;
    let rows: Vec<u16> = [cf.status_row, cf.pane_header]
        .into_iter()
        .flatten()
        .filter(|r| *r >= off && *r < off + usize::from(area.height))
        .filter_map(|r| u16::try_from(r - off).ok())
        .collect();
    let (Some(&top), Some(&bottom)) = (rows.iter().min(), rows.iter().max()) else {
        return Rect::default();
    };
    Rect {
        x: area.x,
        y: area.y + top,
        width: area.width,
        height: bottom - top + 1,
    }
}

/// Render and return the raw buffer (for style assertions in tests).
#[cfg(test)]
pub fn render_buffer(
    view: &View,
    width: u16,
    height: u16,
    theme: Theme,
) -> ratatui::buffer::Buffer {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|f| {
            draw(f, view, theme);
        })
        .expect("draw");
    terminal.backend().buffer().clone()
}

fn buffer_to_string(buf: &ratatui::buffer::Buffer) -> String {
    let mut out = String::new();
    for y in 0..buf.area.height {
        let mut line = String::new();
        for x in 0..buf.area.width {
            line.push_str(buf[(x, y)].symbol());
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}
