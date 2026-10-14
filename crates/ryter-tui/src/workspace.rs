//! The workspace screen: a hat rack, three views, and the instruments.
//!
//! This is the default layout (`[ui] layout = "workspace"`). RYTER sits on
//! the top bar, with Conversation, Hats, and Sessions beside it. The rack
//! is the sidebar. The conversation, the hats, or the sessions fill the
//! middle, and activity, context, spend (session + project glance), files,
//! and the plan sit on the right. Ledger and classic remain as legacy
//! layouts; prefer workspace.

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
use crate::{composer, draw, palette, panel};

/// Paint one frame of the workspace. The workbench keeps the ledger's frame.
pub fn draw(frame: &mut Frame, view: &View, theme: Theme) -> draw::Hit {
    if view.workbench.is_some() {
        return draw::draw_ledger(frame, view, theme);
    }
    let full = frame.area();
    let mut clicks = Vec::new();
    // Quiet chrome (E): no hairline under the top bar — body starts next.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .split(full);
    let (top, body, hint) = (rows[0], rows[1], rows[2]);
    fill(frame, full, theme.bg);
    draw_top(frame, top, view, theme, &mut clicks);
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
    if side_w > 0 {
        draw_sidebar(frame, side, view, theme, &mut clicks);
    }
    let (chat, composer_rect, slot) = draw_stage(frame, stage, view, theme);
    if insp_w > 0 {
        draw_inspector(frame, insp, view, theme);
    }
    draw_hint(frame, hint, view, theme);
    if view.panels.is_empty() {
        palette::draw(frame, stage, composer_rect.y, view, theme);
    }
    let cursor = panel::draw(frame, full, stage, stage, slot, view, theme);
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
        cards: Vec::new(),
        activity: Rect::default(),
        composer: composer_rect,
        clicks,
    }
}

