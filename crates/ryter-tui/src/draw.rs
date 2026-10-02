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
    if !view.ui.classic() {
        return draw_ledger(frame, view, theme);
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
    let panel_cursor = panel::draw(frame, full, body, view, theme);
    if panel_cursor.is_some() {
        composer::draw::paint_cursor(frame, panel_cursor, theme);
    } else if view.panels.is_empty() || view.panels.wants_input(view).is_some() {
        composer::draw::paint_cursor(frame, cursor, theme);
    }
    Hit {
        chat,
        cards,
        activity: act,
        composer: comp,
    }
}

/// Widest the ledger's reading column gets: the timeline gutter plus about
/// a hundred columns of text. Wider lines are harder to read, not better.
pub const LEDGER_COLUMN: u16 = 112;

/// The ledger (`[ui] layout = "ledger"`, the default): one reading column on
/// a timeline, centred; the composer beneath it as a single prompt line; and
/// one bar at the bottom for everything the header and cards used to say.
fn draw_ledger(frame: &mut Frame, view: &View, theme: Theme) -> Hit {
    let full = frame.area();
    if crate::rail::shown(view, full.width) {
        return draw_with_rail(frame, view, theme);
    }
    // The workbench takes the keys, so it has no composer.
    let composer_h = if view.workbench.is_some() {
        0
    } else {
        composer::draw::height(view, full.width).min(full.height.saturating_sub(6).max(2))
    };
    let body_avail = full.height.saturating_sub(composer_h + 2);
    let activity_h = activity::height(view, body_avail).min(body_avail.saturating_sub(6));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(4),
            Constraint::Length(activity_h),
            Constraint::Length(composer_h),
            Constraint::Length(1),
        ])
        .split(full);
    let (strip, body, act, comp_row, bar) = (rows[0], rows[1], rows[2], rows[3], rows[4]);
    draw_view_strip(frame, strip, view, theme);
    // The column, centred; one cell to its right is the scrollbar.
    let col_w = full.width.saturating_sub(2).min(LEDGER_COLUMN);
    let col_x = full.x + (full.width.saturating_sub(col_w + 1)) / 2;
    let column = |r: Rect| Rect {
        x: col_x,
        width: col_w,
        ..r
    };
    let mut chat = column(body);
    if let Some(w) = &view.workbench {
        let area = Rect {
            x: body.x + 1,
            width: body.width.saturating_sub(2),
            ..body
        };
        w.draw(frame, area, view, theme, |f, r| {
            draw_chat(f, r, view, theme);
        });
        chat = area;
    } else {
        let gutter = Rect {
            x: col_x + col_w,
            width: 1,
            ..body
        };
        let cf = draw_chat(frame, chat, view, theme);
        if view.panels.is_empty() {
            draw_scrollbar(frame, gutter, &cf, view.scroll.follow, theme);
        }
    }
    let act = column(act);
    if activity_h > 0 {
        activity::draw(frame, act, view, theme);
    }
    let comp = column(comp_row);
    let cursor = if composer_h > 0 {
        composer::draw::draw(frame, comp, view, theme)
    } else {
        None
    };
    draw_status_bar(frame, bar, view, theme);
    if view.panels.is_empty() {
        palette::draw(frame, chat, comp.y, view, theme);
    }
    let panel_cursor = panel::draw(frame, full, body, view, theme);
    if panel_cursor.is_some() {
        composer::draw::paint_cursor(frame, panel_cursor, theme);
    } else if view.panels.is_empty() || view.panels.wants_input(view).is_some() {
        composer::draw::paint_cursor(frame, cursor, theme);
    }
    Hit {
        chat,
        cards: Vec::new(),
        activity: act,
        composer: comp,
    }
}

