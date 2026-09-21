//! Frame assembly: header, body split, activity strip, composer, hint bar, overlays.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::chat::{layout, wrap};
use crate::composer::Mode as ComposerMode;
use crate::info::{self, CardId};
use crate::theme::Theme;
use crate::view::View;
use crate::{activity, composer, palette, panel};

/// Where things landed this frame, for mouse routing.
#[derive(Debug, Clone, Default)]
pub struct Hit {
    /// Chat pane.
    pub chat: Rect,
    /// Info panel cards.
    pub cards: Vec<(CardId, Rect)>,
    /// Activity strip.
    pub activity: Rect,
    /// Composer.
    pub composer: Rect,
}

/// Paint one frame.
pub fn draw(frame: &mut Frame, view: &View, theme: Theme) -> Hit {
    let full = frame.area();
    frame.render_widget(Block::default().style(theme.body()), full);
    if full.height < 6 || full.width < 20 {
        return Hit::default();
    }
    let composer_h =
        composer::draw::height(view, full.width).min(full.height.saturating_sub(8).max(3));
    let body_avail = full.height.saturating_sub(1 + 1 + 1 + composer_h + 1);
    let activity_h = activity::height(view, body_avail).min(body_avail.saturating_sub(8));
    let hairline_h = u16::from(activity_h > 0);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(8),
            Constraint::Length(activity_h),
            Constraint::Length(hairline_h),
            Constraint::Length(composer_h),
            Constraint::Length(1),
        ])
        .split(full);
    let (header, hair1, body, act, hair2, comp, hint) = (
        rows[0], rows[1], rows[2], rows[3], rows[4], rows[5], rows[6],
    );

    // A popout owns the whole body (`R-POP-01`). The info cards used to keep
    // their columns underneath it, so a centred panel covered their left half
    // and left shredded tails beside its border (`k-4.6`, `ns`, `ew 0/4`).
    // Dimming hid that in a real terminal but not on a monochrome capture, and
    // it cost Help the width it needs to be readable at 100 columns.
    let panel_w = if view.panel_visible && view.panels.is_empty() {
        info::width_for(full.width)
    } else {
        0
    };
    let cols = if panel_w > 0 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(40),
                Constraint::Length(1),
                Constraint::Length(panel_w),
            ])
            .split(body)
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(20), Constraint::Length(1)])
            .split(body)
    };
    let chat = cols[0];
    let gutter = cols[1];

    draw_header(frame, header, view, theme, panel_w == 0);
    hairline(frame, hair1, theme);
    let cf = draw_chat(frame, chat, view, theme);
    // A panel owns the scroll keys, so a live transcript scrollbar beside it is
    // both misleading and, next to a modal interrupt, visual noise on the one
    // screen that has to read as a single closed shape (`R-POP-75`).
    if view.panels.is_empty() {
        draw_scrollbar(frame, gutter, &cf, view.scroll.follow, theme);
    }
    let cards = if panel_w > 0 {
        info::draw(frame, cols[2], view, theme)
    } else {
        Vec::new()
    };
    if activity_h > 0 {
        activity::draw(frame, act, view, theme);
        hairline(frame, hair2, theme);
    }
    let cursor = composer::draw::draw(frame, comp, view, theme);
    draw_hint(frame, hint, view, theme);

    // Overlays, in z-order: palette, panels (which dim the body), cursor.
    if view.panels.is_empty() {
        palette::draw(frame, chat, comp.y, view, theme);
    }
    panel::draw(frame, full, body, view, theme);
    if view.panels.is_empty() || view.panels.wants_input(view).is_some() {
        composer::draw::paint_cursor(frame, cursor, theme);
    }
    Hit {
        chat,
        cards,
        activity: act,
        composer: comp,
    }
}

fn hairline(frame: &mut Frame, area: Rect, theme: Theme) {
    if area.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(area.width as usize),
            theme.muted(),
        ))),
        area,
    );
}

/// Header row (`R-HEAD-01..06`).
fn draw_header(frame: &mut Frame, area: Rect, view: &View, theme: Theme, compact_facts: bool) {
    let w = area.width as usize;
    let mut left: Vec<Span<'static>> = vec![
        Span::styled(" ryter", theme.muted()),
        Span::styled("  ·  ", theme.muted()),
    ];
    let speaker = match view.handoff_to() {
        Some(p) => format!("orchestrator → {p}"),
        None => "orchestrator".into(),
    };
    left.push(Span::styled(
        speaker,
        theme.body().add_modifier(Modifier::BOLD),
    ));
    let left_w: usize = left.iter().map(|s| wrap::width(&s.content)).sum();

    let mut right: Vec<Span<'static>> = Vec::new();
    if compact_facts {
        right.push(Span::styled(
            crate::chat::short_model(&view.model).to_string(),
            theme.body(),
        ));
        right.push(Span::styled(" · ", theme.muted()));
        let pct = (view.ctx_frac() * 100.0).round() as u32;
        right.push(Span::styled(
            format!("{pct}%"),
            theme.on_bg(crate::panel::widgets::gauge_color(view.ctx_frac(), theme)),
        ));
        right.push(Span::styled(" · ", theme.muted()));
        // Unknown spend is not "under budget"; it is unknown. Colouring it
        // green because `unwrap_or(0.0)` compared below the cap said so.
        let (label, style) = match view.spend {
            Some(spent) if view.budget_usd > 0.0 && spent >= view.budget_usd => {
                (format!("!{}", view.spend_label()), theme.on_bg(theme.error))
            }
            Some(spent) if view.warn_usd > 0.0 && spent >= view.warn_usd => {
                (view.spend_label(), theme.on_bg(theme.warn))
            }
            Some(_) => (view.spend_label(), theme.body()),
            None => (view.spend_label(), theme.muted()),
        };
        right.push(Span::styled(label, style));
        right.push(Span::styled("   ", theme.muted()));
    }
    let mut cwd = view.cwd.clone();
    if let Some(b) = &view.git_branch {
        cwd.push_str(&format!(" ({b})"));
    }
    right.push(Span::styled(cwd, theme.muted()));
    right.push(Span::styled(" ", theme.muted()));
    let mut right_w: usize = right.iter().map(|s| wrap::width(&s.content)).sum();
    if left_w + right_w + 2 > w {
        // Truncate the cwd first.
        let over = left_w + right_w + 2 - w;
        if let Some(sp) = right.iter_mut().rev().nth(1) {
            let t = wrap::truncate(&sp.content, wrap::width(&sp.content).saturating_sub(over));
            *sp = Span::styled(t, theme.muted());
        }
        right_w = right.iter().map(|s| wrap::width(&s.content)).sum();
    }
    let mut spans = left;
    let middle_room = w.saturating_sub(left_w + right_w);
    if w >= 100 && !view.session_title.trim().is_empty() && middle_room > 12 {
        let title = wrap::truncate(&view.session_title, middle_room.saturating_sub(4));
        let tw = wrap::width(&title);
        let lpad = (middle_room - tw) / 2;
        let rpad = middle_room - tw - lpad;
        spans.push(Span::styled(" ".repeat(lpad), theme.body()));
        spans.push(Span::styled(title, theme.muted()));
        spans.push(Span::styled(" ".repeat(rpad), theme.body()));
    } else {
        spans.push(Span::styled(" ".repeat(middle_room), theme.body()));
    }
    spans.extend(right);
    frame.render_widget(Paragraph::new(Line::from(spans)).style(theme.body()), area);
}

