//! The columns beside the conversation (`docs/sidebar-design.md` §3, §4):
//! the sidebar at the right, and on a wide screen the rail at the left
//! that takes its header and its hats. Every row on them is a
//! measurement, an event or a name: the hats as a ledger, what the model
//! is doing now, the context gauge, what the session and the project have
//! cost, the files touched, and what the hat may do. Nothing on them
//! reads `off`, `none` or `idle` as a steady state.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ryter_core::Role;

use crate::chat::{turn_usd, wrap};
use crate::theme::Theme;
use crate::view::View;

/// The hats, in the order the sidebar lists them: the primary hats, then
/// the specialists (`docs/specialists-design.md` §2).
pub const HATS: [Role; 4] = [Role::Plan, Role::Build, Role::Audit, Role::Scribe];
/// Dots the `turns` row shows: the newest turns, oldest first.
pub const TURN_DOTS: usize = 15;
/// A hat's name as the screen says it: lowercase everywhere.
pub fn hat_name(hat: Role) -> &'static str {
    hat.hat().as_str()
}

/// What `hat` has cost this session. A call with no price is `$?.??`, and
/// a total that leaves some out says so: never a made-up `$0.00`.
pub fn hat_spend(view: &View, hat: Role) -> String {
    match view.spend_rows_role.get(hat.as_str()) {
        Some(row) if row.unpriced && row.usd <= 0.0 => "$?.??".into(),
        Some(row) if row.unpriced => format!("≥{}", turn_usd(row.usd)),
        Some(row) => turn_usd(row.usd),
        None => "$0".into(),
    }
}

/// What the hat may do, in two or three words.
pub fn hat_may(hat: Role) -> &'static str {
    match hat {
        Role::Plan => "read only",
        // What the audit changes, a checkpoint puts back.
        Role::Audit => "checkpoint",
        Role::Scribe => "docs only",
        Role::Build | Role::Crew => "asks first",
    }
}

/// The context gauge's color: the hat's, until it is full enough to warn.
pub fn gauge_color(view: &View, theme: Theme) -> Color {
    let c = crate::panel::widgets::gauge_color(view.ctx_frac(), theme);
    if c == theme.success {
        theme.mode(view.mode)
    } else {
        c
    }
}

/// The session's cost and the color it is said in: yellow at the warning,
/// red at the budget.
pub fn session_spend(view: &View, theme: Theme) -> (String, Color) {
    let text = match view.spend {
        None if !view.spend_unknown => "$0".to_string(),
        Some(s) if !view.spend_unknown => turn_usd(s),
        _ => view.spend_label(),
    };
    let color = match view.spend {
        Some(s) if view.budget_usd > 0.0 && s >= view.budget_usd => theme.error,
        Some(s) if view.warn_usd > 0.0 && s >= view.warn_usd => theme.warn,
        _ => theme.fg,
    };
    (text, color)
}

/// `no cap`, or `of $N`: what the session's spend is measured against.
pub fn cap(view: &View) -> String {
    if view.budget_usd > 0.0 {
        format!("of ${:.2}", view.budget_usd)
    } else {
        "no cap".into()
    }
}

/// `left` and `right` at the two ends of a row `w` columns wide, on `bg`.
pub(crate) fn ends(
    left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    w: usize,
    bg: Color,
) -> Line<'static> {
    let used: usize = left
        .iter()
        .chain(&right)
        .map(|s| wrap::width(&s.content))
        .sum();
    let mut spans = left;
    spans.push(Span::styled(
        " ".repeat(w.saturating_sub(used)),
        Style::default().bg(bg),
    ));
    spans.extend(right);
    Line::from(spans)
}
