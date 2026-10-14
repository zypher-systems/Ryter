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

    #[test]
    fn hat_labels_are_title_case() {
        assert_eq!(hat_label(Role::SoloPlan), "Plan");
        assert_eq!(hat_label(Role::SoloBuild), "Build");
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
