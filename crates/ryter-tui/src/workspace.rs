//! The workspace screen: a hat rack, three views, and the instruments.
//!
//! This is the default layout (`[ui] layout = "workspace"`). RYTER sits on
//! the top bar, with Conversation, Hats, and Sessions beside it. The rack
//! is the sidebar. The conversation, the hats, or the sessions fill the
//! middle, and activity, context, spend (session + project glance), files,
//! and the plan sit on the right. It is the one screen: the ledger and
//! classic layouts went on 2026-10-19 (`DECISIONS.md`).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ryter_core::Role;

use crate::action::Action;
use crate::chat::wrap;
use crate::glyphs::{self, Mark};
use crate::select::WorkspaceClick;
use crate::sidebar::{self, HATS};
use crate::theme::Theme;
use crate::view::{Focus, Page, SavedSession, View};
use crate::{composer, draw, palette, panel, ring};

/// Paint one frame of the workspace.
pub fn draw(frame: &mut Frame, view: &View, theme: Theme) -> draw::Hit {
    let full = frame.area();
    let mut clicks = Vec::new();
    // Quiet chrome (E): no hairline under the top bar — body starts next.
    // A tall screen gives the bar a row above and below its line.
    let top_h = if full.height >= 30 { 3 } else { 1 };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(top_h),
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .split(full);
    let (top, body, hint) = (rows[0], rows[1], rows[2]);
    fill(frame, full, theme.bg);
    fill(frame, top, side_bg(theme));
    draw_top(
        frame,
        Rect {
            y: top.y + top.height / 2,
            height: 1,
            ..top
        },
        view,
        theme,
        &mut clicks,
    );
    let (side_w, insp_w) = columns(full.width, view.panel_visible);
    let side = Rect {
        width: side_w,
        ..body
    };
    let insp = Rect {
        x: body.x + body.width.saturating_sub(insp_w),
        width: insp_w,
        ..body
    };
    let stage = Rect {
        x: body.x + side_w,
        width: body.width.saturating_sub(side_w + insp_w),
        ..body
    };
    // Leave a consistent breathing row above every pane when height allows.
    let pad = u16::from(body.height >= 28);
    let inset = |r: Rect| Rect {
        y: r.y + pad,
        height: r.height.saturating_sub(pad),
        ..r
    };
    if side_w > 0 {
        fill(frame, side, side_bg(theme));
        draw_sidebar(frame, inset(side), view, theme, &mut clicks);
    }
    let stage = inset(stage);
    let (chat, composer_rect, slot, activity) = draw_stage(frame, stage, view, theme);
    if insp_w > 0 {
        fill(frame, insp, side_bg(theme));
        draw_inspector(frame, inset(insp), view, theme);
    }
    draw_hint(frame, hint, view, theme);
    // Dialogs and docked cards take the prompt's columns.
    let ground = margin(stage);
    if view.panels.is_empty() {
        palette::draw(frame, ground, composer_rect.y, view, theme);
    }
    let cursor = panel::draw(frame, full, panel::Ground::over(ground, slot), view, theme);
    if cursor.is_some() {
        composer::draw::paint_cursor(frame, cursor, theme);
    } else if view.page == Page::Conversation
        && (view.panels.is_empty() || view.panels.wants_input(view).is_some())
    {
        // The composer paints its own cursor inside `draw_stage` only when
        // no panel claimed it. Re-read it from the composer rect.
    }
    draw::Hit {
        chat,
        chat_text: Rect::default(),
        covers: Vec::new(),
        activity,
        composer: composer_rect,
        clicks,
        welcome: None,
        chip: None,
        hat_row: None,
    }
}

/// `F2`, `F6`, `F7`, and `Alt+1`–`Alt+3`. `None` when the key is not one of
/// those, or this is not the workspace. A palette swallows them.
pub fn chrome_key(view: &mut View, key: KeyEvent) -> Option<Action> {
    if false {
        return None;
    }
    let alt =
        key.modifiers.contains(KeyModifiers::ALT) && !key.modifiers.contains(KeyModifiers::CONTROL);
    let plain = key.modifiers.is_empty();
    let chrome = match key.code {
        KeyCode::F(2 | 6 | 7) if plain => true,
        KeyCode::Char('1' | '2' | '3') if alt => true,
        _ => false,
    };
    if !chrome {
        return None;
    }
    if view.palette.is_some() {
        return Some(Action::None);
    }
    match key.code {
        KeyCode::F(2) => {
            view.panel_visible = !view.panel_visible;
            if !view.panel_visible && view.focus == Focus::Inspector {
                view.focus = Focus::Stage;
            }
        }
        KeyCode::F(6) => move_focus(view, -1),
        KeyCode::F(7) => move_focus(view, 1),
        KeyCode::Char(c @ '1'..='3') => {
            view.page = match c {
                '1' => Page::Conversation,
                '2' => Page::Hats,
                _ => Page::Sessions,
            };
            view.focus = Focus::Stage;
            if view.page == Page::Sessions {
                reload_sessions(view);
            }
        }
        _ => {}
    }
    Some(Action::None)
}

/// Arrows and Enter while the sidebar, the hats page, or the sessions list
/// has focus. `None` when the composer should keep the key.
pub fn nav_key(view: &mut View, key: KeyEvent) -> Option<Action> {
    if view.palette.is_some() {
        return None;
    }
    if !key.modifiers.is_empty() {
        return None;
    }
    let dir = match key.code {
        KeyCode::Up => -1,
        KeyCode::Down => 1,
        KeyCode::Enter => 0,
        _ => return None,
    };
    match view.focus {
        Focus::Sidebar => Some(sidebar_key(view, dir)),
        Focus::Stage if view.page == Page::Hats && dir != 0 => {
            view.seat = step_hat(view.seat, dir);
            Some(Action::None)
        }
        Focus::Stage if view.page == Page::Sessions => Some(sessions_key(view, dir)),
        _ => None,
    }
}

/// A click on the rack, a view tab, or the theme button.
pub fn click(view: &mut View, kind: WorkspaceClick) -> Action {
    match kind {
        WorkspaceClick::Page(n) => {
            view.page = match n {
                2 => Page::Hats,
                3 => Page::Sessions,
                _ => Page::Conversation,
            };
            view.focus = Focus::Stage;
            if view.page == Page::Sessions {
                reload_sessions(view);
            }
            Action::None
        }
        WorkspaceClick::Wear(role) => {
            view.page = Page::Hats;
            view.focus = Focus::Stage;
            Action::SetMode(role)
        }
    }
}

/// Read this project's saved sessions into the view.
pub fn reload_sessions(view: &mut View) {
    let (Some(home), Some(cwd)) = (view.ryter_home.clone(), view.workspace.clone()) else {
        return;
    };
    let Ok(list) = ryter_core::Session::list(&home, &cwd) else {
        return;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    view.sessions = list
        .into_iter()
        .map(|s| SavedSession {
            id: s.meta.id.to_string(),
            title: if s.meta.title.trim().is_empty() {
                s.preview.clone()
            } else {
                s.meta.title.clone()
            },
            sub: s.short_id(),
            hat: s
                .meta
                .mode
                .map(|m| m.hat().as_str().to_string())
                .unwrap_or_else(|| "build".into()),
            messages: s.messages,
            spend: session_spend(s.meta.spend_usd_total, s.messages),
            updated: panel::sessions::relative(now, s.updated_millis()),
        })
        .collect();
    if !view.session_id.is_empty() && view.sessions.iter().all(|s| s.id != view.session_id) {
        view.sessions.insert(0, current_saved(view));
    }
    view.sessions_count = view
        .sessions
        .iter()
        .filter(|s| s.id != view.session_id)
        .count();
    if view.session_at >= view.sessions.len() {
        view.session_at = 0;
    }
}

fn sidebar_key(view: &mut View, dir: i32) -> Action {
    let n = HATS.len();
    if dir == 0 {
        let hat = HATS[view.side_at.min(n - 1)];
        view.page = Page::Hats;
        view.focus = Focus::Stage;
        return Action::SetMode(hat);
    }
    let at = view.side_at as i32 + dir;
    view.side_at = at.rem_euclid(n as i32) as usize;
    Action::None
}

fn sessions_key(view: &mut View, dir: i32) -> Action {
    let rows = shown_sessions(view);
    if rows.is_empty() {
        return Action::None;
    }
    if dir == 0 {
        let row = &rows[view.session_at.min(rows.len() - 1)];
        if row.id.is_empty() || row.id == view.session_id {
            return Action::None;
        }
        return Action::Resume(row.id.clone());
    }
    let at = view.session_at as i32 + dir;
    view.session_at = at.rem_euclid(rows.len() as i32) as usize;
    Action::None
}

fn move_focus(view: &mut View, dir: i32) {
    let (w, _) = view.screen.get();
    let (side, insp) = columns(w, view.panel_visible);
    let mut panes = Vec::new();
    if side > 0 {
        panes.push(Focus::Sidebar);
    }
    panes.push(Focus::Stage);
    if insp > 0 {
        panes.push(Focus::Inspector);
    }
    let i = panes.iter().position(|p| *p == view.focus).unwrap_or(0);
    let n = panes.len() as i32;
    view.focus = panes[(i as i32 + dir).rem_euclid(n) as usize];
    if view.focus == Focus::Sidebar {
        view.side_at = hat_index(view.mode);
    }
}

fn step_hat(seat: Role, dir: i32) -> Role {
    let i = hat_index(seat) as i32;
    HATS[(i + dir).rem_euclid(HATS.len() as i32) as usize]
}

fn hat_index(role: Role) -> usize {
    HATS.iter().position(|h| *h == role.hat()).unwrap_or(0)
}

pub(crate) fn columns(width: u16, instruments: bool) -> (u16, u16) {
    // Keep at least 72 columns for the conversation. Fold the rack first;
    // the Hats view still shows every seat, model, and permission.
    if width >= 132 {
        return (28, if instruments { 30 } else { 0 });
    }
    if width >= 100 {
        return if instruments { (0, 28) } else { (26, 0) };
    }
    (0, 0)
}

fn draw_top(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    theme: Theme,
    clicks: &mut Vec<(WorkspaceClick, Rect)>,
) {
    let bg = side_bg(theme);
    fill(frame, area, bg);
    if area.height == 0 {
        return;
    }
    // Brand mark: three hat-color dots + bold RYTER. Quiet chrome still drops
    // the top-right avatar/theme cluster and the hairline under the bar.
    let mark_w = 1 + 3 + 1 + 5 + 2;
    put(
        frame,
        area,
        0,
        Line::from(vec![
            Span::styled(" ", sty(theme.fg, bg)),
            Span::styled("●", sty(theme.plan, bg)),
            Span::styled("●", sty(theme.build, bg)),
            Span::styled("●", sty(theme.audit, bg)),
            Span::styled(" RYTER", sty(theme.fg, bg).add_modifier(Modifier::BOLD)),
        ]),
    );
    draw_top_controls(frame, area, view, theme, clicks, mark_w);
}

/// View tabs on the baseline of the top bar. The brand occupies the first
/// `skip` columns. Theme switching stays on `/theme` — no top-right cluster.
fn draw_top_controls(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    theme: Theme,
    clicks: &mut Vec<(WorkspaceClick, Rect)>,
    skip: u16,
) {
    let tabs = Rect {
        x: area.x + skip.min(area.width),
        width: area.width.saturating_sub(skip),
        ..area
    };
    draw_tabs(frame, tabs, view, theme, clicks);
}

fn draw_tabs(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    theme: Theme,
    clicks: &mut Vec<(WorkspaceClick, Rect)>,
) {
    if area.width < 8 {
        return;
    }
    let bg = side_bg(theme);
    let pages = [
        (Page::Conversation, "Conversation", "1", 1u8),
        (Page::Hats, "Hats", "2", 2),
        (Page::Sessions, "Sessions", "3", 3),
    ];
    let compact = area.width < 52;
    let mut x = area.x;
    let end = area.x + area.width;
    for (page, name, key, n) in pages {
        let mut label = if compact && page == Page::Conversation {
            "Chat"
        } else {
            name
        }
        .to_string();
        if !compact && page == Page::Sessions && view.sessions_count > 0 {
            label.push(' ');
            label.push_str(&view.sessions_count.to_string());
        }
        let active = view.page == page;
        let (fg, fill) = if active {
            (theme.fg, theme.selection_bg)
        } else {
            (theme.dim, bg)
        };
        let name_spans = if active {
            glyphs::pill(&label, fg, fill, bg, view.ui.nerd_font)
        } else {
            vec![Span::styled(format!(" {label} "), sty(fg, fill))]
        };
        let key_span = Span::styled(key, sty(theme.dim, bg));
        let name_w: u16 = name_spans
            .iter()
            .map(|s| wrap::width(&s.content) as u16)
            .sum();
        let key_w = wrap::width(&key_span.content) as u16;
        let need = name_w + 1 + key_w + 1;
        if x + need > end {
            break;
        }
        let mut spans = name_spans;
        spans.push(Span::styled(" ", sty(theme.dim, bg)));
        spans.push(key_span);
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect {
                x,
                width: name_w + 1 + key_w,
                ..area
            },
        );
        clicks.push((
            WorkspaceClick::Page(n),
            Rect {
                x,
                width: name_w + 1 + key_w,
                ..area
            },
        ));
        x += need;
    }
}

