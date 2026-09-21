//! The one `KEYMAP` table (`R-KEY-*`). `/help` renders it; a test checks collisions.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Where a binding applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ctx {
    /// Everywhere (checked first).
    Global,
    /// Chat scroll (no panel or palette open).
    Scroll,
    /// Editing in the composer.
    Composer,
    /// Command palette open.
    Palette,
    /// A popout panel has focus.
    Panel,
}

impl Ctx {
    /// Display order and titles for `/help`.
    pub const ALL: [Ctx; 5] = [
        Ctx::Global,
        Ctx::Scroll,
        Ctx::Composer,
        Ctx::Palette,
        Ctx::Panel,
    ];

    /// Section title.
    pub fn title(self) -> &'static str {
        match self {
            Ctx::Global => "global",
            Ctx::Scroll => "chat scroll",
            Ctx::Composer => "composer",
            Ctx::Palette => "palette",
            Ctx::Panel => "panels",
        }
    }
}

/// Semantic key action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// clear composer → cancel turn → quit.
    CtrlC,
    /// Quit when the composer is empty.
    QuitIfEmpty,
    /// Open the command palette.
    OpenPalette,
    /// Toggle the reasoning pane.
    ToggleReasoning,
    /// Toggle the info panel.
    TogglePanel,
    /// Redraw.
    Redraw,
    /// Release the mouse so the terminal can select text.
    ToggleMouse,
    /// Open `/help`.
    Help,
    /// Back one level.
    Back,
    /// Scroll one viewport up.
    PageUp,
    /// Scroll one viewport down.
    PageDown,
    /// Scroll one row up.
    LineUp,
    /// Scroll one row down.
    LineDown,
    /// Top of the transcript.
    Top,
    /// Bottom (re-follow).
    Bottom,
    /// Previous turn boundary / jump to anchor.
    PrevTurn,
    /// Next turn boundary.
    NextTurn,
    /// Reasoning pane page up.
    ReasoningUp,
    /// Reasoning pane page down.
    ReasoningDown,
    /// Send.
    Send,
    /// Insert newline.
    Newline,
    /// Cursor left.
    Left,
    /// Cursor right.
    Right,
    /// Line start.
    Home,
    /// Line end.
    End,
    /// Word left.
    WordLeft,
    /// Word right.
    WordRight,
    /// Line up or history back.
    Up,
    /// Line down or history forward.
    Down,
    /// Delete word back.
    KillWord,
    /// Delete to line start.
    KillToStart,
    /// Delete to line end.
    KillToEnd,
    /// Backspace.
    Backspace,
    /// Delete under cursor.
    Delete,
    /// Palette: complete name.
    Complete,
    /// Palette / panel: run or activate.
    Activate,
    /// Palette: open the command's panel.
    OpenRow,
    /// Panel: next field.
    NextField,
    /// Panel: previous field.
    PrevField,
    /// Panel: toggle.
    Toggle,
    /// Panel: save.
    Save,
    /// Panel: show its keys.
    ShowKeys,
}

/// One binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    /// Context.
    pub ctx: Ctx,
    /// Canonical key string (`Ctrl+C`, `Shift+Up`, `PgUp`, `F1`, `Enter`).
    pub key: &'static str,
    /// What it does.
    pub action: KeyAction,
    /// One-line description for `/help`.
    pub help: &'static str,
}

const fn b(ctx: Ctx, key: &'static str, action: KeyAction, help: &'static str) -> Binding {
    Binding {
        ctx,
        key,
        action,
        help,
    }
}

