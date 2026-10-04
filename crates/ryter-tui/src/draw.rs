//! Frame assembly: header, body split, activity strip, composer, hint bar, overlays.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ryter_core::Role;

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
    view.screen.set((full.width, full.height));
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
    // One spinner: the status row in the conversation. The strip's rows
    // are gone from this screen; the reasoning pane opens under the row.
    let activity_h: u16 = 0;
    let hairline_h: u16 = 0;
    // A question takes rows of its own under the chat, so the reply's
    // last lines stay in sight above it.
    let prompt_h = panel::prompt_height(view, body_avail.saturating_sub(activity_h + hairline_h));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(8),
            Constraint::Length(prompt_h),
            Constraint::Length(activity_h),
            Constraint::Length(hairline_h),
            Constraint::Length(composer_h),
            Constraint::Length(1),
        ])
        .split(full);
    let (header, hair1, body, slot_row, _act, _hair2, comp, hint) = (
        rows[0], rows[1], rows[2], rows[3], rows[4], rows[5], rows[6], rows[7],
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
    let slot = (prompt_h > 0).then_some(Rect {
        x: chat.x,
        width: chat.width,
        ..slot_row
    });

    draw_header(frame, header, view, theme, panel_w == 0);
    hairline(frame, hair1, theme);
    let cf = draw_chat(frame, chat, view, theme);
    // A panel owns the scroll keys, so a live transcript scrollbar beside it is
    // both misleading and, next to a modal interrupt, visual noise on the one
    // screen that has to read as a single closed shape (`R-POP-75`).
    if view.panels.is_empty() {
        draw_scrollbar(frame, gutter, &cf, view.scroll.follow, theme, true);
    }
    let cards = if panel_w > 0 {
        info::draw(frame, cols[2], view, theme)
    } else {
        Vec::new()
    };
    let cursor = composer::draw::draw(frame, comp, view, theme);
    draw_hint(frame, hint, view, theme);

    // Overlays, in z-order: palette, panels (which dim the body), cursor.
    if view.panels.is_empty() {
        palette::draw(frame, chat, comp.y, view, theme);
    }
    let panel_cursor = panel::draw(frame, full, body, slot, view, theme);
    if panel_cursor.is_some() {
        composer::draw::paint_cursor(frame, panel_cursor, theme);
    } else if view.panels.is_empty() || view.panels.wants_input(view).is_some() {
        composer::draw::paint_cursor(frame, cursor, theme);
    }
    Hit {
        chat,
        cards,
        activity: status_hit(chat, &cf),
        composer: comp,
    }
}