/// `F2`, `F6`, `F7`, and `Alt+1`–`Alt+3`. `None` when the key is not one of
/// those, or this is not the workspace. A palette swallows them.
pub fn chrome_key(view: &mut View, key: KeyEvent) -> Option<Action> {
    if !view.ui.workspace() || view.workbench.is_some() {
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
    if !view.ui.workspace() || view.workbench.is_some() || view.palette.is_some() {
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
            spend: ryter_core::format_usd(s.meta.spend_usd_total),
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

fn columns(width: u16, instruments: bool) -> (u16, u16) {
    if width >= 156 {
        return (36, if instruments { 36 } else { 0 });
    }
    if width >= 120 {
        return (32, if instruments { 30 } else { 0 });
    }
    if width >= 100 {
        return if instruments { (26, 26) } else { (30, 0) };
    }
    if instruments && width >= 80 {
        return (0, 28);
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
    // Quiet chrome (E): bold RYTER only — no hat-color pips, no avatar cluster.
    // Pad + RYTER + two-column gap before the tabs (same rhythm as before).
    let mark_w = 1 + 5 + 2;
    put(
        frame,
        area,
        0,
        Line::from(vec![
            Span::styled(" ", sty(theme.fg, bg)),
            Span::styled(
                "RYTER",
                sty(theme.fg, bg).add_modifier(Modifier::BOLD),
            ),
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
    let mut x = area.x;
    let end = area.x + area.width;
    for (page, name, key, n) in pages {
        let mut label = name.to_string();
        if page == Page::Sessions && view.sessions_count > 0 {
            label.push(' ');
            label.push_str(&view.sessions_count.to_string());
        }
        let active = view.page == page;
        let (fg, fill) = if active {
            (theme.bg, theme.fg)
        } else {
            (theme.dim, bg)
        };
        let name_span = if active {
            glyphs::filled(&label, fg, fill)
        } else {
            Span::styled(format!(" {label} "), sty(fg, fill))
        };
        let key_span = glyphs::filled(key, theme.fg, theme.sticky_bg);
        let name_w = wrap::width(&name_span.content) as u16;
        let key_w = wrap::width(&key_span.content) as u16;
        let need = name_w + 1 + key_w + 2;
        if x + need > end {
            break;
        }
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                name_span,
                Span::styled(" ", sty(theme.dim, bg)),
                key_span,
            ])),
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
    if view.focus == Focus::Sidebar && area.width > 0 {
        let bar = Rect {
            x: area.x + area.width - 1,
            width: 1,
            ..area
        };
        frame.render_widget(
            Paragraph::new("┃".repeat(usize::from(area.height))).style(sty(accent, bg)),
            bar,
        );
    }
    let inner = Rect {
        x: area.x,
        width: area.width.saturating_sub(1),
        ..area
    };
    let w = usize::from(inner.width);
    let tall = inner.height >= 32;
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
            Line::from(Span::styled(" Hats", sty(theme.faint, bg))),
        );
        y += 1;
    }
    let spec_at = HATS.iter().position(|h| *h == Role::SoloAudit);
    for (i, hat) in HATS.iter().enumerate() {
        if spec_at == Some(i) && y < inner.height {
            put(
                frame,
                inner,
                y,
                Line::from(Span::styled(" ── Specialists ", sty(theme.faint, bg))),
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
        y += rows;
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
        theme.selection_bg
    } else {
        side_bg(theme)
    };
    let color = theme.mode(hat);
    let w = usize::from(area.width);
    let wearing = hat == view.mode;
    let cursor = view.focus == Focus::Sidebar && view.side_at == hat_index(hat);
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
        Span::styled(name, sty(color, bg).add_modifier(Modifier::BOLD)),
    ];
    if hat == Role::SoloAudit
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
        let meta = format!("{spend}  {}", sidebar::hat_may(hat));
        put(
            frame,
            area,
            y,
            sidebar::ends(
                name_spans,
                vec![Span::styled(meta, sty(theme.dim, bg))],
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
    let own = view
        .specialists
        .get(hat.as_str())
        .is_some_and(|m| m.is_override());
    let model = if hat == Role::SoloScribe && !own {
        "follows all hats".to_string()
    } else {
        model.to_string()
    };
    put(
        frame,
        area,
        y + 1,
        sidebar::ends(
            vec![Span::styled(
                format!("   {}", wrap::truncate(&model, w.saturating_sub(12))),
                sty(theme.dim, bg),
            )],
            vec![Span::styled(turn_s, sty(theme.faint, bg))],
            w,
            bg,
        ),
    );
    let may_color = if wearing { color } else { theme.dim };
    put(
        frame,
        area,
        y + 2,
        Line::from(Span::styled(
            format!("   {}", sidebar::hat_may(hat)),
            sty(may_color, bg),
        )),
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
        for hat in turns.iter().rev().take(room).rev() {
            spans.push(Span::styled("●", sty(theme.mode(*hat), bg)));
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
) -> (Rect, Rect, Option<Rect>) {
    fill(frame, area, theme.bg);
    match view.page {
        Page::Hats => {
            draw_hats_page(frame, area, view, theme);
            (area, Rect::default(), None)
        }
        Page::Sessions => {
            draw_sessions_page(frame, area, view, theme);
            (area, Rect::default(), None)
        }
        Page::Conversation => draw_conversation(frame, area, view, theme),
    }
}

fn draw_conversation(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    theme: Theme,
) -> (Rect, Rect, Option<Rect>) {
    let w = usize::from(area.width);
    let title = if view.session_title.trim().is_empty() {
        "New session".to_string()
    } else {
        view.session_title.clone()
    };
    let sub = subline(view);
    put(
        frame,
        area,
        0,
        sidebar::ends(
            vec![Span::styled(
                format!(" {}", wrap::truncate(&title, w.saturating_sub(24))),
                sty(theme.fg, theme.bg).add_modifier(Modifier::BOLD),
            )],
            vec![Span::styled(
                pill(view),
                sty(pill_color(view, theme), theme.bg),
            )],
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
    let prompt_h = panel::prompt_height(view, below);
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
    if chat.height > 0 {
        let _ = draw::draw_chat(frame, chat, view, theme);
    }
    let cursor = if comp.height > 0 {
        composer::draw::draw_workspace(frame, comp, view, theme)
    } else {
        None
    };
    if view.panels.is_empty() {
        composer::draw::paint_cursor(frame, cursor, theme);
    }
    (chat, comp, slot)
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
    let lede = "One conversation, four seats. Tab cycles within the mode. Shift+Tab switches mode.";
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
    if y < area.height {
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
        ("", &[Role::SoloPlan, Role::SoloBuild]),
        ("Specialists", &[Role::SoloAudit, Role::SoloScribe]),
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
            if y + 5 > area.height {
                return;
            }
            y = draw_seat(frame, area, y, *hat, view, theme);
        }
        y += 1;
    }
}

fn draw_seat(frame: &mut Frame, area: Rect, y: u16, hat: Role, view: &View, theme: Theme) -> u16 {
    let inset = Rect {
        x: area.x + 1,
        y: area.y + y,
        width: area.width.saturating_sub(2),
        height: 4,
    };
    if inset.width < 8 {
        return y + 5;
    }
    let color = theme.mode(hat);
    let wearing = hat == view.mode;
    let on = view.seat == hat || wearing;
    let bg = if on {
        theme.selection_bg
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
        for row in 1..3 {
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
    put(
        frame,
        inset,
        1,
        Line::from(vec![
            Span::styled("│ ", edge),
            Span::styled(wrap::truncate(hat.describe(), inner_w), sty(theme.fg, bg)),
        ]),
    );
    let own = view
        .specialists
        .get(hat.as_str())
        .is_some_and(|m| m.is_override());
    let follows = if own { "own model" } else { "follows all hats" };
    let model = crate::chat::short_model(view.model_of(hat));
    let meta = format!(
        "{model}  ·  {follows}   {} · {}  {}",
        view.rack.of(hat).turns,
        sidebar::hat_spend(view, hat),
        sidebar::hat_may(hat)
    );
    put(
        frame,
        inset,
        2,
        Line::from(vec![
            Span::styled("│ ", edge),
            Span::styled(wrap::truncate(&meta, inner_w), sty(theme.dim, bg)),
        ]),
    );
    put(
        frame,
        inset,
        3,
        Line::from(vec![
            Span::styled("╰", edge),
            Span::styled("─".repeat(w.saturating_sub(2)), edge),
            Span::styled("╯", edge),
        ]),
    );
    y + 5
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
                Span::styled(
                    line,
                    sty(if on { theme.fg } else { hat_color(hat, theme) }, bg),
                ),
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
        width: area.width.saturating_sub(1),
        ..area
    };
    match view.page {
        Page::Hats => draw_hat_inspector(frame, inner, view, theme),
        Page::Sessions => draw_session_inspector(frame, inner, view, theme),
        Page::Conversation => draw_conv_inspector(frame, inner, view, theme),
    }
    if view.focus == Focus::Inspector && area.width > 0 {
        let accent = theme.mode(view.mode);
        frame.render_widget(
            Paragraph::new("┃".repeat(usize::from(area.height))).style(sty(accent, bg)),
            Rect { width: 1, ..area },
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
        put(
            frame,
            area,
            y,
            Line::from(Span::styled(
                format!(" {verb}"),
                sty(pill_color(view, theme), bg),
            )),
        );
        y += 1;
        if !view.activity.current.is_empty() && y < area.height {
            let tool = if view.ui.nerd_font
                && matches!(view.activity.verb, crate::activity::Verb::Tool(_))
            {
                format!("{} ", glyphs::hat_icon(Role::SoloBuild, true))
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
        y += 1;
    }
    rule(frame, area, &mut y, theme);
    if section(frame, &mut y, "Context") && y < area.height {
        let frac = view.ctx_frac().clamp(0.0, 1.0);
        let gauge_w = w.saturating_sub(2).clamp(4, 24);
        let mut spans = vec![Span::styled(" ", sty(theme.dim, bg))];
        spans.extend(gauge_spans(
            frac,
            gauge_w,
            sidebar::gauge_color(view, theme),
            theme.code_bg,
            bg,
        ));
        put(frame, area, y, Line::from(spans));
        y += 1;
        let (used, window) = match (view.ctx_tokens, view.ctx_window) {
            (Some(u), Some(n)) if n > 0 => (short_count(u), short_count(n)),
            _ => ("—".into(), "—".into()),
        };
        let pct = view
            .ctx_pct
            .map(|p| format!("{p}%"))
            .unwrap_or_else(|| "—".into());
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
    rule(frame, area, &mut y, theme);
    let cap = sidebar::cap(view);
    if section(frame, &mut y, &format!("Spend · {cap}")) && y < area.height {
        let (text, color) = sidebar::session_spend(view, theme);
        let pct = if view.budget_usd > 0.0 {
            let spent = view.spend.unwrap_or(0.0);
            format!("{:.1}% of cap", spent / view.budget_usd * 100.0)
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
        if view.budget_usd > 0.0 && y < area.height {
            let frac = (view.spend.unwrap_or(0.0) / view.budget_usd).clamp(0.0, 1.0);
            let mut spans = vec![Span::styled(" ", sty(theme.dim, bg))];
            spans.extend(gauge_spans(
                frac,
                w.saturating_sub(2).clamp(4, 24),
                theme.mode(view.mode),
                theme.code_bg,
                bg,
            ));
            put(frame, area, y, Line::from(spans));
            y += 1;
        }
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
                    format!(" {}", glyphs::hat_label(hat)),
                    sty(theme.mode(hat), bg),
                ),
                Span::styled(" ", sty(theme.dim, bg)),
            ];
            left.extend(gauge_spans(frac, 8, theme.mode(hat), theme.code_bg, bg));
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
    rule(frame, area, &mut y, theme);
    if let Some(files) = &view.uncommitted {
        if !files.is_empty() && section(frame, &mut y, &format!("Uncommitted · {}", files.len())) {
            for f in files.iter().take(6) {
                if y >= area.height {
                    break;
                }
                put(
                    frame,
                    area,
                    y,
                    sidebar::ends(
                        vec![Span::styled(
                            file_label(&f.path, w.saturating_sub(10), view.ui.nerd_font),
                            sty(theme.fg, bg),
                        )],
                        vec![
                            Span::styled(format!("+{}", f.added), sty(theme.success, bg)),
                            Span::styled(
                                if f.removed == 0 {
                                    String::new()
                                } else {
                                    format!(" −{}", f.removed)
                                },
                                sty(theme.error, bg),
                            ),
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
    rule(frame, area, &mut y, theme);
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
        if section(frame, &mut y, "Plan") && y < area.height {
            put(
                frame,
                area,
                y,
                Line::from(Span::styled(
                    format!(" {}", wrap::truncate(label, w.saturating_sub(2))),
                    sty(theme.fg, bg),
                )),
            );
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
    let own = view
        .specialists
        .get(hat.as_str())
        .is_some_and(|m| m.is_override());
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
    let mut items: Vec<(&str, &str)> = Vec::new();
    if view.palette.is_some() {
        items.extend([("↑↓", "move"), ("enter", "run"), ("esc", "close")]);
    } else if asking {
        items.extend([
            ("y", "allow"),
            ("a", "edits"),
            ("n", "deny"),
            ("tab", "hat"),
            ("⇧tab", "mode"),
            ("/", "commands"),
            ("f2", "instruments"),
            ("f6 f7", "panes"),
        ]);
    } else {
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
    let focus_s = format!("focus · {focus}");
    let w = usize::from(area.width);
    while items.len() > 1 {
        let mut used = wrap::width(&focus_s) + 2;
        for (k, l) in &items {
            used += wrap::width(k) + wrap::width(l) + 8;
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
        spans.push(glyphs::filled(k, theme.fg, theme.sticky_bg));
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
        "plan" => Role::SoloPlan,
        "audit" => Role::SoloAudit,
        "scribe" => Role::SoloScribe,
        _ => Role::SoloBuild,
    }
}

fn hat_color(hat: Role, theme: Theme) -> ratatui::style::Color {
    theme.mode(hat)
}

fn session_cols(cells: &[String], width: usize) -> String {
    let width = width.saturating_sub(1);
    let turns_w = 6;
    let spend_w = 8;
    let when_w = 10;
    let hat_w = 10;
    let title_w = width.saturating_sub(turns_w + spend_w + when_w + hat_w + 4);
    let title_w = title_w.max(8);
    let mut out = String::new();
    let cols = [
        (
            cells.first().map(String::as_str).unwrap_or(""),
            title_w,
            false,
        ),
        (cells.get(1).map(String::as_str).unwrap_or(""), hat_w, false),
        (
            cells.get(2).map(String::as_str).unwrap_or(""),
            turns_w,
            true,
        ),
        (
            cells.get(3).map(String::as_str).unwrap_or(""),
            spend_w,
            true,
        ),
        (cells.get(4).map(String::as_str).unwrap_or(""), when_w, true),
    ];
    for (i, (text, n, right)) in cols.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let shown = wrap::truncate(text, *n);
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

/// A bar of fractional blocks (`▏`–`█`) on a track.
fn gauge_spans(
    frac: f64,
    width: usize,
    fill: Color,
    track: Color,
    _bg: Color,
) -> Vec<Span<'static>> {
    let width = width.max(1);
    let exact = frac.clamp(0.0, 1.0) * width as f64;
    let mut full = exact.floor() as usize;
    let mut eighths = ((exact - full as f64) * 8.0).round() as usize;
    if eighths >= 8 {
        full += 1;
        eighths = 0;
    }
    let full = full.min(width);
    let mut spans = Vec::new();
    let ink = Style::default().fg(fill).bg(track);
    let rest = Style::default().fg(track).bg(track);
    if full > 0 {
        spans.push(Span::styled("█".repeat(full), ink));
    }
    let mut used = full;
    if eighths > 0 && used < width {
        let ch = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"][eighths];
        spans.push(Span::styled(ch.to_string(), ink));
        used += 1;
    }
    if used < width {
        spans.push(Span::styled(" ".repeat(width - used), rest));
    }
    spans
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
        format!(" {}", "─".repeat(w.saturating_sub(1)))
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
        let total = if p.calls > 0 && p.calls == p.unpriced_calls {
            "$?.??".into()
        } else {
            ryter_core::format_usd(Some(p.total_usd))
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
                    format!(" {}", ryter_core::format_usd(Some(p.this_month()))),
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
        let mut v = View::new("spacexai".into(), "grok-4.6".into(), "~/shop".into());
        v.ui.layout = "workspace".into();
        v.username = "Dusty".into();
        v.git_branch = Some("search-patch".into());
        v.session_title = "Empty-query fix".into();
        v.theme_name = "dark".into();
        v.ctx_tokens = Some(97_000);
        v.ctx_window = Some(256_000);
        v.ctx_pct = Some(38);
        v.budget_usd = 5.0;
        v.spend = Some(0.065);
        v.set_mode(Role::SoloBuild);
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
        v.set_mode(Role::SoloPlan);
        assert!(matches!(
            crate::run_keys_handle(&mut v, press(KeyCode::Tab, KeyModifiers::NONE)),
            Action::SetMode(Role::SoloBuild)
        ));
        v.set_mode(Role::SoloBuild);
        assert!(matches!(
            crate::run_keys_handle(&mut v, press(KeyCode::BackTab, KeyModifiers::NONE)),
            Action::SetMode(Role::SoloAudit)
        ));
        v.set_mode(Role::SoloAudit);
        assert!(matches!(
            crate::run_keys_handle(&mut v, press(KeyCode::Tab, KeyModifiers::NONE)),
            Action::SetMode(Role::SoloScribe)
        ));
        // A digit is typed, not a hat.
        v.composer.clear();
        let a = crate::run_keys_handle(&mut v, press(KeyCode::Char('1'), KeyModifiers::NONE));
        assert_eq!(a, Action::None);
        assert_eq!(v.composer.text(), "1");
        assert_eq!(v.mode, Role::SoloAudit);
    }

    #[test]
    fn sidebar_enter_wears_a_hat_and_sessions_enter_resumes() {
        let mut v = ws();
        v.focus = Focus::Sidebar;
        v.side_at = 0; // plan
        assert!(matches!(
            crate::run_keys_handle(&mut v, press(KeyCode::Enter, KeyModifiers::NONE)),
            Action::SetMode(Role::SoloPlan)
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
            .find(|(c, _)| *c == WorkspaceClick::Wear(Role::SoloPlan))
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
            Action::SetMode(Role::SoloPlan)
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
        p.add(Role::SoloBuild, "m", Some(1.24));
        p.total_usd = 4.82;
        p.calls = 40;
        p.unpriced_calls = 3;
        p.sessions = 12;
        p
    }

    #[test]
    fn quiet_chrome_drops_avatar_pips_and_top_hairline() {
        let mut v = ws();
        v.username = "dusty".into();
        let text = crate::draw::render_to_string(&v, 160, 50);
        let top = text.lines().next().unwrap();
        assert!(top.contains("RYTER"), "{top}");
        assert!(top.contains("Conversation"), "{top}");
        assert!(!top.contains('☀') && !top.contains('☾'), "{top}");
        assert!(!top.contains(" dusty") && !top.contains(" Dusty"), "{top}");
        // Hat-color pips before RYTER are gone (E).
        assert!(!top.contains("●●●"), "{top}");
        assert!(!top.trim_start().starts_with('●'), "{top}");
        // No hairline row under the top bar: row 1 is body, not ─.
        let second = text.lines().nth(1).unwrap();
        assert!(
            !second.chars().all(|c| c == '─' || c == ' '),
            "unexpected hairline under top bar: {second}"
        );
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
        assert!(squashed.contains("$4.82") && squashed.contains("total"), "{text}");
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
            insp.iter().all(|l| !l.contains("Project") || !l.contains("$0.00")),
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
}
