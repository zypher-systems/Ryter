//! Icons and hat labels, in the terminal's own font.
//!
//! `[ui] nerd_font` swaps a few marks for Nerd Font icons. A terminal does
//! not report its font, so the default is plain Unicode; `RYTER_NERD_FONT=1`
//! turns the icons on when the key is absent. Hat names on the workspace
//! are title case (`Plan`); the stored name stays lowercase.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ryter_core::Role;

/// `Plan`, `Build`, `Audit`, `Scribe`. The stored name stays lowercase.
pub fn hat_label(hat: Role) -> String {
    let raw = crate::sidebar::hat_name(hat);
    let mut chars = raw.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// A pill: the padded text on a fill, with round caps when `round`. The
/// caps are the Powerline half-circles, the one smooth curve a cell grid
/// has; plain Unicode has no glyph that rounds a cell, so without them
/// the chip is square. `under` is the colour behind the caps.
pub fn pill(text: &str, fg: Color, fill: Color, under: Color, round: bool) -> Vec<Span<'static>> {
    let body = Span::styled(
        format!(" {text} "),
        Style::default()
            .fg(fg)
            .bg(fill)
            .add_modifier(Modifier::BOLD),
    );
    if !round {
        return vec![body];
    }
    let cap = Style::default().fg(fill).bg(under);
    vec![
        Span::styled("\u{e0b6}", cap),
        body,
        Span::styled("\u{e0b4}", cap),
    ]
}

/// Whether the chip and the tab take round caps: `[ui] caps = "round"`,
/// or a Nerd Font, which has the glyphs. `auto` was decided at startup;
/// anything else is square.
pub fn round_caps(ui: &ryter_core::config::UiConfig) -> bool {
    ui.caps == "round" || ui.nerd_font
}

/// Whether the terminal this run is in draws the Powerline glyphs itself,
/// whatever its font: kitty, WezTerm, Ghostty, iTerm2 and Windows
/// Terminal do. `env` is the environment, so a test can hand one in.
pub fn caps_terminal(env: impl Fn(&str) -> Option<String>) -> bool {
    if [
        "KITTY_WINDOW_ID",
        "WEZTERM_EXECUTABLE",
        "GHOSTTY_RESOURCES_DIR",
        "WT_SESSION",
    ]
    .iter()
    .any(|k| env(k).is_some_and(|v| !v.is_empty()))
    {
        return true;
    }
    matches!(
        env("TERM_PROGRAM").as_deref(),
        Some("WezTerm" | "ghostty" | "iTerm.app" | "kitty")
    )
}

/// The hairline between regions: dotted, so the screen is not ruled.
pub const HAIRLINE_V: &str = "┊";
/// Its horizontal twin.
pub const HAIRLINE_H: &str = "┈";
/// The turn's spine down the conversation's gutter.
pub const SPINE: &str = "┆";

/// A mark that has a plain-Unicode form and a Nerd Font form.
#[derive(Clone, Copy)]
pub enum Mark {
    Branch,
    Folder,
    File,
    Status,
    Waiting,
    Failed,
}

/// `nerd` selects the private-use icon. Folder and file have no plain mark:
/// the line stays as it was.
pub fn mark(kind: Mark, nerd: bool) -> &'static str {
    if !nerd {
        return match kind {
            Mark::Branch => "⎇",
            Mark::Folder | Mark::File => "",
            Mark::Status | Mark::Waiting => "●",
            Mark::Failed => "✗",
        };
    }
    match kind {
        Mark::Branch => "\u{e0a0}",
        Mark::Folder => "\u{f07b}",
        Mark::File => "\u{f15b}",
        Mark::Status => "\u{f111}",
        Mark::Waiting => "\u{f128}",
        Mark::Failed => "\u{f00d}",
    }
}

/// The unworn hat's pip. The hat that is on keeps the bar, so the icon is
/// not fighting the highlight.
pub fn hat_icon(hat: Role, nerd: bool) -> &'static str {
    if !nerd {
        return "●";
    }
    match hat {
        Role::Plan => "\u{f0eb}",
        Role::Build => "\u{f0ad}",
        Role::Audit => "\u{f002}",
        Role::Scribe => "\u{f040}",
        Role::Crew => "\u{f0ad}",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hat_labels_are_title_case() {
        assert_eq!(hat_label(Role::Plan), "Plan");
        assert_eq!(hat_label(Role::Build), "Build");
        assert_eq!(hat_label(Role::Audit), "Audit");
        assert_eq!(hat_label(Role::Scribe), "Scribe");
    }

    #[test]
    fn icons_fall_back_to_plain_unicode() {
        assert_eq!(mark(Mark::Branch, false), "⎇");
        assert_ne!(mark(Mark::Branch, true), "⎇");
        assert_eq!(mark(Mark::File, false), "");
        assert!(!mark(Mark::File, true).is_empty());
        assert_eq!(hat_icon(Role::Build, false), "●");
        assert_ne!(hat_icon(Role::Plan, true), hat_icon(Role::Audit, true));
    }

    #[test]
    fn the_caps_follow_the_setting_and_the_font() {
        let mut ui = ryter_core::config::UiConfig::default();
        assert_eq!(ui.caps, "auto");
        assert!(!round_caps(&ui), "auto unresolved is square");
        ui.caps = "round".into();
        assert!(round_caps(&ui));
        ui.caps = "square".into();
        assert!(!round_caps(&ui));
        ui.nerd_font = true;
        assert!(round_caps(&ui), "a Nerd Font has the glyphs");
        let round = pill("Plan", Color::Black, Color::Blue, Color::Reset, true);
        assert_eq!(round.len(), 3);
        assert_eq!(round[0].content, "\u{e0b6}");
        assert_eq!(round[2].content, "\u{e0b4}");
        assert_eq!(
            pill("Plan", Color::Black, Color::Blue, Color::Reset, false).len(),
            1
        );
    }

    #[test]
    fn the_terminal_is_told_by_its_own_variables() {
        let none = |_: &str| None;
        assert!(!caps_terminal(none));
        assert!(caps_terminal(
            |k| (k == "KITTY_WINDOW_ID").then(|| "1".into())
        ));
        assert!(caps_terminal(|k| (k == "WT_SESSION").then(|| "x".into())));
        assert!(caps_terminal(
            |k| (k == "TERM_PROGRAM").then(|| "ghostty".into())
        ));
        assert!(!caps_terminal(
            |k| (k == "TERM_PROGRAM").then(|| "Apple_Terminal".into())
        ));
        assert!(!caps_terminal(
            |k| (k == "KITTY_WINDOW_ID").then(String::new)
        ));
    }
}