/// Where the live status row and the reasoning pane's header landed, as
/// one rectangle in the chat's area: a click there toggles the pane.
fn status_hit(area: Rect, cf: &layout::ChatFrame) -> Rect {
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

/// Where the conversation's column sits on the rack screen at `width ×
/// height`: its left edge and its width, as `draw_solo` lays it out. For
/// tests of what is placed within it.
#[cfg(test)]
pub fn chat_column(view: &View, theme: Theme, width: u16, height: u16) -> (u16, u16) {
    let tier = crate::rail::tier(width);
    let narrow = tier == crate::rail::Tier::Narrow;
    let gap = u16::from(height >= 24);
    let rule_h = u16::from(!narrow);
    let body_h = height.saturating_sub(1 + rule_h + gap + 1);
    let (rack_w, inst_w) = side_columns(view, theme, width, body_h);
    let main_w = width.saturating_sub(rack_w + inst_w);
    let col_w = main_w.saturating_sub(4).min(LEDGER_COLUMN);
    let col_x = rack_w + (main_w.saturating_sub(col_w + 1)) / 2;
    (col_x, col_w)
}

/// Widest the ledger's reading column gets: the timeline gutter plus about
/// a hundred columns of text. Wider lines are harder to read, not better.
pub const LEDGER_COLUMN: u16 = 112;

/// The ledger (`[ui] layout = "ledger"`, the default): the solo screen,
/// or the workbench when it is open, which keeps the ledger's own frame: a
/// strip of views across the top and one bar at the foot.
fn draw_ledger(frame: &mut Frame, view: &View, theme: Theme) -> Hit {
    let full = frame.area();
    if view.workbench.is_none() {
        return draw_solo(frame, view, theme);
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
            draw_scrollbar(frame, gutter, &cf, view.scroll.follow, theme, true);
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
    let panel_cursor = panel::draw(frame, full, body, None, view, theme);
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

/// The side columns a screen has: the rack's width and the instruments'.
/// Both give way to the conversation: the rack first, as the screen
/// narrows or shortens, then the instruments. `^b` hides them.
pub fn side_columns(view: &View, theme: Theme, width: u16, body_h: u16) -> (u16, u16) {
    use crate::rail::Tier;
    if !view.panel_visible {
        return (0, 0);
    }
    match crate::rail::tier(width) {
        Tier::Wide if crate::rail::fits(view, theme, body_h) => {
            (crate::rail::RACK_W, crate::instruments::WIDE_W)
        }
        Tier::Wide => (0, crate::instruments::WIDE_W),
        Tier::Mid => (0, crate::instruments::MID_W),
        Tier::Narrow => (0, 0),
    }
}

/// The solo screen (`docs/hat-rack-design.md`): a bar naming the four
/// hats; the hat rack, the conversation and the instruments side by side;
/// the prompt under a rule in the hat's color; and the keys at the foot.
/// The hat on sets the one accent, and a fedora in that color sits behind
/// the conversation.
fn draw_solo(frame: &mut Frame, view: &View, theme: Theme) -> Hit {
    let full = frame.area();
    let tier = crate::rail::tier(full.width);
    let narrow = tier == crate::rail::Tier::Narrow;
    // A blank row between the prompt and the keys, where there is height
    // for it.
    let gap = u16::from(full.height >= 24);
    let rule_h = u16::from(!narrow);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(rule_h),
            Constraint::Min(4),
            Constraint::Length(gap),
            Constraint::Length(1),
        ])
        .split(full);
    let (top, rule, body, foot) = (rows[0], rows[1], rows[2], rows[4]);
    // The side columns run the body's height, the prompt included: the
    // prompt is the conversation's, the width of its column.
    let (rack_w, inst_w) = side_columns(view, theme, full.width, body.height);
    draw_top_bar(frame, top, view, theme, rack_w == 0);
    if rule_h > 0 {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(usize::from(rule.width)),
                Style::default().fg(theme.rule).bg(theme.bg),
            ))),
            rule,
        );
    }
    let main = Rect {
        x: body.x + rack_w,
        width: body.width.saturating_sub(rack_w + inst_w),
        ..body
    };
    // A popout wider than the conversation's column takes the whole body,
    // and the side columns are left out from under it: half a column
    // showing past a panel's edge reads as a broken screen. One that fits
    // (a plan to approve, a permission) leaves them in sight.
    let panel_owns_body = view
        .panels
        .stack
        .iter()
        .any(|p| !p.docked() && p.size(view).0 > main.width);
    let (rack_w, inst_w) = if panel_owns_body {
        (0, 0)
    } else {
        (rack_w, inst_w)
    };
    if rack_w > 0 {
        crate::rail::draw(
            frame,
            Rect {
                width: rack_w,
                ..body
            },
            view,
            theme,
        );
    }
    if inst_w > 0 {
        crate::instruments::draw(
            frame,
            Rect {
                x: body.x + body.width - inst_w,
                width: inst_w,
                ..body
            },
            view,
            theme,
            tier != crate::rail::Tier::Wide,
        );
    }
    let comp_h =
        composer::draw::solo_height(view, main.width).min(main.height.saturating_sub(6).max(2));
    let col_w = main.width.saturating_sub(4).min(LEDGER_COLUMN);
    let col_x = main.x + (main.width.saturating_sub(col_w + 1)) / 2;
    let column = |r: Rect| Rect {
        x: col_x,
        width: col_w,
        ..r
    };
    let below = main.height.saturating_sub(comp_h);
    // One spinner: the status row in the conversation. The strip's rows
    // are gone from this screen; the reasoning pane opens under the row.
    let activity_h: u16 = 0;
    // A question takes rows of its own between the chat and the strip,
    // so the reply's last lines stay in sight above it.
    let prompt_h = panel::prompt_height(view, below.saturating_sub(activity_h));
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(4),
            Constraint::Length(prompt_h),
            Constraint::Length(activity_h),
            Constraint::Length(comp_h),
        ])
        .split(main);
    let comp = parts[3];
    let slot = (prompt_h > 0).then(|| column(parts[1]));
    let chat = column(parts[0]);
    let cf = draw_chat(frame, chat, view, theme);
    if view.ui.watermark {
        crate::watermark::draw(frame, parts[0], view.mode, theme);
    }
    if view.panels.is_empty() {
        // The gutter runs beside the chat, the rows the thumb measures.
        let gutter = Rect {
            x: col_x + col_w,
            width: 1,
            ..parts[0]
        };
        // The thumb alone: a track beside the instruments' hairline is
        // two lines where one separates.
        draw_scrollbar(frame, gutter, &cf, view.scroll.follow, theme, false);
    }
    let cursor = composer::draw::draw_solo(frame, comp, view, theme);
    if narrow {
        draw_status_line(frame, foot, view, theme);
    } else {
        draw_keys(frame, foot, view, theme);
    }
    if view.panels.is_empty() {
        palette::draw(frame, chat, comp.y, view, theme);
    }
    let panel_body = if panel_owns_body { body } else { main };
    let panel_cursor = panel::draw(frame, full, panel_body, slot, view, theme);
    if panel_cursor.is_some() {
        composer::draw::paint_cursor(frame, panel_cursor, theme);
    } else if view.panels.is_empty() || view.panels.wants_input(view).is_some() {
        composer::draw::paint_cursor(frame, cursor, theme);
    }
    Hit {
        chat,
        cards: Vec::new(),
        activity: status_hit(chat, &cf),
        composer: comp,
    }
}