/// Solo mode with the side rail (design S2): the rail on the left at full
/// height; the conversation, the activity strip and a prompt boxed in the
/// hat's color on the right. The rail says what the strip and the bottom bar
/// said, so neither is drawn; the keys go on the prompt's border.
fn draw_with_rail(frame: &mut Frame, view: &View, theme: Theme) -> Hit {
    let full = frame.area();
    let rail = Rect {
        width: crate::rail::RAIL_W,
        ..full
    };
    crate::rail::draw(frame, rail, view, theme);
    let main = Rect {
        x: full.x + rail.width,
        width: full.width.saturating_sub(rail.width),
        ..full
    };
    let col_w = main.width.saturating_sub(4).min(LEDGER_COLUMN);
    let col_x = main.x + (main.width.saturating_sub(col_w + 1)) / 2;
    let prompt_h =
        composer::draw::boxed_height(view, col_w).min(main.height.saturating_sub(8).max(3));
    let body_avail = main.height.saturating_sub(prompt_h + 1);
    let activity_h = activity::height(view, body_avail).min(body_avail.saturating_sub(6));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(4),
            Constraint::Length(activity_h),
            Constraint::Length(prompt_h),
        ])
        .split(main);
    let (body, act, prompt_row) = (rows[1], rows[2], rows[3]);
    let column = |r: Rect| Rect {
        x: col_x,
        width: col_w,
        ..r
    };
    // Whose conversation this is, when it is the tester's.
    if let Some(line) = thread_line(view, theme, usize::from(col_w)) {
        frame.render_widget(
            Paragraph::new(line).style(Style::default().bg(theme.bg)),
            column(rows[0]),
        );
    }
    let chat = column(body);
    let cf = draw_chat(frame, chat, view, theme);
    if view.panels.is_empty() {
        let gutter = Rect {
            x: col_x + col_w,
            width: 1,
            ..body
        };
        draw_scrollbar(frame, gutter, &cf, view.scroll.follow, theme);
    }
    let act = column(act);
    if activity_h > 0 {
        activity::draw(frame, act, view, theme);
    }
    // The prompt, boxed in the hat's color, the keys on its lower edge.
    let comp = column(prompt_row);
    let keys = prompt_keys(view, theme, usize::from(comp.width.saturating_sub(4)));
    let cursor = composer::draw::draw_boxed(frame, comp, view, theme, keys);
    if view.panels.is_empty() {
        palette::draw(frame, chat, comp.y, view, theme);
    }
    let panel_cursor = panel::draw(frame, full, main, view, theme);
    if panel_cursor.is_some() {
        composer::draw::paint_cursor(frame, panel_cursor, theme);
    } else if view.panels.is_empty() || view.panels.wants_input(view).is_some() {
        composer::draw::paint_cursor(frame, cursor, theme);
    }
    Hit {
        chat,
        cards: Vec::new(),
        activity: act,
        composer: comp,
    }
}