fn draw_sidebar(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    theme: Theme,
    clicks: &mut Vec<(WorkspaceClick, Rect)>,
) {
    let bg = side_bg(theme);
    fill(frame, area, bg);
    let accent = theme.mode(view.mode);
    if area.width > 0 {
        let bar = Rect {
            x: area.x + area.width - 1,
            width: 1,
            ..area
        };
        vertical_rule(
            frame,
            bar,
            if view.focus == Focus::Sidebar {
                accent
            } else {
                theme.rule
            },
            bg,
        );
    }
    let inner = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(3),
        ..area
    };
    let w = usize::from(inner.width);
    let tall = inner.height >= 28;
    let mut y = 0u16;
    let folder = folder_name(view);
    let folder_icon = glyphs::mark(Mark::Folder, view.ui.nerd_font);
    let folder_line = if folder_icon.is_empty() {
        format!(" {}", wrap::truncate(&folder, w.saturating_sub(2)))
    } else {
        format!(
            " {folder_icon} {}",
            wrap::truncate(&folder, w.saturating_sub(wrap::width(folder_icon) + 3))
        )
    };
    put(
        frame,
        inner,
        y,
        Line::from(Span::styled(
            folder_line,
            sty(theme.fg, bg).add_modifier(Modifier::BOLD),
        )),
    );
    y += 1;
    let branch = view.git_branch.as_deref().unwrap_or("");
    if !branch.is_empty() && y < inner.height {
        let branch_icon = glyphs::mark(Mark::Branch, view.ui.nerd_font);
        put(
            frame,
            inner,
            y,
            Line::from(Span::styled(
                format!(
                    " {branch_icon} {}",
                    wrap::truncate(branch, w.saturating_sub(wrap::width(branch_icon) + 3))
                ),
                sty(theme.dim, bg),
            )),
        );
        y += 1;
    }
    if y < inner.height {
        y += 1;
        put(
            frame,
            inner,
            y,
            Line::from(Span::styled(" HATS", sty(theme.dim, bg))),
        );
        y += 1;
    }
    let spec_at = HATS.iter().position(|h| *h == Role::Audit);
    for (i, hat) in HATS.iter().enumerate() {
        if spec_at == Some(i) && y < inner.height {
            put(
                frame,
                inner,
                y,
                Line::from(Span::styled(" Specialists", sty(theme.dim, bg))),
            );
            y += 1;
        }
        if y >= inner.height {
            break;
        }
        let rows = if tall { 3 } else { 1 };
        if y + rows > inner.height {
            break;
        }
        draw_hat(frame, inner, y, *hat, view, theme, tall);
        clicks.push((
            WorkspaceClick::Wear(*hat),
            Rect {
                y: inner.y + y,
                height: rows,
                ..inner
            },
        ));
        y += rows + u16::from(tall);
    }
    if tall {
        draw_side_foot(frame, inner, y, view, theme);
    }
}

fn draw_hat(
    frame: &mut Frame,
    area: Rect,
    y: u16,
    hat: Role,
    view: &View,
    theme: Theme,
    tall: bool,
) {
    let bg = if hat == view.mode {
        on_bg(theme, side_bg(theme))
    } else {
        side_bg(theme)
    };
    let color = theme.mode(hat);
    let w = usize::from(area.width);
    let wearing = hat == view.mode;
    let cursor = view.focus == Focus::Sidebar && view.side_at == hat_index(hat);
    let row = Rect {
        y: area.y + y,
        height: if tall { 3 } else { 1 },
        ..area
    };
    fill(frame, row, bg);
    if wearing {
        view.marks.borrow_mut().hat_row = Some(row);
    }
    let pip = if wearing {
        "┃"
    } else {
        glyphs::hat_icon(hat, view.ui.nerd_font)
    };
    let name = glyphs::hat_label(hat);
    // The cursor occupies the pad column, so a focused row does not shift.
    let mark = if cursor { "▸" } else { " " };
    let mut name_spans = vec![
        Span::styled(format!("{mark}{pip} "), sty(color, bg)),
        Span::styled(
            name,
            sty(if wearing || cursor { color } else { theme.fg }, bg).add_modifier(Modifier::BOLD),
        ),
    ];
    if hat == Role::Audit
        && view
            .last_review
            .as_ref()
            .is_some_and(|(_, _, v)| *v == Some(false))
    {
        name_spans.push(Span::styled(
            format!(" {}", glyphs::mark(Mark::Failed, view.ui.nerd_font)),
            sty(theme.error, bg),
        ));
    }
    let spend = sidebar::hat_spend(view, hat);
    let turns = view.rack.of(hat).turns;
    if !tall {
        put(
            frame,
            area,
            y,
            sidebar::ends(
                name_spans,
                vec![Span::styled(spend, sty(theme.dim, bg))],
                w,
                bg,
            ),
        );
        return;
    }
    let turn_s = if turns == 1 {
        "1 turn".to_string()
    } else if turns == 0 {
        String::new()
    } else {
        format!("{turns} turns")
    };
    put(
        frame,
        area,
        y,
        sidebar::ends(
            name_spans,
            vec![Span::styled(spend, sty(theme.fg, bg))],
            w,
            bg,
        ),
    );
    let model = crate::chat::short_model(view.model_of(hat));
    let own = view.hats.get(hat.as_str()).is_some_and(|m| m.is_override());
    let model = if hat == Role::Scribe && !own {
        "follows all hats".to_string()
    } else {
        model.to_string()
    };
    put(
        frame,
        area,
        y + 1,
        Line::from(Span::styled(
            format!("   {}", wrap::truncate(&model, w.saturating_sub(3))),
            sty(theme.dim, bg),
        )),
    );
    let may_color = if wearing { color } else { theme.dim };
    put(
        frame,
        area,
        y + 2,
        sidebar::ends(
            vec![Span::styled(
                format!("   {}", sidebar::hat_may(hat)),
                sty(may_color, bg),
            )],
            vec![Span::styled(turn_s, sty(theme.dim, bg))],
            w,
            bg,
        ),
    );
}

fn draw_side_foot(frame: &mut Frame, area: Rect, mut y: u16, view: &View, theme: Theme) {
    let bg = side_bg(theme);
    let w = usize::from(area.width);
    if y + 2 >= area.height {
        return;
    }
    y += 1;
    let turns = view.rack.turn_hats();
    if !turns.is_empty() && y < area.height {
        let mut spans = vec![Span::styled(" Turns ", sty(theme.dim, bg))];
        let room = w.saturating_sub(8).min(sidebar::TURN_DOTS);
        let shown = turns.iter().rev().take(room).rev();
        let n = shown.len();
        for hat in shown {
            spans.push(Span::styled("●", sty(theme.mode(*hat), bg)));
        }
        // The row fills to eight: the turns to come are its hollow dots.
        if n < sidebar::TURN_ROW.min(room) {
            spans.push(Span::styled(
                "○".repeat(sidebar::TURN_ROW.min(room) - n),
                sty(theme.rule, bg),
            ));
        }
        put(frame, area, y, Line::from(spans));
        y += 1;
    }
    for (name, label) in [
        ("plan.md", view.plan_file.as_deref()),
        ("audit.md", view.audit_file.as_deref()),
    ] {
        let Some(label) = label else { continue };
        if y >= area.height {
            break;
        }
        put(
            frame,
            area,
            y,
            sidebar::ends(
                vec![Span::styled(format!(" {name} "), sty(theme.dim, bg))],
                vec![Span::styled(
                    wrap::truncate(label, w.saturating_sub(name.len() + 3)),
                    sty(theme.faint, bg),
                )],
                w,
                bg,
            ),
        );
        y += 1;
    }
}

fn draw_stage(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    theme: Theme,
) -> (Rect, Rect, Option<Rect>, Rect) {
    fill(frame, area, theme.bg);
    match view.page {
        Page::Hats => {
            draw_hats_page(frame, area, view, theme);
            (area, Rect::default(), None, Rect::default())
        }
        Page::Sessions => {
            draw_sessions_page(frame, area, view, theme);
            (area, Rect::default(), None, Rect::default())
        }
        Page::Conversation => draw_conversation(frame, area, view, theme),
    }
}