/// The bar across the top (`docs/hat-rack-design.md` §4): the name, then
/// the four hats in the order `Tab` goes round them, then where this is.
/// It says which hats have been worn, not an order to wear them in: no
/// arrows, steps or ticks. `counts` adds each worn hat's turns, for when
/// the rack isn't on screen to say them.
fn draw_top_bar(frame: &mut Frame, area: Rect, view: &View, theme: Theme, counts: bool) {
    let w = usize::from(area.width);
    let bg = theme.panel_bg;
    let dim = Style::default().fg(theme.dim).bg(bg);
    let body = Style::default().fg(theme.fg).bg(bg);
    let gap = if w >= 100 { "   " } else { "  " };
    let mut left = vec![
        Span::styled(" RYTER", body.add_modifier(Modifier::BOLD)),
        Span::styled(" │ ", Style::default().fg(theme.faint).bg(bg)),
    ];
    // The two rows of the rack, a dot between them. Narrower, the row the
    // user is not in folds to its name and how many of its hats were worn;
    // narrower still, only the row the user is in.
    let row_of = |hats: &[Role]| hats.contains(&view.mode);
    let fold = |hats: &[Role], name: &str| -> Vec<Span<'static>> {
        let worn = hats.iter().filter(|h| view.rack.worn(**h)).count();
        let mut v = vec![Span::styled(name.to_string(), dim)];
        if worn > 0 {
            v.push(Span::styled(format!(" ·{worn}"), dim));
        }
        v
    };
    let primary: &[Role] = &crate::rail::PRIMARY;
    let specialists: &[Role] = &crate::rail::SPECIALISTS;
    let in_primary = row_of(primary);
    let (shown, folded): (Vec<Role>, Option<Vec<Span<'static>>>) =
        if w >= usize::from(crate::rail::BOTH_MIN) {
            (crate::rail::HATS.to_vec(), None)
        } else if w >= usize::from(crate::rail::INSTRUMENTS_MIN) {
            if in_primary {
                (
                    primary.to_vec(),
                    Some(fold(specialists, crate::rail::SEPARATOR_LABEL)),
                )
            } else {
                (specialists.to_vec(), Some(fold(primary, "plan · build")))
            }
        } else if in_primary {
            (primary.to_vec(), None)
        } else {
            (specialists.to_vec(), None)
        };
    let dot = Span::styled(
        format!("{gap}·{gap}"),
        Style::default().fg(theme.faint).bg(bg),
    );
    for (i, hat) in shown.iter().copied().enumerate() {
        if i > 0 && shown[i - 1].row() != hat.row() {
            left.push(dot.clone());
        } else if i > 0 {
            left.push(Span::styled(gap, dim));
        }
        let name = crate::rail::hat_name(hat);
        // The hat that is on spins while its model works.
        let mark = if view.mode == hat && view.busy {
            crate::activity::SPINNER[view.activity.frame % crate::activity::SPINNER.len()]
        } else {
            crate::rail::hat_mark(view, hat)
        };
        let turns = view.rack.of(hat).turns;
        let count = if counts && turns > 0 {
            format!(" {turns}")
        } else {
            String::new()
        };
        let color = theme.mode(hat);
        if view.mode == hat {
            left.push(Span::styled(
                format!(" {mark} {name}{count} "),
                theme.chip(color),
            ));
        } else if view.rack.worn(hat) {
            left.push(Span::styled(
                format!("{mark} "),
                Style::default().fg(color).bg(bg),
            ));
            left.push(Span::styled(name, body));
            left.push(Span::styled(count, dim));
        } else {
            left.push(Span::styled(format!("{mark} {name}"), dim));
        }
    }
    if let Some(other) = folded {
        if in_primary {
            left.push(dot.clone());
            left.extend(other);
        } else {
            // The primary row comes first on the bar, folded or not.
            let mut front = other;
            front.push(dot.clone());
            let tail: Vec<Span<'static>> = left.drain(2..).collect();
            left.extend(front);
            left.extend(tail);
        }
    }
    let left_w: usize = left.iter().map(|s| wrap::width(&s.content)).sum();
    // Where this is: the folder and the branch, less of it as the screen
    // narrows.
    let folder = if w >= usize::from(crate::rail::BOTH_MIN) {
        view.cwd.clone()
    } else if w >= usize::from(crate::rail::INSTRUMENTS_MIN) {
        std::path::Path::new(&view.cwd)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| view.cwd.clone())
    } else {
        String::new()
    };
    let place = match (&view.git_branch, folder.is_empty()) {
        (Some(b), false) => format!("{folder} · {b}"),
        (Some(b), true) => b.clone(),
        (None, _) => folder,
    };
    // A long path keeps its end, and leaves the bar to the hats.
    let place = crate::rail::tail(&place, w.saturating_sub(left_w + 3).min(48));
    let place_w = wrap::width(&place) + 1;
    let mut spans = left;
    let middle = w.saturating_sub(left_w + place_w);
    // The session's title in the room between, where there is some.
    let title = view.session_title.trim();
    if w >= 100 && !title.is_empty() && middle > 16 {
        let t = wrap::truncate(title, middle - 6);
        let tw = wrap::width(&t);
        let lpad = (middle - tw) / 2;
        spans.push(Span::styled(" ".repeat(lpad), dim));
        spans.push(Span::styled(t, dim));
        spans.push(Span::styled(" ".repeat(middle - tw - lpad), dim));
    } else {
        spans.push(Span::styled(" ".repeat(middle), dim));
    }
    spans.push(Span::styled(place, dim));
    spans.push(Span::styled(" ", dim));
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(bg)),
        area,
    );
}