/// The line over the tester's conversation: `TEST THREAD · tab: main chat`.
/// `None` for the conversation the other hats share, which needs no name.
fn thread_line(view: &View, theme: Theme, width: usize) -> Option<Line<'static>> {
    if view.shown != ryter_core::Thread::Test {
        return None;
    }
    let dim = Style::default().fg(theme.dim).bg(theme.bg);
    let name = "TEST THREAD";
    let rest = match view.test_runs {
        0 => " · its own conversation · tab: main chat".to_string(),
        1 => " · 1 run this session · tab: main chat".to_string(),
        n => format!(" · {n} runs this session · tab: main chat"),
    };
    let rest = rest.as_str();
    let rest = wrap::truncate(rest, width.saturating_sub(wrap::width(name)));
    Some(Line::from(vec![
        Span::styled(
            name,
            Style::default()
                .fg(theme.mode(ryter_core::Role::SoloTest))
                .bg(theme.bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(rest, dim),
    ]))
}

/// The keys that matter now, for the prompt's border: the bottom bar's, less
/// the least needed until they fit in `width`.
fn prompt_keys(view: &View, theme: Theme, width: usize) -> Line<'static> {
    let mut keys = match view.panels.top() {
        Some(p) => legend_keys(&p.legend(view)),
        None => hints_ranked(view),
    };
    if view.panels.top().is_none() && view.palette.is_none() && view.composer.is_empty() {
        keys.insert(0, ("$".into(), "spend".into(), Hint::Useful));
    }
    // Three columns between keys, and one after the last: the gap before
    // the border's corner is not worth a key.
    let used = |keys: &[(String, String, Hint)]| -> usize {
        (keys
            .iter()
            .map(|(k, l, _)| wrap::width(k) + 1 + wrap::width(l) + 3)
            .sum::<usize>()
            + 1)
        .saturating_sub(2)
    };
    while used(&keys) > width && keys.len() > 1 {
        let worst = keys
            .iter()
            .enumerate()
            .max_by_key(|(i, (_, _, rank))| (*rank, *i))
            .map_or(keys.len() - 1, |(i, _)| i);
        keys.remove(worst);
    }
    let mut spans = vec![Span::styled(" ", Style::default().bg(theme.bg))];
    let last = keys.len().saturating_sub(1);
    for (i, (k, l, _)) in keys.into_iter().enumerate() {
        spans.push(Span::styled(
            k,
            Style::default()
                .fg(theme.accent)
                .bg(theme.bg)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" {l}{}", if i == last { " " } else { "   " }),
            Style::default().fg(theme.dim).bg(theme.bg),
        ));
    }
    Line::from(spans)
}

/// Which view the ledger is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerView {
    /// The conversation.
    Chat,
    /// The workbench.
    Changes,
}

/// The view on screen now.
pub fn ledger_view(view: &View) -> LedgerView {
    if view.workbench.is_some() {
        LedgerView::Changes
    } else {
        LedgerView::Chat
    }
}

/// The strip across the top: the views there are, the one on screen lit,
/// and the key to each. Without it the workbench was there but out of
/// sight.
fn draw_view_strip(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    let now = ledger_view(view);
    let dim = Style::default().fg(theme.dim).bg(theme.bg);
    let tab = |label: &str, key: &str, this: LedgerView| -> Vec<Span<'static>> {
        let on = now == this;
        let style = if on {
            Style::default()
                .fg(theme.bg)
                .bg(theme.mode(view.mode))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg).bg(theme.bg)
        };
        let mut v = vec![Span::styled(format!(" {label} "), style)];
        if !key.is_empty() {
            v.push(Span::styled(format!(" {key}"), dim));
        }
        v.push(Span::styled("   ", dim));
        v
    };
    let mut spans = vec![Span::styled(" ", dim)];
    spans.extend(tab(
        // The tester's conversation is named; the shared one is "chat".
        if view.shown == ryter_core::Thread::Test {
            "test thread"
        } else {
            "chat"
        },
        if now == LedgerView::Changes {
            "esc"
        } else {
            ""
        },
        LedgerView::Chat,
    ));
    spans.extend(tab("changes", "^t", LedgerView::Changes));
    let used: usize = spans.iter().map(|s| wrap::width(&s.content)).sum();
    let title = view.session_title.trim();
    if !title.is_empty() && used + 8 < area.width as usize {
        let t = wrap::truncate(title, area.width as usize - used - 2);
        let pad = (area.width as usize).saturating_sub(used + wrap::width(&t) + 1);
        spans.push(Span::styled(" ".repeat(pad), dim));
        spans.push(Span::styled(t, dim));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(theme.bg)),
        area,
    );
}