fn draw_conversation(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    theme: Theme,
) -> (Rect, Rect, Option<Rect>, Rect) {
    let area = margin(area);
    let w = usize::from(area.width);
    let title = if view.session_title.trim().is_empty() {
        "New session".to_string()
    } else {
        view.session_title.clone()
    };
    let sub = subline(view);
    let status = pill(view);
    let status_w = wrap::width(&status);
    put(
        frame,
        area,
        0,
        sidebar::ends(
            vec![Span::styled(
                format!(
                    " {}",
                    wrap::truncate(&title, w.saturating_sub(status_w + 3))
                ),
                sty(theme.fg, theme.bg).add_modifier(Modifier::BOLD),
            )],
            vec![Span::styled(status, sty(pill_color(view, theme), theme.bg))],
            w,
            theme.bg,
        ),
    );
    put(
        frame,
        area,
        1,
        Line::from(Span::styled(format!(" {sub}"), sty(theme.dim, theme.bg))),
    );
    let comp_w = area.width;
    let comp_h = if area.height > 8 {
        composer::draw::workspace_height(view, comp_w).min(area.height.saturating_sub(6))
    } else {
        0
    };
    let below = area.height.saturating_sub(3 + comp_h);
    let prompt_h = panel::prompt_height(view, below, theme);
    let chat_h = area.height.saturating_sub(3 + prompt_h + comp_h);
    let chat = Rect {
        y: area.y + 3,
        height: chat_h,
        ..area
    };
    let slot = (prompt_h > 0).then_some(Rect {
        y: chat.y + chat.height,
        height: prompt_h,
        ..area
    });
    let comp = Rect {
        y: area.y + area.height.saturating_sub(comp_h),
        height: comp_h,
        ..area
    };
    let mut activity = Rect::default();
    if chat.height > 0 {
        let cf = draw::draw_chat(frame, chat, view, theme);
        activity = draw::status_hit(chat, &cf);
        // A session with nothing said yet: a fresh one counts its first
        // turn from one, so the messages decide, not the turn.
        if view.messages.is_empty() {
            draw_welcome(frame, chat, view, theme);
        }
    }
    let cursor = if comp.height > 0 {
        composer::draw::draw_workspace(frame, comp, view, theme)
    } else {
        None
    };
    if view.panels.is_empty() {
        composer::draw::paint_cursor(frame, cursor, theme);
    }
    (chat, comp, slot, activity)
}

/// A shared one-cell margin each side: it aligns the conversation's heading,
/// transcript and prompt, and the cards docked over them.
fn margin(area: Rect) -> Rect {
    Rect {
        x: area.x + u16::from(area.width > 4),
        width: area.width.saturating_sub(2),
        ..area
    }
}

fn draw_welcome(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    if area.width < 30 || area.height < 6 {
        return;
    }
    let width = area.width.saturating_sub(6).min(58);
    let inner = Rect {
        x: area.x + 3,
        y: area.y + (area.height / 5).min(4),
        width,
        height: area.height.saturating_sub((area.height / 5).min(4)),
    };
    view.marks.borrow_mut().welcome = Some(Rect {
        height: inner.height.min(6),
        ..inner
    });
    let title = match view.mode {
        Role::Plan => "Start with a plan.",
        Role::Audit => "A fresh look at your work.",
        Role::Scribe => "Make the work clear.",
        _ => "What are we building?",
    };
    let desc = match view.mode {
        Role::Plan => {
            "Describe what you want to achieve. Ryter will explore the project and propose the next steps."
        }
        Role::Audit => {
            "Ask Ryter to check the changes, run the tests, and report what needs attention."
        }
        Role::Scribe => "Ask Ryter to explain a feature, update the guide, or document a decision.",
        _ => "Describe a change, ask a question, or pick up where you left off.",
    };
    put(
        frame,
        inner,
        0,
        Line::from(Span::styled(
            title,
            sty(theme.fg, theme.bg).add_modifier(Modifier::BOLD),
        )),
    );
    let mut y = 2;
    for row in wrap::wrap_plain(desc, usize::from(width)) {
        put(
            frame,
            inner,
            y,
            Line::from(Span::styled(row, sty(theme.dim, theme.bg))),
        );
        y += 1;
    }
    if y + 2 < inner.height {
        put(
            frame,
            inner,
            y + 1,
            Line::from(vec![
                Span::styled(
                    "/ ",
                    sty(theme.mode(view.mode), theme.bg).add_modifier(Modifier::BOLD),
                ),
                Span::styled("commands    ", sty(theme.dim, theme.bg)),
                Span::styled("Alt+3 ", sty(theme.fg, theme.bg)),
                Span::styled("resume a session", sty(theme.dim, theme.bg)),
            ]),
        );
    }
}

fn draw_hats_page(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    let w = usize::from(area.width);
    let mut y = 0u16;
    put(
        frame,
        area,
        y,
        Line::from(Span::styled(
            " Hats",
            sty(theme.fg, theme.bg).add_modifier(Modifier::BOLD),
        )),
    );
    y += 1;
    let compact = area.height < 30;
    let lede = if compact {
        "Tab cycles hats. Shift+Tab switches mode."
    } else {
        "One conversation, four seats. Tab cycles hats. Shift+Tab switches mode."
    };
    for row in wrap::wrap_plain(lede, w.saturating_sub(2)) {
        if y >= area.height {
            return;
        }
        put(
            frame,
            area,
            y,
            Line::from(Span::styled(format!(" {row}"), sty(theme.dim, theme.bg))),
        );
        y += 1;
    }
    y += 1;
    if !compact && y < area.height {
        let model = crate::chat::short_model(&view.model);
        put(
            frame,
            area,
            y,
            Line::from(vec![
                Span::styled(" All hats  ", sty(theme.dim, theme.bg)),
                Span::styled(
                    model.to_string(),
                    sty(theme.fg, theme.bg).add_modifier(Modifier::BOLD),
                ),
                Span::styled("  ·  default", sty(theme.dim, theme.bg)),
            ]),
        );
        y += 2;
    }
    let groups: [(&str, &[Role]); 2] = [
        ("", &[Role::Plan, Role::Build]),
        ("Specialists", &[Role::Audit, Role::Scribe]),
    ];
    for (label, hats) in groups {
        if y >= area.height {
            return;
        }
        if !label.is_empty() {
            put(
                frame,
                area,
                y,
                Line::from(Span::styled(
                    format!(" {label}"),
                    sty(theme.faint, theme.bg),
                )),
            );
            y += 1;
        }
        for hat in hats {
            if y + if compact { 3 } else { 4 } > area.height {
                // No room for the card: the hat on one line, so a short
                // screen never hides a hat.
                if y < area.height {
                    put(frame, area, y, seat_line(*hat, view, theme, w));
                    y += 1;
                }
                continue;
            }
            y = draw_seat(frame, area, y, *hat, view, theme, compact);
        }
        y += 1;
    }
}

/// A seat as one line: its name, its model and its standing.
fn seat_line(hat: Role, view: &View, theme: Theme, w: usize) -> Line<'static> {
    let color = theme.mode(hat);
    let wearing = hat == view.mode;
    let mut title: Vec<char> = hat.hat().as_str().chars().collect();
    if let Some(c) = title.first_mut() {
        *c = c.to_ascii_uppercase();
    }
    let title: String = title.into_iter().collect();
    let name = format!(" {} {title}", if wearing { "┃" } else { " " });
    let rest = format!(
        "  {}  ·  {}",
        crate::chat::short_model(view.model_of(hat)),
        crate::sidebar::hat_may(hat)
    );
    Line::from(vec![
        Span::styled(name, sty(color, theme.bg).add_modifier(Modifier::BOLD)),
        Span::styled(
            wrap::truncate(&rest, w.saturating_sub(14)),
            sty(theme.dim, theme.bg),
        ),
    ])
}

fn draw_seat(
    frame: &mut Frame,
    area: Rect,
    y: u16,
    hat: Role,
    view: &View,
    theme: Theme,
    compact: bool,
) -> u16 {
    let inset = Rect {
        x: area.x + 1,
        y: area.y + y,
        width: area.width.saturating_sub(2),
        height: if compact { 3 } else { 4 },
    };
    if inset.width < 8 {
        return y + 5;
    }
    let color = theme.mode(hat);
    let wearing = hat == view.mode;
    let on = view.seat == hat || wearing;
    let bg = if on {
        on_bg(theme, theme.composer_bg)
    } else {
        theme.composer_bg
    };
    let border = if wearing { color } else { theme.rule };
    let edge = Style::default().fg(border).bg(bg);
    frame.render_widget(Block::default().style(Style::default().bg(bg)), inset);
    let w = usize::from(inset.width);
    let name = glyphs::hat_label(hat);
    let name_text = format!(" {name} ");
    let tag = if wearing { "wearing " } else { "" };
    let fill = w.saturating_sub(3 + wrap::width(&name_text) + wrap::width(tag));
    put(
        frame,
        inset,
        0,
        Line::from(vec![
            Span::styled("╭", edge),
            Span::styled("─", edge),
            Span::styled(name_text, sty(color, bg).add_modifier(Modifier::BOLD)),
            Span::styled(tag.to_string(), sty(color, bg)),
            Span::styled("─".repeat(fill), edge),
            Span::styled("╮", edge),
        ]),
    );
    {
        let buf = frame.buffer_mut();
        for row in 1..inset.height - 1 {
            let yy = inset.y + row;
            for x in [inset.x, inset.x + inset.width - 1] {
                if x < buf.area.width && yy < buf.area.height {
                    let c = &mut buf[(x, yy)];
                    c.set_symbol("│");
                    c.set_style(edge);
                }
            }
        }
    }
    let inner_w = w.saturating_sub(4);
    let model = crate::chat::short_model(view.model_of(hat));
    let description = if compact {
        format!("{model} · {}", sidebar::hat_may(hat))
    } else {
        hat.describe().to_string()
    };
    put(
        frame,
        inset,
        1,
        Line::from(vec![
            Span::styled("│ ", edge),
            Span::styled(wrap::truncate(&description, inner_w), sty(theme.fg, bg)),
        ]),
    );
    let own = view.hats.get(hat.as_str()).is_some_and(|m| m.is_override());
    let follows = if own { "own model" } else { "follows all hats" };
    let meta = format!(
        "{model}  ·  {follows}   {} · {}  {}",
        view.rack.of(hat).turns,
        sidebar::hat_spend(view, hat),
        sidebar::hat_may(hat)
    );
    if !compact {
        put(
            frame,
            inset,
            2,
            Line::from(vec![
                Span::styled("│ ", edge),
                Span::styled(wrap::truncate(&meta, inner_w), sty(theme.dim, bg)),
            ]),
        );
    }
    put(
        frame,
        inset,
        inset.height - 1,
        Line::from(vec![
            Span::styled("╰", edge),
            Span::styled("─".repeat(w.saturating_sub(2)), edge),
            Span::styled("╯", edge),
        ]),
    );
    y + inset.height + u16::from(!compact)
}

