//! The workspace screen: the conversation and one rail.
//!
//! RYTER sits on the top bar. The conversation fills the left, and the
//! rail at the right carries the hats, the context, the spend (session +
//! project glance), the files and the plan. It is the one screen: the
//! ledger and classic layouts went on 2026-10-19, and the Hats and
//! Sessions views with their tabs on 2026-10-21 (`DECISIONS.md`).

use crossterm::event::{KeyCode, KeyEvent};
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
use crate::view::View;
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
        theme,
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
    debug_assert_eq!(side_w, 0, "the hats live on the one rail");
    let _ = side;
    // The text reads from the left edge, a gutter in from it where the
    // column has the room: the conversation keeps its seventy columns.
    let gutter = u16::from(stage.width >= 90) * 2;
    let stage = inset(Rect {
        x: stage.x + gutter,
        width: stage.width.saturating_sub(gutter),
        ..stage
    });
    let (chat, composer_rect, slot, activity) = draw_stage(frame, stage, view, theme);
    if insp_w > 0 {
        fill(frame, insp, side_bg(theme));
        draw_inspector(frame, inset(insp), view, theme, &mut clicks);
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

/// `F2`, which hides and shows the rail. `None` when the key is not that,
/// or this is not the workspace. A palette swallows it.
pub fn chrome_key(view: &mut View, key: KeyEvent) -> Option<Action> {
    if key.code != KeyCode::F(2) || !key.modifiers.is_empty() {
        return None;
    }
    if view.palette.is_some() {
        return Some(Action::None);
    }
    view.panel_visible = !view.panel_visible;
    Some(Action::None)
}

/// A click on a hat row of the rail: the hat goes on in place.
pub fn click(_view: &mut View, kind: WorkspaceClick) -> Action {
    match kind {
        WorkspaceClick::Wear(role) => Action::SetMode(role),
    }
}

pub(crate) fn columns(width: u16, instruments: bool) -> (u16, u16) {
    // One rail, at the right, from 100 columns: 28 wide there, growing to
    // 44 by 132, where the changes list fits whole paths and the spend
    // rows their amounts. The conversation keeps at least 70 columns.
    // There is no left rail: the hats are the rail's first block.
    if width < 100 || !instruments {
        return (0, 0);
    }
    (0, width.saturating_sub(88).clamp(28, 44))
}

fn draw_top(frame: &mut Frame, area: Rect, theme: Theme) {
    let bg = side_bg(theme);
    fill(frame, area, bg);
    if area.height == 0 {
        return;
    }
    // Brand mark: three hat-color dots + bold RYTER, and nothing else.
    // Quiet chrome drops the top-right avatar/theme cluster and the
    // hairline under the bar; the view tabs went with the views.
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
}

/// The rail's first block: the project and its branch, then the hats,
/// the one that is on with its three rows and the others on one row each
/// with their model and their cost, then the turns and the plan and audit
/// files. Returns the row after it.
fn draw_hats_block(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    theme: Theme,
    clicks: &mut Vec<(WorkspaceClick, Rect)>,
) -> u16 {
    let bg = side_bg(theme);
    let w = usize::from(area.width);
    let mut y = 0u16;
    let folder = folder_name(view);
    let folder_icon = glyphs::mark(Mark::Folder, view.ui.nerd_font);
    let branch = view.git_branch.as_deref().unwrap_or("");
    let branch_icon = glyphs::mark(Mark::Branch, view.ui.nerd_font);
    let mut head = vec![Span::styled(
        if folder_icon.is_empty() {
            format!(" {}", wrap::truncate(&folder, w.saturating_sub(2)))
        } else {
            format!(
                " {folder_icon} {}",
                wrap::truncate(&folder, w.saturating_sub(5))
            )
        },
        sty(theme.fg, bg).add_modifier(Modifier::BOLD),
    )];
    if !branch.is_empty() {
        let used = wrap::width(&head[0].content);
        head.push(Span::styled(
            format!(
                "  {branch_icon} {}",
                wrap::truncate(
                    branch,
                    w.saturating_sub(used + wrap::width(branch_icon) + 4)
                )
            ),
            sty(theme.dim, bg),
        ));
    }
    put(frame, area, y, Line::from(head));
    y += 2;
    if y >= area.height {
        return y;
    }
    put(
        frame,
        area,
        y,
        Line::from(Span::styled(" HATS", sty(theme.dim, bg))),
    );
    y += 1;
    for hat in HATS {
        let worn = hat == view.mode;
        let rows = if worn { 3 } else { 1 };
        if y + rows > area.height {
            break;
        }
        if worn {
            draw_hat(frame, area, y, hat, view, theme, true);
        } else {
            draw_hat_compact(frame, area, y, hat, view, theme);
        }
        clicks.push((
            WorkspaceClick::Wear(hat),
            Rect {
                y: area.y + y,
                height: rows,
                ..area
            },
        ));
        y += rows;
    }
    let y = draw_side_foot(frame, area, y, view, theme);
    (y + 1).min(area.height)
}

/// A hat that is not on: its pip, its name, its model and its cost on one
/// row, after the pad column the worn hat's rows keep too.
fn draw_hat_compact(frame: &mut Frame, area: Rect, y: u16, hat: Role, view: &View, theme: Theme) {
    let bg = side_bg(theme);
    let w = usize::from(area.width);
    let color = theme.mode(hat);
    let name = glyphs::hat_label(hat);
    let spend = sidebar::hat_spend(view, hat);
    let model = crate::chat::short_model(view.model_of(hat));
    let own = view.hats.get(hat.as_str()).is_some_and(|m| m.is_override());
    let model = if hat == Role::Scribe && !own {
        "follows all hats".to_string()
    } else {
        model.to_string()
    };
    let pip = format!(" {} ", glyphs::hat_icon(hat, view.ui.nerd_font));
    let label = format!("{name:<7}");
    let room = w.saturating_sub(wrap::width(&pip) + wrap::width(&label) + wrap::width(&spend) + 2);
    let left = vec![
        Span::styled(pip, sty(color, bg)),
        Span::styled(label, sty(theme.fg, bg).add_modifier(Modifier::BOLD)),
        Span::styled(wrap::truncate(&model, room), sty(theme.dim, bg)),
    ];
    put(
        frame,
        area,
        y,
        sidebar::ends(left, vec![Span::styled(spend, sty(theme.dim, bg))], w, bg),
    );
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
    // No tint behind the hat that is on: a block of three rows has no
    // round in a cell grid, and the user asked for nothing square. The
    // bar and the bold name in the hat's colour mark it.
    let bg = side_bg(theme);
    let color = theme.mode(hat);
    let w = usize::from(area.width);
    let wearing = hat == view.mode;
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
    let mut name_spans = vec![
        Span::styled(format!(" {pip} "), sty(color, bg)),
        Span::styled(
            name,
            sty(if wearing { color } else { theme.fg }, bg).add_modifier(Modifier::BOLD),
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

/// The turns row and the plan and audit files, after a blank row.
/// Returns the row after them.
fn draw_side_foot(frame: &mut Frame, area: Rect, mut y: u16, view: &View, theme: Theme) -> u16 {
    let bg = side_bg(theme);
    let w = usize::from(area.width);
    if y + 2 >= area.height {
        return y;
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
    y
}

fn draw_stage(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    theme: Theme,
) -> (Rect, Rect, Option<Rect>, Rect) {
    fill(frame, area, theme.bg);
    draw_conversation(frame, area, view, theme)
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
        // A session with nothing said yet. A system line at its head (a
        // config warning, `/new`'s note) is not content; the welcome sits
        // under whatever rows such lines took.
        if !view.has_content() {
            draw_welcome(frame, chat, cf.doc_rows, view, theme);
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

fn draw_welcome(frame: &mut Frame, area: Rect, taken: usize, view: &View, theme: Theme) {
    if area.width < 30 || area.height < 6 {
        return;
    }
    let width = area.width.saturating_sub(6).min(58);
    // Below the rows the transcript's system lines took, else a fifth of
    // the way down.
    let top = ((area.height / 5).min(4)).max(u16::try_from(taken + 1).unwrap_or(u16::MAX));
    if top + 3 > area.height {
        return;
    }
    let inner = Rect {
        x: area.x + 3,
        y: area.y + top,
        width,
        height: area.height.saturating_sub(top),
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
        let mut keys = vec![
            Span::styled(
                "/ ",
                sty(theme.mode(view.mode), theme.bg).add_modifier(Modifier::BOLD),
            ),
            Span::styled("commands", sty(theme.dim, theme.bg)),
        ];
        // Only a project with a session to go back to is told how.
        if view.sessions_count > 0 {
            keys.push(Span::styled("    /sessions ", sty(theme.fg, theme.bg)));
            keys.push(Span::styled("resume one", sty(theme.dim, theme.bg)));
        }
        put(frame, inner, y + 1, Line::from(keys));
    }
}

fn draw_inspector(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    theme: Theme,
    clicks: &mut Vec<(WorkspaceClick, Rect)>,
) {
    let bg = side_bg(theme);
    fill(frame, area, bg);
    // One column of pad lives in the strings, after the hairline. The
    // right column stays empty.
    let inner = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(3),
        ..area
    };
    draw_conv_inspector(frame, inner, view, theme, clicks);
    if area.width > 0 {
        vertical_rule(frame, Rect { width: 1, ..area }, theme.rule, bg);
    }
}

fn draw_conv_inspector(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    theme: Theme,
    clicks: &mut Vec<(WorkspaceClick, Rect)>,
) {
    let bg = side_bg(theme);
    let w = usize::from(area.width);
    let mut y = draw_hats_block(frame, area, view, theme, clicks);
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
        ]);
    } else {
        items.extend([
            ("tab", "hat"),
            ("⇧tab", "mode"),
            ("/", "commands"),
            ("f2", "instruments"),
        ]);
    }
    // With the rail folded away, the hat and the spend sit at the right;
    // with it up, they are on the rail and nothing is said twice.
    let focus_s = if columns(area.width, view.panel_visible).1 == 0 {
        format!("{} · {}", glyphs::hat_label(view.mode), view.spend_label())
    } else {
        String::new()
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
    fn f2_hides_and_shows_the_rail_and_the_old_chrome_keys_are_typed() {
        let mut v = ws();
        v.screen.set((160, 50));
        assert!(v.panel_visible);
        crate::run_keys_handle(&mut v, press(KeyCode::F(2), KeyModifiers::NONE));
        assert!(!v.panel_visible);
        crate::run_keys_handle(&mut v, press(KeyCode::F(2), KeyModifiers::NONE));
        assert!(v.panel_visible);
        // F6, F7 and Alt+1–3 are nobody's: the views and the rail's focus
        // went on 2026-10-21. They change nothing and type nothing.
        for key in [
            press(KeyCode::F(6), KeyModifiers::NONE),
            press(KeyCode::F(7), KeyModifiers::NONE),
            press(KeyCode::Char('2'), KeyModifiers::ALT),
            press(KeyCode::Char('3'), KeyModifiers::ALT),
        ] {
            assert_eq!(crate::run_keys_handle(&mut v, key), Action::None);
        }
        assert!(v.panel_visible);
        assert_eq!(v.composer.text(), "");
        assert_eq!(v.mode, Role::Build);
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
        assert!(!text.contains("Conversation"), "no view tabs: {text}");
        assert!(!text.contains("Sessions"), "no view tabs: {text}");
        assert!(text.contains("HATS"), "{text}");
        assert!(text.contains("Build"), "{text}");
        assert!(!text.contains("alt 1") && !text.contains("f6"), "{text}");
        assert!(!text.contains("focus ·"), "{text}");
        assert!(text.contains("tab") && text.contains("commands"), "{text}");
        assert!(text.contains(" mode"), "{text}");
        assert!(!text.contains("row"), "{text}");
        // No number badge on a hat.
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
        assert_eq!(top.trim(), "●●● RYTER", "the bar is the brand alone: {top}");
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
                .all(|(c, _)| matches!(c, WorkspaceClick::Wear(_))),
            "only hat clicks remain: {:?}",
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

    /// The rail's hairline runs the whole pane in the rule colour: there
    /// is no focus to mark since the rail stopped taking the keys.
    #[test]
    fn the_rails_hairline_runs_down_the_entire_pane() {
        let v = ws();
        let theme = Theme::truecolor_dark();
        let buf = crate::draw::render_buffer(&v, 160, 50, theme);
        let (_, right) = columns(160, true);
        // Below the top bar and the pane's breathing row, down to the hints.
        for y in 4..49 {
            let cell = &buf[(160 - right, y)];
            assert_eq!(cell.symbol(), glyphs::HAIRLINE_V, "row {y}");
            assert_eq!(cell.fg, theme.rule, "row {y}");
        }
    }

    /// `[ui] caps`: `round` and `square` decide by hand, `square` even
    /// with a Nerd Font; `auto` follows the terminal asked at startup, or
    /// the font. The file's word is never rewritten.
    #[test]
    fn the_caps_follow_the_setting_then_the_terminal_and_the_font() {
        let mut v = ws();
        assert_eq!(v.ui.caps, "auto");
        assert!(!v.round_caps(), "auto on a plain terminal is square");
        v.caps_terminal = true;
        assert!(
            v.round_caps(),
            "auto on a terminal with the glyphs is round"
        );
        v.ui.caps = "square".into();
        v.ui.nerd_font = true;
        assert!(
            !v.round_caps(),
            "square wins over the font and the terminal"
        );
        v.ui.caps = "round".into();
        v.caps_terminal = false;
        v.ui.nerd_font = false;
        assert!(v.round_caps(), "round wins over a plain terminal");
        v.ui.caps = "auto".into();
        v.ui.nerd_font = true;
        assert!(v.round_caps(), "auto with a Nerd Font is round");
        assert_eq!(v.ui.caps, "auto", "the file's word stays");
    }
}
