//! Card builders for the info panel (`R-PANEL-01..16`).

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ryter_core::{format_tokens, format_usd};

use super::{Card, CardId};
use crate::chat::{short_model, wrap};
use crate::panel::widgets::gauge_color;
use crate::theme::Theme;
use crate::view::View;

fn s(text: impl Into<String>, style: Style) -> Span<'static> {
    Span::styled(text.into(), style)
}

fn row(spans: Vec<Span<'static>>) -> Line<'static> {
    Line::from(spans)
}

/// Left label + right value, value never truncated (`R-PANEL-19`).
fn kv(label: &str, value: &str, w: usize, theme: Theme, value_style: Style) -> Line<'static> {
    let vw = wrap::width(value);
    let room = w.saturating_sub(vw + 1);
    let l = wrap::truncate(label, room);
    let pad = w.saturating_sub(wrap::width(&l) + vw);
    row(vec![
        s(l, theme.side_muted()),
        s(" ".repeat(pad), theme.side()),
        s(value, value_style),
    ])
}

fn bar(frac: f64, cells: usize, color: Color, theme: Theme) -> Vec<Span<'static>> {
    let frac = frac.clamp(0.0, 1.0);
    let filled = (frac * cells as f64).round() as usize;
    vec![
        s(
            "█".repeat(filled),
            Style::default().fg(color).bg(theme.sidebar_bg),
        ),
        s(
            "░".repeat(cells.saturating_sub(filled)),
            Style::default().fg(theme.gauge_track).bg(theme.sidebar_bg),
        ),
    ]
}

/// Bar cells for a `lbl ████░░  38%` row: `w` is the card's inner width, and
/// the 4-column label plus the 5-column ` NNN%` suffix must fit beside the bar.
fn gauge_cells(w: usize) -> usize {
    w.saturating_sub(4 + 5 + 1).clamp(4, 20)
}

/// `session` card (`R-PANEL-01`).
pub fn session(view: &View, w: usize, theme: Theme) -> Card {
    let title = if view.session_title.trim().is_empty() {
        "untitled".to_string()
    } else {
        view.session_title.clone()
    };
    // Title, then the hat. The raw session id belongs in `/sessions`.
    let (state, color) = (view.mode_label().to_string(), theme.mode(view.mode));
    let mut rows = vec![kv(
        &wrap::truncate(&title, w.saturating_sub(state.chars().count() + 2)),
        &state,
        w,
        theme,
        Style::default().fg(color).bg(theme.sidebar_bg),
    )];
    let detail_from = rows.len();
    let tools = format!("tools {}", view.perm_mode);
    rows.push(row(vec![s(
        tools,
        Style::default()
            .fg(if view.perm_mode == "always" {
                theme.warn
            } else {
                theme.dim
            })
            .bg(theme.sidebar_bg),
    )]));
    Card {
        id: CardId::Session,
        title: "session".into(),
        counter: String::new(),
        rows,
        detail_from,
    }
}

/// `model` card (`R-PANEL-02..04`).
pub fn model(view: &View, w: usize, theme: Theme) -> Card {
    let dot = if view.has_key { "●" } else { "○" };
    let dot_color = if view.has_key {
        theme.success
    } else {
        theme.warn
    };
    let model = short_model(view.hat_model()).to_string();
    let mut rows = Vec::new();
    let conn_w = w.saturating_sub(wrap::width(&model) + 3);
    let conn = wrap::truncate(&view.connection, conn_w);
    let pad = w.saturating_sub(wrap::width(&conn) + 2 + wrap::width(&model));
    rows.push(row(vec![
        s(conn, theme.side()),
        s(" ", theme.side()),
        s(dot, Style::default().fg(dot_color).bg(theme.sidebar_bg)),
        s(" ".repeat(pad), theme.side()),
        s(model, theme.side().add_modifier(Modifier::BOLD)),
    ]));
    let frac = view.ctx_frac();
    let pct = (frac * 100.0).round() as u32;
    let color = gauge_color(frac, theme);
    let cells = gauge_cells(w);
    let mut spans = vec![s("ctx ", theme.side_muted())];
    spans.extend(bar(frac, cells, color, theme));
    spans.push(s(
        format!(" {pct:>3}%"),
        Style::default().fg(color).bg(theme.sidebar_bg),
    ));
    rows.push(row(spans));
    let used = format_tokens(view.ctx_tokens.unwrap_or(0));
    let window = format_tokens(view.ctx_window_or_default());
    // Drop the unit word before clipping when the card is narrow.
    let long = format!("    {used} / {window} tokens");
    let text = if wrap::width(&long) <= w {
        long
    } else {
        wrap::truncate(&format!("    {used} / {window}"), w)
    };
    rows.push(row(vec![s(text, theme.side_muted())]));
    match (view.price_in, view.price_out) {
        (Some(i), Some(o)) => rows.push(row(vec![s(
            wrap::truncate(&format!("${i:.2}/M in · ${o:.2}/M out"), w),
            theme.side_muted(),
        )])),
        _ => rows.push(row(vec![s("price unknown", theme.side_muted())])),
    }
    // How hard this model reasons in this mode: the user's choice, or what
    // auto picks. Change it with Tab in /models.
    let choice = view.reasoning_label(&view.model);
    let role = view.mode;
    let shown = match choice {
        "auto" => format!("auto · {}", view.reasoning_effective(role, &view.model)),
        c => c.to_string(),
    };
    rows.push(kv("reasoning", &shown, w, theme, theme.side()));
    if frac >= 0.85 {
        rows.push(row(vec![s(
            "/compact to reclaim",
            Style::default().fg(theme.warn).bg(theme.sidebar_bg),
        )]));
    }
    let detail_from = rows.len();
    Card {
        id: CardId::Model,
        title: "model".into(),
        counter: String::new(),
        rows,
        detail_from,
    }
}