fn draw_sessions_page(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    let w = usize::from(area.width);
    let mut y = 0u16;
    let n = shown_sessions(view).len();
    put(
        frame,
        area,
        y,
        Line::from(Span::styled(
            " Sessions",
            sty(theme.fg, theme.bg).add_modifier(Modifier::BOLD),
        )),
    );
    y += 1;
    let lede = format!(
        "{n} in {}. Resume keeps the hat the session was left in. Enter resumes.",
        folder_name(view)
    );
    put(
        frame,
        area,
        y,
        Line::from(Span::styled(
            format!(" {}", wrap::truncate(&lede, w.saturating_sub(2))),
            sty(theme.dim, theme.bg),
        )),
    );
    y += 2;
    if y >= area.height {
        return;
    }
    // Two spaces: the row mark sits in front of the title, and the header
    // lines up with the title rather than the mark.
    let head = ["  Session", "Left in", "Msgs", "Spend", "Updated"];
    put(
        frame,
        area,
        y,
        Line::from(Span::styled(
            format!(" {}", session_cols(&head.map(str::to_string), w)),
            sty(theme.faint, theme.bg),
        )),
    );
    y += 1;
    let rows = shown_sessions(view);
    if rows.is_empty() {
        put(
            frame,
            area,
            y,
            Line::from(Span::styled(
                " No saved sessions yet.",
                sty(theme.dim, theme.bg),
            )),
        );
        return;
    }
    let sel = view.session_at.min(rows.len() - 1);
    let room = usize::from(area.height.saturating_sub(y));
    let first = sel.saturating_sub(room.saturating_sub(1));
    for (i, row) in rows.iter().enumerate().skip(first).take(room) {
        let on = i == sel && view.focus == Focus::Stage;
        let bg = if on { theme.selection_bg } else { theme.bg };
        let hat = hat_from_name(&row.hat);
        let mark = if row.id == view.session_id {
            format!("{} ", glyphs::mark(Mark::Status, view.ui.nerd_font))
        } else {
            "  ".to_string()
        };
        let cells = [
            format!("{mark}{}", row.title),
            glyphs::hat_label(hat),
            row.messages.to_string(),
            row.spend.clone(),
            row.updated.clone(),
        ];
        let line = session_cols(&cells, w);
        put(
            frame,
            area,
            y,
            Line::from(vec![
                Span::styled(" ", sty(theme.fg, bg)),
                Span::styled(line, sty(theme.fg, bg)),
            ]),
        );
        y += 1;
    }
}

fn draw_inspector(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    let bg = side_bg(theme);
    fill(frame, area, bg);
    // One column of pad lives in the strings. The right column stays empty,
    // matching the sidebar. The focus bar covers that pad when this pane is on.
    let inner = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(3),
        ..area
    };
    match view.page {
        Page::Hats => draw_hat_inspector(frame, inner, view, theme),
        Page::Sessions => draw_session_inspector(frame, inner, view, theme),
        Page::Conversation => draw_conv_inspector(frame, inner, view, theme),
    }
    if area.width > 0 {
        vertical_rule(
            frame,
            Rect { width: 1, ..area },
            if view.focus == Focus::Inspector {
                theme.mode(view.mode)
            } else {
                theme.rule
            },
            bg,
        );
    }
}

fn draw_conv_inspector(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    let bg = side_bg(theme);
    let w = usize::from(area.width);
    let mut y = 0u16;
    let section = |frame: &mut Frame, y: &mut u16, title: &str| {
        if *y >= area.height {
            return false;
        }
        put(
            frame,
            area,
            *y,
            Line::from(Span::styled(format!(" {title}"), sty(theme.faint, bg))),
        );
        *y += 1;
        true
    };
    if section(frame, &mut y, "Activity") {
        let model = crate::chat::short_model(view.model_of(view.working_hat()));
        put(
            frame,
            area,
            y,
            Line::from(Span::styled(
                format!(" {}", wrap::truncate(model, w.saturating_sub(2))),
                sty(theme.fg, bg).add_modifier(Modifier::BOLD),
            )),
        );
        y += 1;
        let verb = {
            let mut s = view.activity.verb.label();
            if let Some(c) = s.chars().next() {
                s.replace_range(..c.len_utf8(), &c.to_uppercase().to_string());
            }
            s
        };
        // A turn in flight shimmers; a verb at rest is said in its colour.
        let mut verb_spans = vec![Span::styled(" ", sty(theme.dim, bg))];
        if view.activity.busy() {
            verb_spans.extend(crate::shimmer::spans(
                &verb,
                sty(pill_color(view, theme), bg),
                theme.fg,
                view.now_ms,
                theme.mode,
            ));
        } else {
            verb_spans.push(Span::styled(verb, sty(pill_color(view, theme), bg)));
        }
        put(frame, area, y, Line::from(verb_spans));
        y += 1;
        if !view.activity.current.is_empty() && y < area.height {
            let tool = if view.ui.nerd_font
                && matches!(view.activity.verb, crate::activity::Verb::Tool(_))
            {
                format!("{} ", glyphs::hat_icon(Role::Build, true))
            } else {
                String::new()
            };
            put(
                frame,
                area,
                y,
                Line::from(Span::styled(
                    format!(
                        " {tool}{}",
                        wrap::truncate(
                            &view.activity.current,
                            w.saturating_sub(2 + wrap::width(&tool))
                        )
                    ),
                    sty(theme.dim, bg),
                )),
            );
            y += 1;
        }
        // The trace: output tokens a second while the turn runs. The row
        // is kept from the first second so the blocks below do not jump.
        if view.activity.busy() && y < area.height {
            let mut history = view.pulse.history(view.now_ms).to_vec();
            history.reverse();
            let label = format!("{} tok/s", view.pulse.rate(view.now_ms).unwrap_or(0));
            let trace_w = w.saturating_sub(wrap::width(&label) + 4).min(24);
            let mut spans = vec![Span::styled(" ", sty(theme.dim, bg))];
            if history.iter().any(|n| *n > 0) && trace_w >= 4 {
                spans.extend(trace_spans(&history, trace_w, pill_color(view, theme), bg));
                put(
                    frame,
                    area,
                    y,
                    sidebar::ends(spans, vec![Span::styled(label, sty(theme.dim, bg))], w, bg),
                );
            }
            y += 1;
        }
        y += 1;
    }
    rule(frame, area, &mut y, theme);
    if section(frame, &mut y, "Context") && y < area.height {
        let frac = view.ctx_frac().clamp(0.0, 1.0);
        let (used, window) = match (view.ctx_tokens, view.ctx_window) {
            (Some(u), Some(n)) if n > 0 => (short_count(u), short_count(n)),
            _ => ("—".into(), "—".into()),
        };
        let pct = view
            .ctx_pct
            .map(|p| format!("{p}%"))
            .unwrap_or_else(|| "—".into());
        let ring_fits = area.height >= 28
            && w >= ring::WIDTH + 10
            && theme.mode != crate::theme::ColorMode::Mono
            && y + ring::HEIGHT as u16 <= area.height;
        if ring_fits {
            // The dial: the ring at the left, the counts beside it, the
            // percentage in its middle.
            let rows = ring::rows(frac, sidebar::gauge_color(view, theme), theme.rule, bg);
            let beside_x = ring::WIDTH + 3;
            for (i, row) in rows.into_iter().enumerate() {
                let mut spans = vec![Span::styled("  ", sty(theme.dim, bg))];
                spans.extend(row.spans);
                let beside = match i {
                    1 => Some(Span::styled(
                        format!(" {used}"),
                        sty(theme.fg, bg).add_modifier(Modifier::BOLD),
                    )),
                    2 => Some(Span::styled(format!(" of {window}"), sty(theme.dim, bg))),
                    _ => None,
                };
                if let Some(s) = beside {
                    spans.push(s);
                }
                put(frame, area, y + i as u16, Line::from(spans));
            }
            // The percentage sits in the ring's middle row, centred.
            let mid = y + (ring::HEIGHT / 2) as u16;
            let inner_w = ring::WIDTH - 2;
            let text = wrap::truncate(&pct, inner_w);
            let x = 2 + 1 + (inner_w - wrap::width(&text)) / 2;
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    text.clone(),
                    sty(theme.fg, bg).add_modifier(Modifier::BOLD),
                ))),
                Rect {
                    x: area.x + x as u16,
                    y: area.y + mid,
                    width: (wrap::width(&text) as u16).min(area.width.saturating_sub(x as u16)),
                    height: 1,
                },
            );
            let _ = beside_x;
            y += ring::HEIGHT as u16 + 1;
        } else {
            let gauge_w = w.saturating_sub(2).clamp(4, 24);
            let mut spans = vec![Span::styled(" ", sty(theme.dim, bg))];
            spans.extend(gauge_spans(
                frac,
                gauge_w,
                sidebar::gauge_color(view, theme),
                theme.rule,
                bg,
            ));
            put(frame, area, y, Line::from(spans));
            y += 1;
            put(
                frame,
                area,
                y,
                sidebar::ends(
                    vec![Span::styled(
                        format!(" {used} of {window}"),
                        sty(theme.dim, bg),
                    )],
                    vec![Span::styled(pct, sty(theme.dim, bg))],
                    w,
                    bg,
                ),
            );
            y += 2;
        }
    }
    rule(frame, area, &mut y, theme);
    let cap = sidebar::cap(view);
    if section(frame, &mut y, &format!("Spend · {cap}")) && y < area.height {
        let (text, color) = sidebar::session_spend(view, theme);
        let pct = if view.budget_usd > 0.0 {
            view.spend
                .map(|spent| format!("{:.1}% of cap", spent / view.budget_usd * 100.0))
                .unwrap_or_else(|| "unpriced".into())
        } else {
            String::new()
        };
        put(
            frame,
            area,
            y,
            sidebar::ends(
                vec![Span::styled(
                    format!(" {text}"),
                    sty(color, bg).add_modifier(Modifier::BOLD),
                )],
                vec![Span::styled(pct, sty(theme.dim, bg))],
                w,
                bg,
            ),
        );
        y += 1;
        // Under a cap, the session's part of it.
        if view.budget_usd > 0.0 && view.spend.is_some() && y < area.height {
            let frac = (view.spend.unwrap_or(0.0) / view.budget_usd).clamp(0.0, 1.0);
            let mut spans = vec![Span::styled(" ", sty(theme.dim, bg))];
            spans.extend(gauge_spans(
                frac,
                w.saturating_sub(2).clamp(4, 24),
                theme.mode(view.mode),
                theme.rule,
                bg,
            ));
            put(frame, area, y, Line::from(spans));
            y += 1;
        }
        // A row for each hat that spent, and for the one that is on: its
        // name, a bar against the hat that spent most, and its amount.
        let mut most = 0.0f64;
        for hat in HATS {
            if let Some(row) = view.spend_rows_role.get(hat.as_str()) {
                most = most.max(row.usd);
            }
        }
        for hat in HATS {
            if y >= area.height {
                break;
            }
            let usd = view
                .spend_rows_role
                .get(hat.as_str())
                .map(|r| r.usd)
                .unwrap_or(0.0);
            if usd <= 0.0 && hat != view.mode {
                continue;
            }
            let frac = if most > 0.0 { usd / most } else { 0.0 };
            let mut left = vec![
                Span::styled(
                    format!(" {:<6}", glyphs::hat_label(hat)),
                    sty(theme.mode(hat), bg),
                ),
                Span::styled(" ", sty(theme.dim, bg)),
            ];
            left.extend(gauge_spans(frac, 8, theme.mode(hat), theme.rule, bg));
            put(
                frame,
                area,
                y,
                sidebar::ends(
                    left,
                    vec![Span::styled(
                        sidebar::hat_spend(view, hat),
                        sty(theme.dim, bg),
                    )],
                    w,
                    bg,
                ),
            );
            y += 1;
        }
        // B — project cost glance when the project has calls or unpriced calls.
        // Absent when empty; never a fake $0.00 row.
        if let Some(p) = &view.project_spend {
            if p.calls > 0 || p.unpriced_calls > 0 {
                y = draw_project_spend_glance(frame, area, y, p, theme, bg, w);
            }
        }
        y += 1;
    }
    if let Some(files) = &view.uncommitted {
        if !files.is_empty() {
            rule(frame, area, &mut y, theme);
        }
        if !files.is_empty() && section(frame, &mut y, &format!("Changes · {}", files.len())) {
            for f in files.iter().take(6) {
                if y >= area.height {
                    break;
                }
                let added = format!("+{}", f.added);
                let removed = if f.removed == 0 {
                    String::new()
                } else {
                    format!(" −{}", f.removed)
                };
                let path_w = w.saturating_sub(wrap::width(&added) + wrap::width(&removed) + 1);
                put(
                    frame,
                    area,
                    y,
                    sidebar::ends(
                        vec![Span::styled(
                            file_label(&f.path, path_w, view.ui.nerd_font),
                            sty(theme.fg, bg),
                        )],
                        vec![
                            Span::styled(added, sty(theme.success, bg)),
                            Span::styled(removed, sty(theme.error, bg)),
                        ],
                        w,
                        bg,
                    ),
                );
                y += 1;
            }
            y += 1;
        }
    }
    if !view.plan_steps.is_empty() || view.plan_file.is_some() {
        rule(frame, area, &mut y, theme);
    }
    if !view.plan_steps.is_empty() {
        let done = view.plan_steps.iter().filter(|(d, _)| *d).count();
        let title = format!("Plan · {done} of {}", view.plan_steps.len());
        if section(frame, &mut y, &title) {
            for (done, text) in view.plan_steps.iter().take(6) {
                if y >= area.height {
                    break;
                }
                let mark = if *done { "✓" } else { "●" };
                let color = if *done {
                    theme.dim
                } else {
                    theme.mode(view.mode)
                };
                put(
                    frame,
                    area,
                    y,
                    Line::from(vec![
                        Span::styled(format!(" {mark} "), sty(color, bg)),
                        Span::styled(
                            wrap::truncate(text, w.saturating_sub(4)),
                            sty(if *done { theme.dim } else { theme.fg }, bg),
                        ),
                    ]),
                );
                y += 1;
            }
        }
    } else if let Some(label) = &view.plan_file {
        // The day and the heading, over two rows when the pane is narrow.
        if section(frame, &mut y, "Plan") {
            for row in wrap::wrap_plain(label, w.saturating_sub(2))
                .into_iter()
                .take(2)
            {
                if y >= area.height {
                    break;
                }
                put(
                    frame,
                    area,
                    y,
                    Line::from(Span::styled(format!(" {row}"), sty(theme.fg, bg))),
                );
                y += 1;
            }
        }
    }
}