/// The keys that matter now, as ranked hints: an open panel's own, or the
/// screen's.
fn hints_or_legend(view: &View) -> Vec<(String, String, Hint)> {
    match view.panels.top() {
        Some(p) => legend_keys(&p.legend(view)),
        None => hints_ranked(view),
    }
}

/// Those, with the spend drawer's first while there is nothing else to
/// say.
fn solo_keys(view: &View) -> Vec<(String, String, Hint)> {
    let mut keys = hints_or_legend(view);
    if view.panels.top().is_none() && view.palette.is_none() && view.composer.is_empty() {
        keys.insert(0, ("$".into(), "spend".into(), Hint::Useful));
    }
    keys
}

/// Drop the least needed keys until the rest fit in `width`, a key's
/// separator `sep` columns wide.
fn fit_keys(
    mut keys: Vec<(String, String, Hint)>,
    width: usize,
    sep: usize,
) -> Vec<(String, String, Hint)> {
    let used = |keys: &[(String, String, Hint)]| -> usize {
        keys.iter()
            .map(|(k, l, _)| wrap::width(k) + 1 + wrap::width(l) + sep)
            .sum::<usize>()
            .saturating_sub(sep)
    };
    // Cancel, quit and an answer to a prompt are never dropped.
    while used(&keys) > width {
        let worst = keys
            .iter()
            .enumerate()
            .filter(|(_, (_, _, rank))| *rank != Hint::Essential)
            .max_by_key(|(i, (_, _, rank))| (*rank, *i))
            .map(|(i, _)| i);
        match worst {
            Some(i) => {
                keys.remove(i);
            }
            None => break,
        }
    }
    keys
}