/// The full binding table (§12).
pub const KEYMAP: &[Binding] = &[
    // Global
    b(
        Ctx::Global,
        "Ctrl+C",
        KeyAction::CtrlC,
        "clear composer → cancel turn → quit (double-press)",
    ),
    b(
        Ctx::Global,
        "Ctrl+D",
        KeyAction::QuitIfEmpty,
        "quit when the composer is empty",
    ),
    b(
        Ctx::Global,
        "Ctrl+P",
        KeyAction::OpenPalette,
        "open the command palette",
    ),
    b(
        Ctx::Global,
        "Ctrl+R",
        KeyAction::ToggleReasoning,
        "toggle the reasoning pane",
    ),
    b(
        Ctx::Global,
        "Ctrl+B",
        KeyAction::TogglePanel,
        "toggle the info panel",
    ),
    b(Ctx::Global, "Ctrl+L", KeyAction::Redraw, "redraw"),
    b(
        Ctx::Global,
        "Ctrl+G",
        KeyAction::ToggleMouse,
        "release/grab the mouse (release it to select and copy text)",
    ),
    b(Ctx::Global, "F1", KeyAction::Help, "open /help"),
    b(Ctx::Global, "Esc", KeyAction::Back, "back one level"),
    // Chat scroll
    b(
        Ctx::Scroll,
        "PgUp",
        KeyAction::PageUp,
        "scroll one viewport up",
    ),
    b(
        Ctx::Scroll,
        "PgDn",
        KeyAction::PageDown,
        "scroll one viewport down",
    ),
    b(
        Ctx::Scroll,
        "Shift+Up",
        KeyAction::LineUp,
        "scroll one row up",
    ),
    b(
        Ctx::Scroll,
        "Shift+Down",
        KeyAction::LineDown,
        "scroll one row down",
    ),
    b(
        Ctx::Scroll,
        "Ctrl+Home",
        KeyAction::Top,
        "top of the transcript",
    ),
    b(
        Ctx::Scroll,
        "Ctrl+End",
        KeyAction::Bottom,
        "bottom (re-engages follow)",
    ),
    b(
        Ctx::Scroll,
        "Ctrl+Up",
        KeyAction::PrevTurn,
        "previous turn / jump to the sticky message",
    ),
    b(Ctx::Scroll, "Ctrl+Down", KeyAction::NextTurn, "next turn"),
    b(
        Ctx::Scroll,
        "Alt+PgUp",
        KeyAction::ReasoningUp,
        "reasoning pane up",
    ),
    b(
        Ctx::Scroll,
        "Alt+PgDn",
        KeyAction::ReasoningDown,
        "reasoning pane down",
    ),
    // Composer
    b(Ctx::Composer, "Enter", KeyAction::Send, "send"),
    b(Ctx::Composer, "Shift+Enter", KeyAction::Newline, "newline"),
    b(Ctx::Composer, "Alt+Enter", KeyAction::Newline, "newline"),
    b(Ctx::Composer, "Left", KeyAction::Left, "move left"),
    b(Ctx::Composer, "Right", KeyAction::Right, "move right"),
    b(Ctx::Composer, "Home", KeyAction::Home, "line start"),
    b(Ctx::Composer, "End", KeyAction::End, "line end"),
    b(Ctx::Composer, "Ctrl+A", KeyAction::Home, "line start"),
    b(Ctx::Composer, "Ctrl+E", KeyAction::End, "line end"),
    b(Ctx::Composer, "Ctrl+Left", KeyAction::WordLeft, "word left"),
    b(
        Ctx::Composer,
        "Ctrl+Right",
        KeyAction::WordRight,
        "word right",
    ),
    b(
        Ctx::Composer,
        "Up",
        KeyAction::Up,
        "previous line, or prompt history when empty",
    ),
    b(
        Ctx::Composer,
        "Down",
        KeyAction::Down,
        "next line, or prompt history when empty",
    ),
    b(
        Ctx::Composer,
        "Ctrl+W",
        KeyAction::KillWord,
        "delete word back",
    ),
    b(
        Ctx::Composer,
        "Ctrl+U",
        KeyAction::KillToStart,
        "delete to line start",
    ),
    b(
        Ctx::Composer,
        "Ctrl+K",
        KeyAction::KillToEnd,
        "delete to line end",
    ),
    b(
        Ctx::Composer,
        "Backspace",
        KeyAction::Backspace,
        "delete back",
    ),
    b(Ctx::Composer, "Delete", KeyAction::Delete, "delete forward"),
    // Palette
    b(Ctx::Palette, "Up", KeyAction::Up, "move up"),
    b(Ctx::Palette, "Down", KeyAction::Down, "move down"),
    b(
        Ctx::Palette,
        "Tab",
        KeyAction::Complete,
        "complete the name",
    ),
    b(Ctx::Palette, "Enter", KeyAction::Activate, "run"),
    b(
        Ctx::Palette,
        "Right",
        KeyAction::OpenRow,
        "open the command's panel",
    ),
    b(Ctx::Palette, "Esc", KeyAction::Back, "close"),
    // Panels
    b(Ctx::Panel, "Up", KeyAction::Up, "move up"),
    b(Ctx::Panel, "Down", KeyAction::Down, "move down"),
    b(Ctx::Panel, "PgUp", KeyAction::PageUp, "page up"),
    b(Ctx::Panel, "PgDn", KeyAction::PageDown, "page down"),
    b(Ctx::Panel, "Enter", KeyAction::Activate, "activate"),
    b(Ctx::Panel, "Tab", KeyAction::NextField, "next field"),
    b(
        Ctx::Panel,
        "Shift+Tab",
        KeyAction::PrevField,
        "previous field",
    ),
    b(Ctx::Panel, "Space", KeyAction::Toggle, "toggle"),
    b(Ctx::Panel, "Left", KeyAction::Left, "cycle a select back"),
    b(
        Ctx::Panel,
        "Right",
        KeyAction::Right,
        "cycle a select forward",
    ),
    b(Ctx::Panel, "Ctrl+S", KeyAction::Save, "save"),
    b(
        Ctx::Panel,
        "?",
        KeyAction::ShowKeys,
        "show this panel's keys",
    ),
    b(Ctx::Panel, "Esc", KeyAction::Back, "back / close"),
];