fn draw_hat_inspector(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    let bg = side_bg(theme);
    let w = usize::from(area.width);
    let mut y = 0u16;
    let wearing = glyphs::hat_label(view.mode);
    let line = if view.busy && view.mode != view.working_hat() {
        format!(
            " Wearing {wearing} for the next message. {} is still in flight.",
            glyphs::hat_label(view.working_hat())
        )
    } else {
        format!(" Wearing {wearing}.")
    };
    for row in wrap::wrap_plain(&line, w.saturating_sub(1)) {
        if y >= area.height {
            return;
        }
        put(
            frame,
            area,
            y,
            Line::from(Span::styled(row, sty(theme.fg, bg))),
        );
        y += 1;
    }
    y += 1;
    let hat = view.seat;
    put(
        frame,
        area,
        y,
        Line::from(Span::styled(" Seat", sty(theme.faint, bg))),
    );
    y += 1;
    put(
        frame,
        area,
        y,
        Line::from(Span::styled(
            format!(" {}", glyphs::hat_label(hat)),
            sty(theme.mode(hat), bg).add_modifier(Modifier::BOLD),
        )),
    );
    y += 1;
    for row in wrap::wrap_plain(hat.describe(), w.saturating_sub(2)) {
        if y >= area.height {
            return;
        }
        put(
            frame,
            area,
            y,
            Line::from(Span::styled(format!(" {row}"), sty(theme.dim, bg))),
        );
        y += 1;
    }
    y += 1;
    if y >= area.height {
        return;
    }
    put(
        frame,
        area,
        y,
        Line::from(Span::styled(" Model", sty(theme.faint, bg))),
    );
    y += 1;
    let model = view.model_of(hat);
    put(
        frame,
        area,
        y,
        Line::from(Span::styled(
            format!(" {}", crate::chat::short_model(model)),
            sty(theme.fg, bg).add_modifier(Modifier::BOLD),
        )),
    );
    y += 1;
    let own = view.hats.get(hat.as_str()).is_some_and(|m| m.is_override());
    put(
        frame,
        area,
        y,
        Line::from(Span::styled(
            format!(" {}", if own { "own model" } else { "follows all hats" }),
            sty(theme.dim, bg),
        )),
    );
    y += 2;
    if y >= area.height {
        return;
    }
    put(
        frame,
        area,
        y,
        Line::from(Span::styled(" Reasoning", sty(theme.faint, bg))),
    );
    y += 1;
    put(
        frame,
        area,
        y,
        Line::from(Span::styled(
            format!(" {}", view.reasoning_label(model)),
            sty(theme.fg, bg),
        )),
    );
    y += 2;
    if y < area.height {
        put(
            frame,
            area,
            y,
            Line::from(Span::styled(" Saved", sty(theme.faint, bg))),
        );
        y += 1;
        put(
            frame,
            area,
            y,
            Line::from(Span::styled(" ~/.ryter/hats.toml", sty(theme.dim, bg))),
        );
    }
}

fn draw_session_inspector(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    let bg = side_bg(theme);
    let w = usize::from(area.width);
    let rows = shown_sessions(view);
    let Some(row) = rows.get(view.session_at.min(rows.len().saturating_sub(1))) else {
        put(
            frame,
            area,
            0,
            Line::from(Span::styled(" No session.", sty(theme.dim, bg))),
        );
        return;
    };
    let hat = hat_from_name(&row.hat);
    let mut y = 0u16;
    put(
        frame,
        area,
        y,
        Line::from(Span::styled(" Session", sty(theme.faint, bg))),
    );
    y += 1;
    put(
        frame,
        area,
        y,
        Line::from(Span::styled(
            format!(" {}", wrap::truncate(&row.title, w.saturating_sub(2))),
            sty(theme.fg, bg).add_modifier(Modifier::BOLD),
        )),
    );
    y += 1;
    put(
        frame,
        area,
        y,
        Line::from(Span::styled(
            format!(" {}", wrap::truncate(&row.sub, w.saturating_sub(2))),
            sty(theme.dim, bg),
        )),
    );
    y += 2;
    let current = row.id == view.session_id || row.id.is_empty();
    put(
        frame,
        area,
        y,
        Line::from(Span::styled(
            if current {
                " Current"
            } else {
                " Enter resumes"
            },
            sty(theme.mode(view.mode), bg),
        )),
    );
    y += 2;
    put(
        frame,
        area,
        y,
        Line::from(Span::styled(" Left in", sty(theme.faint, bg))),
    );
    y += 1;
    put(
        frame,
        area,
        y,
        Line::from(Span::styled(
            format!(
                " {} {}",
                glyphs::mark(Mark::Status, view.ui.nerd_font),
                glyphs::hat_label(hat)
            ),
            sty(theme.mode(hat), bg).add_modifier(Modifier::BOLD),
        )),
    );
    y += 2;
    put(
        frame,
        area,
        y,
        Line::from(Span::styled(
            format!(" {}  ·  {}", row.spend, row.updated),
            sty(theme.dim, bg),
        )),
    );
}

fn draw_hint(frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
    let bg = side_bg(theme);
    fill(frame, area, bg);
    let asking = view.panels.has_modal();
    // A card's keys are its own: the plan panel's y/e/n are approve,
    // adjust and reject, the audit card's Enter is repair, and only the
    // allow card has `a edits`. The legend is the panel's.
    let legend = view
        .panels
        .top()
        .filter(|_| asking)
        .map(|p| p.legend(view))
        .unwrap_or_default();
    let own: Vec<(&str, &str)> = legend
        .split(" · ")
        .filter_map(|item| item.split_once(' '))
        .collect();
    let mut items: Vec<(&str, &str)> = Vec::new();
    if view.palette.is_some() {
        items.extend([("↑↓", "move"), ("enter", "run"), ("esc", "close")]);
    } else if asking {
        items.extend(own);
        items.extend([
            ("tab", "hat"),
            ("⇧tab", "mode"),
            ("/", "commands"),
            ("f2", "instruments"),
            ("f6 f7", "panes"),
        ]);
    } else {
        if view.page == Page::Sessions || view.focus == Focus::Sidebar {
            items.extend([("↑↓", "select"), ("enter", "open")]);
        }
        items.extend([
            ("tab", "hat"),
            ("⇧tab", "mode"),
            ("/", "commands"),
            ("alt 1–3", "views"),
            ("f2", "instruments"),
            ("f6 f7", "panes"),
        ]);
    }
    let focus = match view.focus {
        Focus::Sidebar => "sidebar",
        Focus::Stage => match view.page {
            Page::Conversation => "conversation",
            Page::Hats => "hats",
            Page::Sessions => "sessions",
        },
        Focus::Inspector => "instruments",
    };
    // With the instruments folded away, the hat and the spend take their place.
    let focus_s = if columns(area.width, view.panel_visible).1 == 0 {
        format!("{} · {}", glyphs::hat_label(view.mode), view.spend_label())
    } else {
        format!("focus · {focus}")
    };
    let w = usize::from(area.width);
    while items.len() > 1 {
        let mut used = wrap::width(&focus_s) + 2;
        for (k, l) in &items {
            used += wrap::width(k) + wrap::width(l) + 3;
        }
        if used <= w {
            break;
        }
        items.pop();
    }
    let mut spans = vec![Span::styled(" ", sty(theme.dim, bg))];
    for (i, (k, l)) in items.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", sty(theme.dim, bg)));
        }
        spans.push(Span::styled(
            k.to_string(),
            sty(theme.fg, bg).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(" {l}"), sty(theme.dim, bg)));
    }
    let used: usize = spans.iter().map(|s| wrap::width(&s.content)).sum();
    let gap = w.saturating_sub(used + wrap::width(&focus_s) + 1);
    spans.push(Span::styled(" ".repeat(gap), sty(theme.dim, bg)));
    spans.push(Span::styled(focus_s, sty(theme.dim, bg)));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn shown_sessions(view: &View) -> Vec<SavedSession> {
    if !view.sessions.is_empty() {
        return view.sessions.clone();
    }
    vec![current_saved(view)]
}