fn draw_chat(frame: &mut Frame, area: Rect, view: &View, theme: Theme) -> layout::ChatFrame {
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
        frame.render_widget(Paragraph::new(lines), Rect { height: 2, ..area });
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
        }
    }
    cf
}

/// Gutter scrollbar (`R-BAR-01..04`).
fn draw_scrollbar(
    frame: &mut Frame,
    area: Rect,
    cf: &layout::ChatFrame,
    follow: bool,
    theme: Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let h = usize::from(area.height);
    if !cf.resolved.overflow || cf.doc_rows == 0 {
        frame.render_widget(Block::default().style(theme.body()), area);
        return;
    }
    let thumb_h = ((h * h) / cf.doc_rows).max(1).min(h);
    let max_off = cf.doc_rows.saturating_sub(h).max(1);
    let thumb_y = (cf.resolved.offset.min(max_off) * (h - thumb_h)) / max_off;
    let thumb_color = if follow { theme.accent } else { theme.warn };
    let buf = frame.buffer_mut();
    for i in 0..h {
        let y = area.y + i as u16;
        if y >= buf.area.height || area.x >= buf.area.width {
            continue;
        }
        let c = &mut buf[(area.x, y)];
        if i >= thumb_y && i < thumb_y + thumb_h {
            c.set_symbol("┃");
            c.set_style(Style::default().fg(thumb_color).bg(theme.bg));
        } else {
            c.set_symbol("│");
            c.set_style(Style::default().fg(theme.dim).bg(theme.bg));
        }
    }
}

/// Context-sensitive hint bar.
pub fn hints(view: &View) -> Vec<(&'static str, String)> {
    if let Some(until) = view.quit_armed_until {
        if view.now_ms <= until {
            return vec![("^c", "press ^c again to quit".into())];
        }
    }
    if let Some(p) = view.panels.top() {
        let _ = p;
        return vec![
            ("↑↓", "move".into()),
            ("enter", "select".into()),
            ("tab", "next field".into()),
            ("?", "keys".into()),
            ("esc", "back".into()),
        ];
    }
    if view.palette.is_some() {
        return vec![
            ("↑↓", "move".into()),
            ("tab", "complete".into()),
            ("enter", "run".into()),
            ("→", "open".into()),
            ("esc", "close".into()),
        ];
    }
    match &view.composer.mode {
        ComposerMode::Handoff(p) => {
            return vec![
                ("enter", format!("hand off to {p}")),
                ("⇧enter", "newline".into()),
                ("esc", "cancel handoff".into()),
            ];
        }
        ComposerMode::Secret { .. } => {
            return vec![("enter", "save key".into()), ("esc", "cancel".into())];
        }
        _ => {}
    }
    let mut v = vec![
        ("enter", "send".to_string()),
        ("⇧enter", "newline".into()),
        ("/", "commands".into()),
    ];
    if view.activity.has_history {
        v.push(("^r", "reasoning".into()));
    }
    v.push(("^b", "panel".into()));
    if view.busy {
        v.push(("esc", "cancel".into()));
    } else if !view.scroll.follow {
        v.push(("end", "follow".into()));
    }
    v.push(("^c", "quit".into()));
    v
}

fn draw_hint(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    let w = area.width as usize;
    let mut spans: Vec<Span<'static>> = vec![Span::styled(" ", theme.body())];
    let mut used = 1;
    for (i, (key, label)) in hints(view).into_iter().enumerate() {
        let piece = wrap::width(key) + 1 + wrap::width(&label) + 4;
        if used + piece > w {
            break;
        }
        if i > 0 {
            spans.push(Span::styled("    ", theme.body()));
            used += 4;
        }
        spans.push(Span::styled(
            key.to_string(),
            theme.body().add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(" {label}"), theme.muted()));
        used += piece - 4;
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(theme.body()), area);
}

/// Render a frame to plain text (tests). Secret text never appears (`R-COMP-07`).
pub fn render_to_string(view: &View, width: u16, height: u16) -> String {
    render_with_theme(view, width, height, Theme::truecolor_dark())
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