fn key_spans(
    keys: &[(String, String, Hint)],
    theme: Theme,
    bg: ratatui::style::Color,
    sep: usize,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, (k, l, _)) in keys.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" ".repeat(sep), Style::default().bg(bg)));
        }
        spans.push(Span::styled(
            k.clone(),
            Style::default()
                .fg(theme.fg)
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" {l}"),
            Style::default().fg(theme.dim).bg(bg),
        ));
    }
    spans
}

/// Key spans with the first `warm` keys in the warn color: a question's
/// own keys, then the rest as usual.
fn key_spans_colored(
    keys: &[(String, String, Hint)],
    warm: usize,
    theme: Theme,
    bg: ratatui::style::Color,
    sep: usize,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, (k, l, _)) in keys.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" ".repeat(sep), Style::default().bg(bg)));
        }
        let (kc, lc) = if i < warm {
            (theme.warn, theme.warn)
        } else {
            (theme.fg, theme.dim)
        };
        spans.push(Span::styled(
            k.clone(),
            Style::default().fg(kc).bg(bg).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" {l}"),
            Style::default().fg(lc).bg(bg),
        ));
    }
    spans
}

/// The solo screen's foot: the keys, from the left.
/// What `esc` does on the open card: the same as its `n` key, which a
/// permission card calls `deny` and the run-file card `reject`. A card
/// without an `n` key denies.
fn esc_does(keys: &[(String, String, Hint)]) -> String {
    keys.iter()
        .find(|(k, _, _)| k == "n")
        .map_or_else(|| "deny".to_string(), |(_, label, _)| label.clone())
}

fn draw_keys(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    let bg = theme.panel_bg;
    // A question's keys are in the warn color, where the eye lands when
    // the card above has not been seen; the ways out stay dim after them.
    let asking = view
        .panels
        .top()
        .is_some_and(|p| p.docked() && p.modal().is_some());
    let mut all = solo_keys(view);
    let card_keys = if asking { all.len() } else { 0 };
    if asking {
        if !all.iter().any(|(k, _, _)| k == "esc") {
            let out = esc_does(&all);
            all.push(("esc".into(), out, Hint::Useful));
        }
        if view.busy {
            all.push(("^c".into(), "stop the turn".into(), Hint::Essential));
        }
    }
    let keys = fit_keys(all, usize::from(area.width).saturating_sub(2), 4);
    let mut spans = vec![Span::styled(" ", Style::default().bg(bg))];
    if asking {
        spans.extend(key_spans_colored(&keys, card_keys, theme, bg, 4));
    } else {
        spans.extend(key_spans(&keys, theme, bg, 4));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(bg)),
        area,
    );
}