fn current_saved(view: &View) -> SavedSession {
    let title = if view.session_title.trim().is_empty() {
        "New session".into()
    } else {
        view.session_title.clone()
    };
    let sub = view
        .git_branch
        .clone()
        .unwrap_or_else(|| "this session".into());
    SavedSession {
        id: view.session_id.clone(),
        title,
        sub,
        hat: view.mode.as_str().to_string(),
        messages: view.turn as usize,
        spend: view.spend_label(),
        updated: "now".into(),
    }
}

fn subline(view: &View) -> String {
    let mut parts = Vec::new();
    if let Some(b) = &view.git_branch {
        parts.push(b.clone());
    }
    match &view.uncommitted {
        Some(f) if f.is_empty() => parts.push("nothing uncommitted".into()),
        Some(f) => parts.push(format!(
            "{} file{} uncommitted",
            f.len(),
            if f.len() == 1 { "" } else { "s" }
        )),
        None => {}
    }
    if parts.is_empty() {
        folder_name(view)
    } else {
        parts.join("  ·  ")
    }
}

fn pill(view: &View) -> String {
    let waiting =
        view.panels.has_modal() || matches!(view.activity.verb, crate::activity::Verb::Waiting);
    let dot = glyphs::mark(
        if waiting { Mark::Waiting } else { Mark::Status },
        view.ui.nerd_font,
    );
    if waiting {
        return format!("{dot} Waiting for you · {}", perm_word(view));
    }
    let mut s = view.activity.verb.label();
    if let Some(c) = s.chars().next() {
        s.replace_range(..c.len_utf8(), &c.to_uppercase().to_string());
    }
    if view.busy {
        format!("{dot} {s}")
    } else {
        format!("{dot} {s} · {}", perm_word(view))
    }
}

fn file_label(path: &str, width: usize, nerd: bool) -> String {
    let icon = glyphs::mark(Mark::File, nerd);
    if icon.is_empty() {
        format!(" {}", wrap::truncate(path, width.saturating_sub(1)))
    } else {
        format!(
            " {icon} {}",
            wrap::truncate(path, width.saturating_sub(wrap::width(icon) + 2))
        )
    }
}

fn perm_word(view: &View) -> &'static str {
    if view.perm_mode == "always" {
        "allows edits"
    } else {
        "asks first"
    }
}

fn pill_color(view: &View, theme: Theme) -> ratatui::style::Color {
    use crate::activity::Verb;
    match &view.activity.verb {
        Verb::Waiting | Verb::WaitingModel => theme.warn,
        Verb::Failed => theme.error,
        Verb::Thinking | Verb::Writing | Verb::Tool(_) | Verb::Cancelling => theme.mode(view.mode),
        _ => theme.dim,
    }
}

fn folder_name(view: &View) -> String {
    let raw = view
        .workspace
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| view.cwd.clone());
    std::path::Path::new(&raw)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or(raw)
}

fn hat_from_name(name: &str) -> Role {
    match name {
        "plan" => Role::Plan,
        "audit" => Role::Audit,
        "scribe" => Role::Scribe,
        _ => Role::Build,
    }
}

fn session_cols(cells: &[String], width: usize) -> String {
    let width = width.saturating_sub(1);
    // Keep the title first, then spend and recency. The other columns come
    // back when the table has room; every row uses the same columns.
    let mut fields = Vec::new();
    if width >= 66 {
        fields.extend([(1, 8, false), (2, 5, true)]);
    }
    if width >= 28 {
        fields.push((3, 8, true));
    }
    if width >= 42 {
        fields.push((4, 10, true));
    }
    let meta_w: usize = fields.iter().map(|(_, n, _)| n + 1).sum();
    fields.insert(0, (0, width.saturating_sub(meta_w), false));
    let mut out = String::new();
    for (i, (field, n, right)) in fields.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let shown = wrap::truncate(cells.get(*field).map(String::as_str).unwrap_or(""), *n);
        let pad = n.saturating_sub(wrap::width(&shown));
        if *right {
            out.push_str(&" ".repeat(pad));
            out.push_str(&shown);
        } else {
            out.push_str(&shown);
            out.push_str(&" ".repeat(pad));
        }
    }
    out
}

/// A thick line with a cap (`━━╸`) on a dashed track (`╌╌`). A half cell
/// or more past the last whole one earns the cap.
fn gauge_spans(
    frac: f64,
    width: usize,
    fill: Color,
    track: Color,
    bg: Color,
) -> Vec<Span<'static>> {
    let width = width.max(1);
    let exact = frac.clamp(0.0, 1.0) * width as f64;
    let full = (exact.floor() as usize).min(width);
    let cap = usize::from(exact - full as f64 >= 0.5 && full < width);
    let mut spans = Vec::new();
    let ink = Style::default().fg(fill).bg(bg);
    if full > 0 {
        spans.push(Span::styled("━".repeat(full), ink));
    }
    if cap > 0 {
        spans.push(Span::styled("╸".to_string(), ink));
    }
    let used = full + cap;
    if used < width {
        spans.push(Span::styled(
            "╌".repeat(width - used),
            Style::default().fg(track).bg(bg),
        ));
    }
    spans
}

/// Tokens a second over the last seconds as a wave of eighth blocks, the
/// tallest at the busiest second.
fn trace_spans(rate: &[u64], width: usize, ink: Color, bg: Color) -> Vec<Span<'static>> {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let shown = &rate[rate.len().saturating_sub(width)..];
    let most = shown.iter().copied().max().unwrap_or(0).max(1);
    let wave: String = shown
        .iter()
        .map(|v| BARS[((*v * 7) / most) as usize])
        .collect();
    vec![Span::styled(wave, Style::default().fg(ink).bg(bg))]
}

fn short_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1000 {
        format!("{}k", n / 1000)
    } else {
        n.to_string()
    }
}

fn rule(frame: &mut Frame, area: Rect, y: &mut u16, theme: Theme) {
    if *y >= area.height {
        return;
    }
    let w = usize::from(area.width);
    let line = if w == 0 {
        String::new()
    } else {
        format!(" {}", glyphs::HAIRLINE_H.repeat(w.saturating_sub(1)))
    };
    put(
        frame,
        area,
        *y,
        Line::from(Span::styled(line, sty(theme.rule, side_bg(theme)))),
    );
    *y += 1;
}

/// Sidebar and instruments, a step off the conversation. In 256-color the
/// two truecolor tints can quantize to one index; step the gray ramp then.
/// The fill under the row that is on. In sixteen colours the fill is the
/// terminal's dark grey, and a hat's colour on it is nothing to read, so
/// the row keeps `plain` and is marked by its bar and its bold name.
fn on_bg(theme: Theme, plain: Color) -> Color {
    if theme.mode == crate::theme::ColorMode::Ansi16 {
        plain
    } else {
        theme.selection_bg
    }
}

/// A session's cost as the list says it: `$0` before anything has run,
/// `$?.??` only when something ran and had no price.
fn session_spend(total: Option<f64>, messages: usize) -> String {
    match total {
        None if messages == 0 => "$0".into(),
        None => ryter_core::format_usd(None),
        Some(usd) => crate::chat::turn_usd(usd),
    }
}

fn side_bg(theme: Theme) -> Color {
    if theme.mode == crate::theme::ColorMode::Ansi256 {
        let side = crate::theme::quantize_256(theme.sidebar_bg);
        let stage = crate::theme::quantize_256(theme.bg);
        if side == stage {
            return Color::Indexed(237);
        }
        return side;
    }
    if theme.sidebar_bg == theme.bg {
        return match theme.bg {
            Color::Rgb(r, g, b) => Color::Rgb(
                r.saturating_add(6),
                g.saturating_add(7),
                b.saturating_add(10),
            ),
            other => other,
        };
    }
    theme.sidebar_bg
}

/// Inspector Spend block: project total, this month, unpriced, sessions.
/// Call only when `p` has calls or unpriced calls.
fn draw_project_spend_glance(
    frame: &mut Frame,
    area: Rect,
    mut y: u16,
    p: &ryter_core::project::ProjectSpend,
    theme: Theme,
    bg: Color,
    w: usize,
) -> u16 {
    if y >= area.height {
        return y;
    }
    put(
        frame,
        area,
        y,
        Line::from(Span::styled(" Project", sty(theme.dim, bg))),
    );
    y += 1;
    if y < area.height {
        // Three places under a dime, as the hats and the spend panel
        // say it: a project that has cost something is never `$0.00`.
        let total = if p.calls > 0 && p.calls == p.unpriced_calls {
            "$?.??".into()
        } else {
            crate::chat::turn_usd(p.total_usd)
        };
        put(
            frame,
            area,
            y,
            sidebar::ends(
                vec![Span::styled(
                    format!(" {total}"),
                    sty(theme.fg, bg).add_modifier(Modifier::BOLD),
                )],
                vec![Span::styled("total", sty(theme.dim, bg))],
                w,
                bg,
            ),
        );
        y += 1;
    }
    if y < area.height {
        put(
            frame,
            area,
            y,
            sidebar::ends(
                vec![Span::styled(
                    format!(" {}", crate::chat::turn_usd(p.this_month())),
                    sty(theme.dim, bg),
                )],
                vec![Span::styled("this month", sty(theme.dim, bg))],
                w,
                bg,
            ),
        );
        y += 1;
    }
    if p.unpriced_calls > 0 && y < area.height {
        put(
            frame,
            area,
            y,
            Line::from(Span::styled(
                format!(" {} unpriced", p.unpriced_calls),
                sty(theme.warn, bg),
            )),
        );
        y += 1;
    }
    if p.sessions > 0 && y < area.height {
        put(
            frame,
            area,
            y,
            Line::from(Span::styled(
                format!(" {} sessions", p.sessions),
                sty(theme.dim, bg),
            )),
        );
        y += 1;
    }
    y
}