/// The ledger's bottom bar: who you're talking to, where, with what, how
/// full the context is, what it has cost (this turn, the session, the
/// project) against the budget, and the keys that matter now. Facts drop
/// from the least needed when it doesn't fit; the mode and cost stay.
fn draw_status_bar(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    let w = area.width as usize;
    let bar_bg = theme.panel_bg;
    let on = |fg| Style::default().fg(fg).bg(bar_bg);
    let mode = if view.workbench.is_some() {
        " WORKBENCH ".to_string()
    } else {
        format!(" {} ", view.mode_label().to_ascii_uppercase())
    };
    let mode_span = Span::styled(
        mode,
        Style::default()
            .fg(theme.bg)
            .bg(theme.mode(view.mode))
            .add_modifier(Modifier::BOLD),
    );
    let project = std::path::Path::new(&view.cwd)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| view.cwd.clone());
    let place = match &view.git_branch {
        Some(b) => format!(" {project}  {b} "),
        None => format!(" {project} "),
    };
    let model = format!(" {} ", crate::chat::short_model(view.hat_model()));
    let frac = view.ctx_frac();
    let filled = ((frac * 8.0).round() as usize).min(8);
    let gauge = [
        Span::styled(" ctx ", on(theme.dim)),
        Span::styled(
            "▰".repeat(filled),
            on(crate::panel::widgets::gauge_color(frac, theme)),
        ),
        Span::styled("▱".repeat(8 - filled), on(theme.dim)),
        Span::styled(
            format!(" {}% ", (frac * 100.0).round() as u32),
            on(theme.dim),
        ),
    ];
    // Cost: the turn, the session, the project, and the budget, together.
    let turn = view
        .spend
        .map(|now| now - view.turn_spend_from.unwrap_or(0.0))
        .map(crate::chat::turn_usd);
    let session_style = match view.spend {
        Some(s) if view.budget_usd > 0.0 && s >= view.budget_usd => on(theme.error),
        Some(s) if view.warn_usd > 0.0 && s >= view.warn_usd => on(theme.warn),
        Some(_) => on(theme.fg),
        None => on(theme.dim),
    };
    let mut cost: Vec<Span<'static>> = vec![Span::styled("│ ", on(theme.dim))];
    if let Some(t) = &turn {
        cost.push(Span::styled("turn ", on(theme.dim)));
        cost.push(Span::styled(t.clone(), on(theme.fg)));
        cost.push(Span::styled(" · ", on(theme.dim)));
    }
    cost.push(Span::styled("session ", on(theme.dim)));
    // Nothing spent yet is $0, not unknown; a few cents keep their digits.
    let session = match view.spend {
        None if !view.spend_unknown => "$0".to_string(),
        Some(s) if !view.spend_unknown => crate::chat::turn_usd(s),
        _ => view.spend_label(),
    };
    cost.push(Span::styled(session, session_style));
    let project_cost = view.project_spend.as_ref().map(|p| {
        let known = crate::chat::turn_usd(p.total_usd);
        if p.unpriced_calls > 0 {
            format!("≥{known}")
        } else {
            known
        }
    });
    let mut project_spans = Vec::new();
    if let Some(p) = &project_cost {
        project_spans.push(Span::styled(" · project ", on(theme.dim)));
        project_spans.push(Span::styled(p.clone(), on(theme.fg)));
    }
    let budget = if view.budget_usd > 0.0 {
        format!(" · budget ${:.2} ", view.budget_usd)
    } else {
        " · budget off ".to_string()
    };
    let budget_span = Span::styled(budget, on(theme.dim));

    // Keys, as the hint bar ranks them, with the spend drawer first; an open
    // panel's own keys while it is open.
    let mut keys = match (view.panels.top(), &view.workbench) {
        (Some(p), _) => legend_keys(&p.legend(view)),
        (None, Some(w)) => legend_keys(&w.legend()),
        (None, None) => hints_ranked(view),
    };
    if view.panels.top().is_none()
        && view.palette.is_none()
        && view.workbench.is_none()
        && view.composer.is_empty()
    {
        keys.insert(0, ("$".into(), "spend".into(), Hint::Useful));
    }
    let key_spans = |keys: &[(String, String, Hint)]| -> Vec<Span<'static>> {
        let mut v = Vec::new();
        for (k, l, _) in keys {
            v.push(Span::styled(
                k.clone(),
                on(theme.accent).add_modifier(Modifier::BOLD),
            ));
            v.push(Span::styled(format!(" {l}  "), on(theme.dim)));
        }
        v
    };
    let width_of =
        |spans: &[Span<'static>]| spans.iter().map(|s| wrap::width(&s.content)).sum::<usize>();

    // Fit: drop keys first (least needed first), then the project name, the
    // model, the gauge, the turn and project costs.
    let mut show_place = true;
    let mut show_model = true;
    let mut show_gauge = true;
    let mut show_project = true;
    let left = |show_place: bool, show_model: bool, show_gauge: bool, show_project: bool| {
        let mut v = vec![mode_span.clone()];
        if show_place {
            v.push(Span::styled(
                place.clone(),
                Style::default().fg(theme.fg).bg(theme.sticky_bg),
            ));
        }
        if show_model {
            v.push(Span::styled(model.clone(), on(theme.dim)));
        }
        if show_gauge {
            v.extend(gauge.iter().cloned());
        }
        v.extend(cost.iter().cloned());
        if show_project {
            v.extend(project_spans.iter().cloned());
        }
        v.push(budget_span.clone());
        v
    };
    // In the workbench the keys are the point: facts go before they do.
    let keys_first = view.workbench.is_some();
    loop {
        let l = left(show_place, show_model, show_gauge, show_project);
        let used = width_of(&l) + width_of(&key_spans(&keys));
        if used <= w {
            break;
        }
        if keys_first && (show_model || show_gauge || show_place) {
            if show_model {
                show_model = false;
            } else if show_gauge {
                show_gauge = false;
            } else {
                show_place = false;
            }
            continue;
        }
        if keys.len() > 1 {
            let worst = keys
                .iter()
                .enumerate()
                .max_by_key(|(i, (_, _, rank))| (*rank, *i))
                .map(|(i, _)| i);
            if let Some(i) = worst {
                if keys[i].2 != Hint::Essential || keys.len() > 2 {
                    keys.remove(i);
                    continue;
                }
            }
        }
        if show_place {
            show_place = false;
        } else if show_model {
            show_model = false;
        } else if show_gauge {
            show_gauge = false;
        } else if show_project {
            show_project = false;
        } else if !keys.is_empty() {
            keys.pop();
        } else {
            break;
        }
    }
    let mut spans = left(show_place, show_model, show_gauge, show_project);
    let ks = key_spans(&keys);
    let pad = w.saturating_sub(width_of(&spans) + width_of(&ks));
    spans.push(Span::styled(" ".repeat(pad), on(theme.dim)));
    spans.extend(ks);
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(bar_bg)),
        area,
    );
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
    // The hat the next message goes out in.
    let speaker = view.mode_label().to_string();
    left.push(Span::styled(
        speaker,
        Style::default()
            .fg(theme.mode(view.mode))
            .bg(theme.bg)
            .add_modifier(Modifier::BOLD),
    ));
    let left_w: usize = left.iter().map(|s| wrap::width(&s.content)).sum();

    let mut right: Vec<Span<'static>> = Vec::new();
    if compact_facts {
        right.push(Span::styled(
            crate::chat::short_model(view.hat_model()).to_string(),
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
/// How willing a hint is to be dropped when the bar does not fit.
///
/// The old bar truncated the tail, so the 80-column streaming frame lost `^c
/// quit` — the one key you want while a turn is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Hint {
    /// Never dropped: cancel and quit.
    Essential,
    /// Dropped only after every optional hint has gone.
    Useful,
    /// First to go.
    Optional,
}

/// Context hints with their priority.
pub fn hints_ranked(view: &View) -> Vec<(String, String, Hint)> {
    hints(view)
        .into_iter()
        .map(|(k, l)| {
            let rank = match k.as_str() {
                // Getting out: cancel, quit, and the permission answers.
                "^c" | "esc" | "^d" | "y" | "n" | "a" | "⏎" => Hint::Essential,
                // The lanes' reasoning switch is on the board alone.
                "^r" if l.ends_with(" reasoning") => Hint::Useful,
                // Discoverable without the bar, so first to go.
                "⇧enter" | "^r" | "end" => Hint::Optional,
                // `enter` included: everyone knows Enter sends.
                _ => Hint::Useful,
            };
            (k, l, rank)
        })
        .collect()
}

pub fn hints(view: &View) -> Vec<(String, String)> {
    hints_static(view)
        .into_iter()
        .map(|(k, l)| (k.to_string(), l))
        .collect()
}

fn hints_static(view: &View) -> Vec<(&'static str, String)> {
    if let Some(until) = view.quit_armed_until {
        if view.now_ms <= until {
            return vec![("^c", "press ^c again to quit".into())];
        }
    }
    if view.panels.top().is_some() {
        // The open panel's own keys, as its last row lists them: the bar
        // used to offer form keys (`tab next field`) even on a permission
        // prompt, which takes ⏎, a and n.
        return Vec::new();
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
    if let ComposerMode::Secret { .. } = &view.composer.mode {
        return vec![("enter", "save key".into()), ("esc", "cancel".into())];
    }
    let mut v = vec![("enter", "send".to_string())];
    // The one key that isn't discoverable any other way.
    v.push(("tab", view.mode.next_hat().as_str().to_string()));
    v.push(("⇧enter", "newline".into()));
    v.push(("/", "commands".into()));
    if !view.ui.classic() {
        v.push(("^t", "changes".into()));
    }
    // Finished turns fold to one line on the ledger; ^O opens them.
    if !view.ui.classic() && !view.diffs_expanded && view.has_folded_turns() {
        v.push(("^o", "expand".into()));
    }
    if view.activity.has_history {
        v.push(("^r", "reasoning".into()));
    }
    // `^b` shows and hides the info panel on the classic screen, the rail
    // on the ledger.
    if view.ui.classic() {
        v.push(("^b", "panel".into()));
    } else {
        let label = if view.panel_visible {
            "hide rail"
        } else {
            "show rail"
        };
        v.push(("^b", label.into()));
    }
    if view.busy {
        v.push(("esc", "cancel".into()));
    } else if !view.scroll.follow {
        v.push(("end", "follow".into()));
    }
    v.push(("^c", "quit".into()));
    v
}

/// Width one hint occupies, including its leading separator.
fn hint_width(key: &str, label: &str, first: bool) -> usize {
    wrap::width(key) + 1 + wrap::width(label) + if first { 0 } else { 4 }
}

/// A panel's legend (`⏎ allow · a allow edits · n deny`) as ranked hints.
fn legend_keys(legend: &str) -> Vec<(String, String, Hint)> {
    legend
        .split(" · ")
        .filter(|i| !i.trim().is_empty())
        .map(|i| {
            let (k, l) = i.split_once(' ').unwrap_or((i, ""));
            let rank = match k {
                "esc" | "⏎" | "y" | "n" | "a" => Hint::Essential,
                _ => Hint::Useful,
            };
            (k.to_string(), l.to_string(), rank)
        })
        .collect()
}

fn draw_hint(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    let w = area.width as usize;
    let mut items = hints_ranked(view);
    if let Some(p) = view.panels.top() {
        items = legend_keys(&p.legend(view));
    }
    // Drop the least important hints until the rest fit, rather than chopping
    // whatever happens to be last.
    loop {
        let mut used = 1;
        for (i, (k, l, _)) in items.iter().enumerate() {
            used += hint_width(k, l, i == 0);
        }
        if used <= w || items.len() <= 1 {
            break;
        }
        let worst = items
            .iter()
            .enumerate()
            .max_by_key(|(i, (_, _, rank))| (*rank, *i))
            .map(|(i, _)| i);
        match worst {
            Some(i) => {
                items.remove(i);
            }
            None => break,
        }
    }
    let mut spans: Vec<Span<'static>> = vec![Span::styled(" ", theme.body())];
    for (i, (key, label, _)) in items.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("    ", theme.body()));
        }
        spans.push(Span::styled(
            key.clone(),
            theme.body().add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(" {label}"), theme.muted()));
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