/// The foot of a screen too narrow for the instruments: the model, how
/// full its context is, what the session has cost and the budget, then as
/// many keys as fit. The project's cost is in the `^b` panel.
fn draw_status_line(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    let w = usize::from(area.width);
    let bg = theme.panel_bg;
    let on = |fg| Style::default().fg(fg).bg(bg);
    let frac = view.ctx_frac();
    const CELLS: usize = 8;
    let filled = ((frac * CELLS as f64).round() as usize).min(CELLS);
    let (session, color) = crate::instruments::session_spend(view, theme);
    let model = wrap::truncate(
        crate::chat::short_model(crate::rail::hat_model(view, view.mode)),
        16,
    );
    let mut left = vec![
        Span::styled(format!(" {model}"), on(theme.mode(view.mode))),
        Span::styled("  ctx ", on(theme.dim)),
        Span::styled(
            "━".repeat(filled),
            on(crate::instruments::gauge_color(view, theme)),
        ),
        Span::styled("─".repeat(CELLS - filled), on(theme.rule)),
        Span::styled(format!(" {}%", (frac * 100.0).round() as u32), on(theme.fg)),
        Span::styled(" │ ", on(theme.faint)),
        Span::styled(session, on(color)),
        Span::styled(
            format!(" · budget {}", crate::instruments::budget(view)),
            on(theme.dim),
        ),
    ];
    let left_w: usize = left.iter().map(|s| wrap::width(&s.content)).sum();
    // The keys take what is left; cancel and quit are never dropped, so on
    // a very narrow screen the facts give way from the end.
    let keys = fit_keys(hints_or_legend(view), w.saturating_sub(left_w + 4), 2);
    let ks = key_spans(&keys, theme, bg, 2);
    let ks_w: usize = ks.iter().map(|s| wrap::width(&s.content)).sum();
    while left.len() > 1
        && left.iter().map(|s| wrap::width(&s.content)).sum::<usize>() + ks_w + 3 > w
    {
        left.pop();
    }
    let left_w: usize = left.iter().map(|s| wrap::width(&s.content)).sum();
    let mut spans = left;
    spans.push(Span::styled(
        " ".repeat(w.saturating_sub(left_w + ks_w + 1)),
        on(theme.dim),
    ));
    spans.extend(ks);
    spans.push(Span::styled(" ", on(theme.dim)));
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(bg)),
        area,
    );
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
        "chat",
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

/// Gutter scrollbar (`R-BAR-01..04`). `track` draws the line the thumb
/// runs on as well as the thumb.
fn draw_scrollbar(
    frame: &mut Frame,
    area: Rect,
    cf: &layout::ChatFrame,
    follow: bool,
    theme: Theme,
    track: bool,
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
        } else if track {
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
                // The only way to the hat rack on a screen that has folded
                // it away.
                "^b" if l == "hat rack" => Hint::Essential,
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

/// What `Shift+Tab` leads to, as the hints say it: the specialists from
/// the primary row, `plan · build` from the specialists.
pub fn other_row_label(view: &View) -> String {
    match view.mode.row() {
        ryter_core::role::Row::Primary => crate::rail::SEPARATOR_LABEL.to_string(),
        ryter_core::role::Row::Specialist => "plan · build".to_string(),
    }
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
    // The two keys that aren't discoverable any other way: the next hat in
    // this row, and the other row.
    v.push(("tab", view.mode.next_in_row().as_str().to_string()));
    v.push(("⇧tab", other_row_label(view)));
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
    // `^b` shows and hides the info panel on the classic screen and the
    // side columns on the solo one; on a screen too narrow to hold both
    // columns it opens them as a panel.
    if view.ui.classic() {
        v.push(("^b", "panel".into()));
    } else if view.screen.get().0 >= crate::rail::BOTH_MIN {
        v.push(("^b", "panels".into()));
    } else {
        v.push(("^b", "hat rack".into()));
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
    // A question's keys are in the warn color here as on the rack screen,
    // where the eye lands when the card above has not been seen; the ways
    // out stay dim after them.
    let asking = view
        .panels
        .top()
        .is_some_and(|p| p.docked() && p.modal().is_some());
    let mut card_keys = 0;
    if let Some(p) = view.panels.top() {
        items = legend_keys(&p.legend(view));
        if asking {
            card_keys = items.len();
            if !items.iter().any(|(k, _, _)| k == "esc") {
                let out = esc_does(&items);
                items.push(("esc".into(), out, Hint::Useful));
            }
            if view.busy {
                items.push(("^c".into(), "stop the turn".into(), Hint::Essential));
            }
        }
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
    if asking {
        spans.extend(key_spans_colored(&items, card_keys, theme, theme.bg, 4));
    } else {
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
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(theme.body()), area);
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
