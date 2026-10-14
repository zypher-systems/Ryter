//! A second face, and a few icons, without asking the terminal to change fonts.
//!
//! A cell is painted by whatever font the terminal is using. Of the ways to
//! look like another face anyway:
//!
//! * Mathematical sans-serif bold (`U+1D5D4`…) is a real second face, one
//!   cell wide, wherever the font or its fallback covers the block. Headings
//!   and the one-row wordmark use it.
//! * The two-row RYTER mark is built from `█▀▄`. Every coding font has those,
//!   so the logo does not depend on the math block.
//! * Kitty graphics, Sixel, and the kitty text-sizing protocol need one
//!   terminal and do not survive this grid being redrawn. They are not used.
//!
//! `[ui] styled_text` turns both off, back to plain `RYTER`. It defaults off
//! when `TERM` is `dumb` or `linux`, which have no math letters. `[ui]
//! nerd_font` swaps a few marks for Nerd Font icons. A terminal does not
//! report its font, so the default is plain Unicode; `RYTER_NERD_FONT=1`
//! turns the icons on when the key is absent.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ryter_core::{Role, UiConfig};

/// Two rows, nineteen columns. The top of R Y T E R, then the bottom.
pub const WORDMARK: [&str; 2] = ["█▀█ █ █ ▀█▀ █▀▀ █▀█", "█▀▄ ▀▄▀  █  █▄▄ █▀▄"];

/// Rows the brand takes. A tall screen with styled text gets the block mark.
pub fn brand_rows(ui: &UiConfig, height: u16) -> u16 {
    if ui.styled_text && height >= 28 { 2 } else { 1 }
}

/// `RYTER` in the sans face, or as itself.
pub fn wordmark_line(styled: bool) -> String {
    face("RYTER", styled)
}

/// Letters in mathematical sans-serif bold. Anything else is copied through.
pub fn face(text: &str, styled: bool) -> String {
    if !styled {
        return text.to_string();
    }
    text.chars().map(styled_char).collect()
}

fn styled_char(c: char) -> char {
    match c {
        'A'..='Z' => char::from_u32(0x1D5D4 + (c as u32 - 'A' as u32)).unwrap_or(c),
        'a'..='z' => char::from_u32(0x1D5EE + (c as u32 - 'a' as u32)).unwrap_or(c),
        _ => c,
    }
}

/// `Plan`, `Build`, `Audit`, `Scribe`. The stored name stays lowercase.
pub fn hat_label(hat: Role) -> String {
    let raw = crate::sidebar::hat_name(hat);
    let mut chars = raw.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// The hat's name in the heading face.
pub fn hat_face(hat: Role, styled: bool) -> String {
    face(&hat_label(hat), styled)
}

/// A solid chip: padded text on a fill, no half-block caps.
pub fn filled(text: &str, fg: Color, bg: Color) -> Span<'static> {
    Span::styled(
        format!(" {text} "),
        Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
    )
}

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
        Role::SoloPlan => "\u{f0eb}",
        Role::SoloBuild => "\u{f0ad}",
        Role::SoloAudit => "\u{f002}",
        Role::SoloScribe => "\u{f040}",
        Role::Crew => "\u{f0ad}",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn the_sans_face_is_one_cell_and_plain_text_is_ascii() {
        assert_eq!(face("RYTER", false), "RYTER");
        let on = face("Hats", true);
        assert_ne!(on, "Hats");
        assert_eq!(on.chars().count(), 4);
        assert_eq!(on.width(), 4, "a heading must not shift the grid");
        assert_eq!(on.chars().next(), Some('\u{1D5DB}'));
        assert_eq!(WORDMARK[0].width(), WORDMARK[1].width());
        assert_eq!(hat_label(Role::SoloPlan), "Plan");
        assert_eq!(hat_label(Role::SoloAudit), "Audit");
        assert_eq!(hat_label(Role::SoloScribe), "Scribe");
    }

    #[test]
    fn icons_fall_back_to_plain_unicode() {
        assert_eq!(mark(Mark::Branch, false), "⎇");
        assert_ne!(mark(Mark::Branch, true), "⎇");
        assert_eq!(mark(Mark::File, false), "");
        assert!(!mark(Mark::File, true).is_empty());
        assert_eq!(hat_icon(Role::SoloBuild, false), "●");
        assert_ne!(
            hat_icon(Role::SoloPlan, true),
            hat_icon(Role::SoloAudit, true)
        );
    }
}