/// Parse a canonical key string.
pub fn parse_key(s: &str) -> Option<(KeyCode, KeyModifiers)> {
    let mut mods = KeyModifiers::NONE;
    let mut code: Option<KeyCode> = None;
    for part in s.split('+') {
        match part {
            "Ctrl" => mods |= KeyModifiers::CONTROL,
            "Shift" => mods |= KeyModifiers::SHIFT,
            "Alt" => mods |= KeyModifiers::ALT,
            other => {
                code = Some(match other {
                    "Enter" => KeyCode::Enter,
                    "Esc" => KeyCode::Esc,
                    "Tab" => KeyCode::Tab,
                    "Space" => KeyCode::Char(' '),
                    "Backspace" => KeyCode::Backspace,
                    "Delete" => KeyCode::Delete,
                    "Up" => KeyCode::Up,
                    "Down" => KeyCode::Down,
                    "Left" => KeyCode::Left,
                    "Right" => KeyCode::Right,
                    "Home" => KeyCode::Home,
                    "End" => KeyCode::End,
                    "PgUp" => KeyCode::PageUp,
                    "PgDn" => KeyCode::PageDown,
                    f if f.starts_with('F') && f[1..].parse::<u8>().is_ok() => {
                        KeyCode::F(f[1..].parse().unwrap_or(1))
                    }
                    c if c.chars().count() == 1 => {
                        KeyCode::Char(c.chars().next()?.to_ascii_lowercase())
                    }
                    _ => return None,
                });
            }
        }
    }
    code.map(|c| (c, mods))
}