fn vertical_rule(frame: &mut Frame, area: Rect, color: Color, bg: Color) {
    for y in area.y..area.bottom() {
        frame.render_widget(
            Paragraph::new(glyphs::HAIRLINE_V).style(sty(color, bg)),
            Rect {
                y,
                height: 1,
                ..area
            },
        );
    }
}

fn fill(frame: &mut Frame, area: Rect, bg: ratatui::style::Color) {
    frame.render_widget(Block::default().style(Style::default().bg(bg)), area);
}

fn put(frame: &mut Frame, area: Rect, row: u16, line: Line) {
    if row >= area.height || area.width == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(line),
        Rect {
            y: area.y + row,
            height: 1,
            ..area
        },
    );
}

fn sty(fg: ratatui::style::Color, bg: ratatui::style::Color) -> Style {
    Style::default().fg(fg).bg(bg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Action;
    use crate::panel::modal::PermissionModal;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ryter_core::Role;

    fn ws() -> View {
        let mut v = View::new("spacexsi".into(), "grok-4.6".into(), "~/shop".into());
        v.username = "Dusty".into();
        v.git_branch = Some("search-patch".into());
        v.session_title = "Empty-query fix".into();
        v.theme_name = "dark".into();
        v.ctx_tokens = Some(97_000);
        v.ctx_window = Some(256_000);
        v.ctx_pct = Some(38);
        v.budget_usd = 5.0;
        v.spend = Some(0.065);
        v.set_mode(Role::Build);
        v.ui.nerd_font = false;
        v
    }

    fn press(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn alt_switches_views_and_fkeys_move_focus_and_instruments() {
        let mut v = ws();
        v.screen.set((160, 50));
        assert_eq!(
            crate::run_keys_handle(&mut v, press(KeyCode::Char('2'), KeyModifiers::ALT)),
            Action::None
        );
        assert_eq!(v.page, Page::Hats);
        assert_eq!(
            crate::run_keys_handle(&mut v, press(KeyCode::Char('3'), KeyModifiers::ALT)),
            Action::None
        );
        assert_eq!(v.page, Page::Sessions);
        assert_eq!(
            crate::run_keys_handle(&mut v, press(KeyCode::Char('1'), KeyModifiers::ALT)),
            Action::None
        );
        assert_eq!(v.page, Page::Conversation);
        assert!(v.panel_visible);
        crate::run_keys_handle(&mut v, press(KeyCode::F(2), KeyModifiers::NONE));
        assert!(!v.panel_visible);
        crate::run_keys_handle(&mut v, press(KeyCode::F(2), KeyModifiers::NONE));
        assert!(v.panel_visible);
        crate::run_keys_handle(&mut v, press(KeyCode::F(6), KeyModifiers::NONE));
        assert_eq!(v.focus, Focus::Sidebar);
        crate::run_keys_handle(&mut v, press(KeyCode::F(7), KeyModifiers::NONE));
        assert_eq!(v.focus, Focus::Stage);
        crate::run_keys_handle(&mut v, press(KeyCode::F(7), KeyModifiers::NONE));
        assert_eq!(v.focus, Focus::Inspector);
        // Hidden instruments are not a pane.
        v.panel_visible = false;
        v.focus = Focus::Stage;
        crate::run_keys_handle(&mut v, press(KeyCode::F(7), KeyModifiers::NONE));
        assert_eq!(v.focus, Focus::Sidebar);
    }

    #[test]
    fn tab_cycles_within_the_mode_and_shift_tab_switches_mode() {
        let mut v = ws();
        v.set_mode(Role::Plan);
        assert!(matches!(
            crate::run_keys_handle(&mut v, press(KeyCode::Tab, KeyModifiers::NONE)),
            Action::SetMode(Role::Build)
        ));
        v.set_mode(Role::Build);
        assert!(matches!(
            crate::run_keys_handle(&mut v, press(KeyCode::BackTab, KeyModifiers::NONE)),
            Action::SetMode(Role::Audit)
        ));
        v.set_mode(Role::Audit);
        assert!(matches!(
            crate::run_keys_handle(&mut v, press(KeyCode::Tab, KeyModifiers::NONE)),
            Action::SetMode(Role::Scribe)
        ));
        // A digit is typed, not a hat.
        v.composer.clear();
        let a = crate::run_keys_handle(&mut v, press(KeyCode::Char('1'), KeyModifiers::NONE));
        assert_eq!(a, Action::None);
        assert_eq!(v.composer.text(), "1");
        assert_eq!(v.mode, Role::Audit);
    }

    #[test]
    fn sidebar_enter_wears_a_hat_and_sessions_enter_resumes() {
        let mut v = ws();
        v.focus = Focus::Sidebar;
        v.side_at = 0; // plan
        assert!(matches!(
            crate::run_keys_handle(&mut v, press(KeyCode::Enter, KeyModifiers::NONE)),
            Action::SetMode(Role::Plan)
        ));
        assert_eq!(v.page, Page::Hats);
        v.page = Page::Sessions;
        v.focus = Focus::Stage;
        v.sessions = vec![
            current_saved(&v),
            SavedSession {
                id: "other".into(),
                title: "Checkout tax".into(),
                sub: "main".into(),
                hat: "plan".into(),
                messages: 2,
                spend: "$0.02".into(),
                updated: "Yesterday".into(),
            },
        ];
        v.session_at = 0;
        crate::run_keys_handle(&mut v, press(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(v.session_at, 1);
        assert_eq!(
            crate::run_keys_handle(&mut v, press(KeyCode::Enter, KeyModifiers::NONE)),
            Action::Resume("other".into())
        );
    }

    #[test]
    fn slash_opens_the_palette_and_a_hat_command_is_there() {
        let mut v = ws();
        crate::run_keys_handle(&mut v, press(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert!(v.palette.is_none(), "ctrl+k is not a palette key");
        crate::run_keys_handle(&mut v, press(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(v.palette.is_none(), "ctrl+p is not a palette key");
        crate::run_keys_handle(&mut v, press(KeyCode::Char('/'), KeyModifiers::NONE));
        assert!(v.palette.is_some(), "/ opens the palette");
        let names: Vec<_> = crate::palette::registry::COMMANDS
            .iter()
            .map(|c| c.name)
            .collect();
        for hat in ["plan", "build", "audit", "scribe"] {
            assert!(names.contains(&hat), "{hat} missing from the palette");
        }
    }

    #[test]
    fn a_permission_card_keeps_y_enter_a_and_n() {
        use ryter_core::Permission;
        let mut v = ws();
        v.now_ms = 10_000;
        v.panels
            .push(Box::new(PermissionModal::new("bash".into(), "ls".into())));
        assert_eq!(
            crate::run_keys_handle(&mut v, press(KeyCode::Char('y'), KeyModifiers::NONE)),
            Action::PermissionReply(Permission::Allow)
        );
        v.panels
            .push(Box::new(PermissionModal::new("bash".into(), "ls".into())));
        assert_eq!(
            crate::run_keys_handle(&mut v, press(KeyCode::Enter, KeyModifiers::NONE)),
            Action::PermissionReply(Permission::Allow)
        );
        v.panels
            .push(Box::new(PermissionModal::new("bash".into(), "ls".into())));
        assert_eq!(
            crate::run_keys_handle(&mut v, press(KeyCode::Char('n'), KeyModifiers::NONE)),
            Action::PermissionReply(Permission::Deny)
        );
        v.panels.push(Box::new(
            PermissionModal::new("search_replace".into(), "app/server.js".into())
                .with_answers(false, Some("edits".into())),
        ));
        assert_eq!(
            crate::run_keys_handle(&mut v, press(KeyCode::Char('a'), KeyModifiers::NONE)),
            Action::PermissionReply(Permission::Always)
        );
    }

    #[test]
    fn the_screen_names_ryter_and_carries_the_hat() {
        let v = ws();
        let text = crate::draw::render_to_string(&v, 160, 50);
        assert!(text.contains("RYTER"), "{text}");
        assert!(!text.contains("█▀█"), "the block wordmark is gone");
        assert!(!text.contains('\u{1D5D4}'), "math-sans headings are gone");
        assert!(!text.contains("Search commands"), "{text}");
        assert!(text.contains("Conversation"), "{text}");
        assert!(text.contains("Specialists"), "{text}");
        assert!(text.contains("Build"), "{text}");
        assert!(text.contains("alt 1"), "{text}");
        assert!(text.contains("focus · conversation"), "{text}");
        assert!(text.contains("tab") && text.contains("commands"), "{text}");
        assert!(text.contains(" mode"), "{text}");
        assert!(!text.contains("row"), "{text}");
        // No number badge on a hat. The view key is "alt 1", not a hat number.
        assert!(!text.contains("1 plan") && !text.contains("[1]"), "{text}");
        assert!(!text.contains('▐'), "half-block pill caps are gone");
        let theme = Theme::truecolor_dark();
        let buf = crate::draw::render_buffer(&v, 160, 50, theme);
        let chip = (0..50u16)
            .flat_map(|y| (0..160u16).map(move |x| (x, y)))
            .find(|&at| buf[at].symbol() == "B" && buf[at].bg == theme.build)
            .expect("build chip");
        assert_eq!(buf[chip].bg, theme.build, "the chip is the hat");
        let light = Theme::truecolor_light();
        let light_text = crate::draw::render_with_theme(&v, 160, 50, light);
        assert!(light_text.contains("RYTER"), "{light_text}");
        assert!(!light_text.contains('\u{1D5D4}'), "{light_text}");
        let mut hats = v.clone();
        hats.page = Page::Hats;
        let hats_text = crate::draw::render_to_string(&hats, 160, 50);
        assert!(hats_text.contains("wearing"), "{hats_text}");
        assert!(
            hats_text.contains("Shift+Tab") || hats_text.contains("four seats"),
            "{hats_text}"
        );
        let mut sessions = v.clone();
        sessions.page = Page::Sessions;
        sessions.sessions = vec![
            current_saved(&sessions),
            SavedSession {
                id: "tax".into(),
                title: "Checkout tax rounding".into(),
                sub: "main".into(),
                hat: "plan".into(),
                messages: 2,
                spend: "$0.02".into(),
                updated: "Yesterday".into(),
            },
        ];
        let sessions_text = crate::draw::render_to_string(&sessions, 160, 50);
        assert!(
            sessions_text.contains("Checkout tax rounding"),
            "{sessions_text}"
        );
        let mut pal = v.clone();
        crate::run_keys_handle(&mut pal, press(KeyCode::Char('/'), KeyModifiers::NONE));
        let pal_text = crate::draw::render_to_string(&pal, 160, 50);
        assert!(
            pal_text.contains("enter") && pal_text.contains("close"),
            "{pal_text}"
        );
    }

    #[test]
    fn a_click_on_a_hat_wears_it() {
        let v = ws();
        let hit = crate::draw::render_hit(&v, 160, 50);
        let rect = hit
            .clicks
            .iter()
            .find(|(c, _)| *c == WorkspaceClick::Wear(Role::Plan))
            .map(|(_, r)| *r)
            .expect("plan row");
        let mut v = v;
        let down = crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: rect.x + 2,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        };
        let up = crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Up(crossterm::event::MouseButton::Left),
            ..down
        };
        assert_eq!(
            crate::run_mouse_handle_with(&mut v, down, &hit),
            Action::None
        );
        assert_eq!(
            crate::run_mouse_handle_with(&mut v, up, &hit),
            Action::SetMode(Role::Plan)
        );
    }

    #[test]
    fn a_click_on_a_view_tab_switches_the_page() {
        let v = ws();
        let hit = crate::draw::render_hit(&v, 160, 50);
        let rect = hit
            .clicks
            .iter()
            .find(|(c, _)| *c == WorkspaceClick::Page(2))
            .map(|(_, r)| *r)
            .expect("hats tab");
        let mut v = v;
        let down = crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: rect.x + 1,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        };
        let up = crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Up(crossterm::event::MouseButton::Left),
            ..down
        };
        assert_eq!(
            crate::run_mouse_handle_with(&mut v, down, &hit),
            Action::None
        );
        assert_eq!(crate::run_mouse_handle_with(&mut v, up, &hit), Action::None);
        assert_eq!(v.page, Page::Hats);
    }

    #[test]
    fn nerd_icons_are_opt_in() {
        let mut nerd = ws();
        nerd.ui.nerd_font = true;
        let text = crate::draw::render_to_string(&nerd, 160, 50);
        assert!(text.contains('\u{e0a0}'), "{text}");
        assert!(!text.contains('⎇'), "{text}");
        assert!(text.contains('\u{f0eb}'), "plan's icon");
    }

    fn sample_project_spend() -> ryter_core::project::ProjectSpend {
        let mut p = ryter_core::project::ProjectSpend::default();
        // Seeds this_month via month_of_now, then the glance totals override.
        p.add(Role::Build, "m", Some(1.24));
        p.total_usd = 4.82;
        p.calls = 40;
        p.unpriced_calls = 3;
        p.sessions = 12;
        p
    }

    #[test]
    fn quiet_chrome_keeps_brand_dots_drops_avatar_and_hairline() {
        let mut v = ws();
        v.username = "dusty".into();
        let text = crate::draw::render_to_string(&v, 160, 50);
        // A tall screen gives the bar a row above and below: its line is
        // the second.
        let top = text.lines().nth(1).unwrap();
        assert!(top.contains("RYTER"), "{top}");
        assert!(top.contains("Conversation"), "{top}");
        // Brand logo: three colored dots before RYTER.
        assert!(top.contains("●●● RYTER"), "{top}");
        assert!(!top.contains('☀') && !top.contains('☾'), "{top}");
        assert!(!top.contains(" dusty") && !top.contains(" Dusty"), "{top}");
        // No hairline above or below the bar's line.
        for row in [0, 2] {
            let line = text.lines().nth(row).unwrap();
            assert!(!line.contains('─'), "a hairline at row {row}: {line}");
        }
        let hit = crate::draw::render_hit(&v, 160, 50);
        assert!(
            hit.clicks
                .iter()
                .all(|(c, _)| matches!(c, WorkspaceClick::Page(_) | WorkspaceClick::Wear(_))),
            "only page/hat clicks remain: {:?}",
            hit.clicks
        );
    }

    #[test]
    fn project_spend_glance_shows_total_month_unpriced_sessions() {
        let mut v = ws();
        v.project_spend = Some(sample_project_spend());
        let text = crate::draw::render_to_string(&v, 160, 50);
        let squashed = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(squashed.contains("Project"), "{text}");
        assert!(
            squashed.contains("$4.82") && squashed.contains("total"),
            "{text}"
        );
        assert!(
            squashed.contains("$1.24") && squashed.contains("this month"),
            "{text}"
        );
        assert!(squashed.contains("3 unpriced"), "{text}");
        assert!(squashed.contains("12 sessions"), "{text}");
        // Session spend remains.
        assert!(squashed.contains("$0.065"), "{text}");
    }

    #[test]
    fn project_spend_glance_stays_absent_when_empty() {
        let mut v = ws();
        v.project_spend = Some(ryter_core::project::ProjectSpend::default());
        let text = crate::draw::render_to_string(&v, 160, 50);
        assert!(!text.contains("this month"), "{text}");
        assert!(!text.contains("unpriced"), "{text}");
        assert!(!text.contains("sessions"), "{text}");
        // No fake project $0.00.
        let insp = text
            .lines()
            .filter(|l| l.contains("Spend") || l.contains("Project") || l.contains("$0.00"))
            .collect::<Vec<_>>();
        assert!(
            insp.iter()
                .all(|l| !l.contains("Project") || !l.contains("$0.00")),
            "{insp:?}"
        );
    }

    #[test]
    fn project_spend_glance_all_unpriced_is_unknown_not_zero() {
        let mut p = ryter_core::project::ProjectSpend::default();
        p.calls = 2;
        p.unpriced_calls = 2;
        p.sessions = 1;
        let mut v = ws();
        v.project_spend = Some(p);
        let text = crate::draw::render_to_string(&v, 160, 50);
        assert!(text.contains("$?.??"), "{text}");
        assert!(text.contains("2 unpriced"), "{text}");
        // Total is unknown — never a priced $0.00 on the total row.
        assert!(
            !text
                .lines()
                .any(|l| l.contains("$0.00") && l.contains("total")),
            "{text}"
        );
    }

    /// The hint bar carries the open card's own keys: the plan panel's
    /// approve, adjust and reject; the audit card's repair; `a edits`
    /// only on an allow card.
    #[test]
    fn the_hint_bar_shows_the_open_cards_keys() {
        let mut v = ws();
        v.panels.push(Box::new(crate::panel::plan::PlanModal::new(
            "T".into(),
            "## Goal\nx\n".into(),
            0,
        )));
        let text = crate::draw::render_to_string(&v, 120, 40);
        let hint = text.lines().last().unwrap().to_string();
        assert!(
            hint.contains("approve") && hint.contains("reject"),
            "{hint}"
        );
        assert!(!hint.contains("allow") && !hint.contains("edits"), "{hint}");
        let mut v = ws();
        v.panels
            .push(Box::new(crate::panel::modal::PermissionModal::new(
                "bash".into(),
                "git push".into(),
            )));
        let text = crate::draw::render_to_string(&v, 120, 40);
        let hint = text.lines().last().unwrap().to_string();
        assert!(hint.contains("allow") && hint.contains("deny"), "{hint}");
    }

    /// In sixteen colours the hat that is on keeps the sidebar's own
    /// background: a hat's colour on the terminal's grey was unreadable.
    #[test]
    fn sixteen_colours_mark_the_hat_without_a_grey_fill() {
        let v = ws();
        let theme = Theme::default_16();
        // The rack is on screen beside the instruments from 132 columns.
        let buf = crate::draw::render_buffer(&v, 140, 40, theme);
        let text = crate::draw::render_with_theme(&v, 140, 40, theme);
        let mut title: Vec<char> = v.mode.hat().as_str().chars().collect();
        title[0] = title[0].to_ascii_uppercase();
        let title: String = title.into_iter().collect();
        let marked = format!("┃ {title}");
        let (y, row) = text
            .lines()
            .enumerate()
            .find(|(_, l)| l.contains(&marked))
            .unwrap_or_else(|| panic!("no {marked}\n{text}"));
        let at = row.find(&title).unwrap();
        let x = row[..at].chars().count() as u16;
        let cell = &buf[(x, y as u16)];
        assert_ne!(
            cell.bg,
            theme.selection_bg,
            "the fill under {:?}",
            cell.symbol()
        );
    }

    /// On a short screen the Hats page still names every hat: a hat
    /// with no room for its card gets one line.
    #[test]
    fn a_short_hats_page_names_every_hat() {
        let mut v = ws();
        v.page = Page::Hats;
        let text = crate::draw::render_to_string(&v, 80, 24);
        for hat in ["Audit", "Scribe"] {
            assert!(text.contains(hat), "{hat} missing\n{text}");
        }
    }

    /// A new session costs nothing yet; `$?.??` is for a run with no price.
    #[test]
    fn a_new_sessions_cost_is_nothing_yet() {
        assert_eq!(session_spend(None, 0), "$0");
        assert_eq!(session_spend(None, 3), "$?.??");
        assert_eq!(session_spend(Some(0.004), 3), "$0.004");
    }

    /// A project that has cost a few tenths of a cent says so on the
    /// total and this-month rows, as the hats do: never a fake `$0.00`.
    #[test]
    fn a_small_project_total_is_never_a_fake_zero() {
        let mut v = ws();
        let mut p = ryter_core::project::ProjectSpend::default();
        p.add(ryter_core::Role::Build, "m", Some(0.004));
        v.project_spend = Some(p);
        let text = crate::draw::render_to_string(&v, 160, 50);
        assert!(
            text.lines()
                .any(|l| l.contains("$0.004") && l.contains("total")),
            "{text}"
        );
        assert!(
            !text
                .lines()
                .any(|l| l.contains("$0.00 ") || l.ends_with("$0.00")),
            "{text}"
        );
    }

    #[test]
    fn session_columns_fit_without_sacrificing_the_title() {
        let cells = [
            "A long session about 日本語 and Unicode",
            "Build",
            "123",
            "$12.34",
            "Yesterday",
        ]
        .map(str::to_string);
        for width in 12..160 {
            let row = session_cols(&cells, width);
            assert_eq!(wrap::width(&row), width - 1, "{width}: {row}");
            assert!(row.starts_with("A long"), "{width}: {row}");
        }
    }

    #[test]
    fn focus_rule_runs_down_the_entire_pane() {
        let mut v = ws();
        v.focus = Focus::Inspector;
        let theme = Theme::truecolor_dark();
        let buf = crate::draw::render_buffer(&v, 160, 50, theme);
        let (_, right) = columns(160, true);
        // Below the top bar and the pane's breathing row, down to the hints.
        for y in 4..49 {
            let cell = &buf[(160 - right, y)];
            assert_eq!(cell.symbol(), glyphs::HAIRLINE_V, "row {y}");
            assert_eq!(cell.fg, theme.build, "row {y}");
        }
    }
}