/// `spend` card (`R-PANEL-05..07`).
pub fn spend(view: &View, w: usize, theme: Theme) -> Card {
    let mut rows = Vec::new();
    let total = view.spend_label();
    let total_style = if view.budget_usd > 0.0 && view.spend.unwrap_or(0.0) >= view.budget_usd {
        Style::default()
            .fg(theme.error)
            .bg(theme.sidebar_bg)
            .add_modifier(Modifier::BOLD)
    } else if view.spend.unwrap_or(0.0) >= view.warn_usd && view.warn_usd > 0.0 {
        Style::default()
            .fg(theme.warn)
            .bg(theme.sidebar_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        theme.side().add_modifier(Modifier::BOLD)
    };
    rows.push(kv("session", &total, w, theme, total_style));
    if let Some(p) = &view.project_spend {
        rows.push(kv("project", &project_label(p), w, theme, theme.side()));
        if let Some(root) = &view.project_root {
            let root = crate::sidebar::tail(root, w.saturating_sub(5));
            rows.push(kv("  in", &root, w, theme, theme.side_muted()));
        }
    }
    let detail_from = rows.len();
    let mut by_role: Vec<(&String, &f64)> = view.spend_by_role.iter().collect();
    by_role.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
    let shown = by_role.iter().take(4);
    for (role, usd) in shown {
        rows.push(kv(
            role,
            &format_usd(Some(**usd)),
            w,
            theme,
            Style::default().fg(theme.role(role)).bg(theme.sidebar_bg),
        ));
    }
    if by_role.len() > 4 {
        let rest: f64 = by_role.iter().skip(4).map(|(_, v)| **v).sum();
        rows.push(kv(
            &format!("+{} more", by_role.len() - 4),
            &format_usd(Some(rest)),
            w,
            theme,
            theme.side_muted(),
        ));
    }
    Card {
        id: CardId::Spend,
        title: "spend".into(),
        counter: String::new(),
        rows,
        detail_from,
    }
}

/// `$14.20`, or `$14.20+` when some calls had no known price (never a
/// silent undercount).
pub fn project_label(p: &ryter_core::project::ProjectSpend) -> String {
    if p.calls > 0 && p.calls == p.unpriced_calls {
        "$?.??".into()
    } else if p.unpriced_calls > 0 {
        format!("{}+", format_usd(Some(p.total_usd)))
    } else {
        format_usd(Some(p.total_usd))
    }
}

/// `budget` card: the cap and how much of it is left, or that there is none.
/// Clicking it opens `/budget`.
pub fn budget(view: &View, w: usize, theme: Theme) -> Card {
    let mut rows = Vec::new();
    let counter: String;
    if view.budget_usd <= 0.0 {
        // Say it: no gauge could mean "no cap" or "not loaded yet".
        rows.push(kv("cap", "off", w, theme, theme.side_muted()));
        return Card {
            id: CardId::Budget,
            title: "budget".into(),
            counter: "off".into(),
            rows,
            detail_from: 1,
        };
    }
    let cells = gauge_cells(w);
    match view.spend {
        Some(spent) => {
            // An unpriced turn has no known cost, so there is no honest bar:
            // see the `None` arm.
            let frac = spent / view.budget_usd;
            let color = if spent >= view.budget_usd {
                theme.error
            } else if spent >= view.warn_usd && view.warn_usd > 0.0 {
                theme.warn
            } else {
                theme.success
            };
            counter = format!("{}%", (frac * 100.0).round().min(999.0) as u32);
            let mut spans = vec![s("    ", theme.side())];
            spans.extend(bar(frac, cells, color, theme));
            rows.push(row(spans));
            rows.push(kv(
                "used",
                &format!(
                    "{} of {}",
                    format_usd(Some(spent)),
                    format_usd(Some(view.budget_usd))
                ),
                w,
                theme,
                Style::default().fg(color).bg(theme.sidebar_bg),
            ));
            if spent >= view.budget_usd {
                rows.push(kv(
                    "reached",
                    "/budget +2",
                    w,
                    theme,
                    Style::default().fg(theme.error).bg(theme.sidebar_bg),
                ));
            } else {
                rows.push(kv(
                    "left",
                    &format_usd(Some(view.budget_usd - spent)),
                    w,
                    theme,
                    theme.side(),
                ));
            }
        }
        None => {
            counter = "?%".into();
            let mut spans = vec![s("    ", theme.side())];
            spans.extend(bar(0.0, cells, theme.dim, theme));
            rows.push(row(spans));
            rows.push(kv(
                "used",
                &format!(
                    "{} of {}",
                    format_usd(None),
                    format_usd(Some(view.budget_usd))
                ),
                w,
                theme,
                theme.side_muted(),
            ));
        }
    }
    Card {
        id: CardId::Budget,
        title: "budget".into(),
        counter,
        rows,
        detail_from: 2,
    }
}

/// `mcp` card (`R-PANEL-16`), only when relevant.
pub fn mcp(view: &View, w: usize, theme: Theme) -> Option<Card> {
    if view.mcp_servers.is_empty() && view.mcp_listen.is_none() && view.mcp_tcp_listen.is_none() {
        return None;
    }
    let mut rows = Vec::new();
    if !view.mcp_servers.is_empty() {
        let up = view
            .mcp_status
            .values()
            .filter(|v| v.starts_with("connected"))
            .count();
        let dot_color = if up == view.mcp_servers.len() {
            theme.success
        } else if up > 0 {
            theme.warn
        } else {
            theme.dim
        };
        rows.push(row(vec![
            s("●", Style::default().fg(dot_color).bg(theme.sidebar_bg)),
            s(
                format!(" {up}/{} servers", view.mcp_servers.len()),
                theme.side(),
            ),
        ]));
    }
    if let Some(p) = &view.mcp_listen {
        let short = p.rsplit('/').next().unwrap_or(p);
        rows.push(row(vec![
            s("●", Style::default().fg(theme.success).bg(theme.sidebar_bg)),
            s(
                format!(" in  {}", wrap::truncate(short, w.saturating_sub(6))),
                theme.side_muted(),
            ),
        ]));
    }
    if let Some(t) = &view.mcp_tcp_listen {
        rows.push(row(vec![
            s("●", Style::default().fg(theme.success).bg(theme.sidebar_bg)),
            s(
                format!(" tcp {}", wrap::truncate(t, w.saturating_sub(6))),
                theme.side_muted(),
            ),
        ]));
    }
    Some(Card {
        id: CardId::Mcp,
        title: "mcp".into(),
        counter: String::new(),
        detail_from: rows.len(),
        rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(card: &Card) -> String {
        card.rows
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
                    + "\n"
            })
            .collect()
    }

    /// No gauge could mean "no cap" or "not loaded"; the card says which.
    #[test]
    fn the_budget_card_says_where_the_cap_stands() {
        let theme = Theme::truecolor_dark();
        let mut v = View::new("c".into(), "m".into(), "/tmp".into());
        v.spend = Some(1.25);
        v.budget_usd = 0.0;
        let off = budget(&v, 26, theme);
        assert_eq!(off.counter, "off");
        assert!(text(&off).contains("off"));
        v.budget_usd = 5.0;
        let on = budget(&v, 26, theme);
        assert_eq!(on.counter, "25%");
        assert!(
            text(&on).contains("$1.25 of $5.00") && text(&on).contains("$3.75"),
            "{}",
            text(&on)
        );
        // Reached: the card says how to raise it.
        v.spend = Some(5.5);
        assert!(text(&budget(&v, 26, theme)).contains("/budget +2"));
        // The spend card no longer carries the gauge.
        assert!(!text(&spend(&v, 26, theme)).contains("of $5.00"));
    }
}