/// Normalize a terminal event for matching: `Shift+Tab` arrives as `BackTab`,
/// shifted letters carry SHIFT, and `Ctrl+letters` may be uppercase.
fn normalize(ev: KeyEvent) -> (KeyCode, KeyModifiers) {
    let mut mods = ev.modifiers & (KeyModifiers::CONTROL | KeyModifiers::SHIFT | KeyModifiers::ALT);
    let code = match ev.code {
        KeyCode::BackTab => {
            mods |= KeyModifiers::SHIFT;
            KeyCode::Tab
        }
        KeyCode::Char(c) => {
            if c == ' ' {
                mods.remove(KeyModifiers::SHIFT);
                KeyCode::Char(' ')
            } else if c == '?' {
                mods.remove(KeyModifiers::SHIFT);
                KeyCode::Char('?')
            } else if mods.contains(KeyModifiers::CONTROL) || mods.contains(KeyModifiers::ALT) {
                mods.remove(KeyModifiers::SHIFT);
                KeyCode::Char(c.to_ascii_lowercase())
            } else {
                KeyCode::Char(c)
            }
        }
        other => other,
    };
    (code, mods)
}

/// Look up a binding for `ev` in `ctx`.
pub fn lookup(ctx: Ctx, ev: KeyEvent) -> Option<KeyAction> {
    let (code, mods) = normalize(ev);
    KEYMAP
        .iter()
        .filter(|b| b.ctx == ctx)
        .find(|b| parse_key(b.key) == Some((code, mods)))
        .map(|b| b.action)
}

/// Bindings for one context, display-ready.
pub fn bindings(ctx: Ctx) -> Vec<&'static Binding> {
    KEYMAP.iter().filter(|b| b.ctx == ctx).collect()
}

/// Human display of a key string (`Ctrl+Up` → `^↑`, `Shift+Enter` → `⇧enter`).
pub fn display(key: &str) -> String {
    let mut out = String::new();
    let parts: Vec<&str> = key.split('+').collect();
    let n = parts.len();
    for (i, part) in parts.iter().enumerate() {
        let last = i + 1 == n;
        match *part {
            "Ctrl" => out.push('^'),
            "Shift" => out.push('⇧'),
            "Alt" => out.push('⌥'),
            "Up" => out.push('↑'),
            "Down" => out.push('↓'),
            "Left" => out.push('←'),
            "Right" => out.push('→'),
            "Enter" if last => out.push_str("enter"),
            "Esc" => out.push_str("esc"),
            "Tab" => out.push_str("tab"),
            "PgUp" => out.push_str("pgup"),
            "PgDn" => out.push_str("pgdn"),
            other => out.push_str(&other.to_ascii_lowercase()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn no_collisions_within_a_context() {
        for ctx in Ctx::ALL {
            let mut seen: HashSet<(KeyCode, KeyModifiers)> = HashSet::new();
            let mut actions: std::collections::HashMap<(KeyCode, KeyModifiers), KeyAction> =
                std::collections::HashMap::new();
            for b in bindings(ctx) {
                let parsed = parse_key(b.key).unwrap_or_else(|| panic!("unparsable {}", b.key));
                if let Some(prev) = actions.get(&parsed) {
                    assert_eq!(
                        *prev, b.action,
                        "{:?}: {} bound to two different actions",
                        ctx, b.key
                    );
                }
                actions.insert(parsed, b.action);
                seen.insert(parsed);
            }
            assert!(!seen.is_empty());
        }
    }

    #[test]
    fn no_terminal_critical_sequences() {
        for b in KEYMAP {
            assert!(
                !matches!(b.key, "Ctrl+Z" | "Ctrl+Q"),
                "{} shadows a terminal key",
                b.key
            );
        }
    }

    #[test]
    fn lookup_normalizes_backtab_and_case() {
        let ev = KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE);
        assert_eq!(lookup(Ctx::Panel, ev), Some(KeyAction::PrevField));
        let ev = KeyEvent::new(
            KeyCode::Char('C'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(lookup(Ctx::Global, ev), Some(KeyAction::CtrlC));
        let ev = KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT);
        assert_eq!(lookup(Ctx::Panel, ev), Some(KeyAction::ShowKeys));
        let ev = KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT);
        assert_eq!(lookup(Ctx::Scroll, ev), Some(KeyAction::LineUp));
        assert_eq!(display("Ctrl+Up"), "^↑");
        assert_eq!(display("Shift+Enter"), "⇧enter");
    }
}
