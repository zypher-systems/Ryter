//! 16-color-safe default theme plus loadable `~/.ryter/themes/*.toml`.

use std::fs;
use std::path::Path;

use ratatui::style::{Color, Style};
use ryter_core::Phase;
use serde::Deserialize;

/// Palette. Accents are used only on phase/crew labels.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// Canvas background. Always set — never rely on the terminal default.
    pub bg: Color,
    /// Right-hand column background.
    pub sidebar_bg: Color,
    /// Composer pane background.
    pub composer_bg: Color,
    /// Body text.
    pub fg: Color,
    /// Dim chrome (hairline, footer). Must stay readable on `bg`.
    pub dim: Color,
    /// Composer prompt.
    pub prompt: Color,
    /// User messages.
    pub user: Color,
    /// Tool rows.
    pub tool: Color,
    /// Errors / permission ask.
    pub warn: Color,
    /// Plan accent.
    pub plan: Color,
    /// Architect accent.
    pub architect: Color,
    /// Build accent.
    pub build: Color,
    /// Audit accent.
    pub audit: Color,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ThemeFile {
    bg: Option<String>,
    sidebar_bg: Option<String>,
    composer_bg: Option<String>,
    fg: Option<String>,
    dim: Option<String>,
    prompt: Option<String>,
    user: Option<String>,
    tool: Option<String>,
    warn: Option<String>,
    plan: Option<String>,
    architect: Option<String>,
    build: Option<String>,
    audit: Option<String>,
}

impl Theme {
    /// Default 16-color theme (explicit light-on-dark; never `Reset`).
    pub fn default_16() -> Self {
        Self {
            bg: Color::Black,
            sidebar_bg: Color::Black,
            composer_bg: Color::Black,
            fg: Color::White,
            dim: Color::Gray,
            prompt: Color::White,
            user: Color::White,
            tool: Color::Gray,
            warn: Color::Yellow,
            plan: Color::Cyan,
            architect: Color::Magenta,
            build: Color::Green,
            audit: Color::Yellow,
        }
    }

    /// Shipped truecolor dark theme. This is what the TUI starts with.
    pub fn truecolor_dark() -> Self {
        Self {
            bg: rgb(0x12, 0x12, 0x12),
            sidebar_bg: rgb(0x1a, 0x1a, 0x1a),
            composer_bg: rgb(0x1c, 0x1c, 0x1c),
            fg: rgb(0xf4, 0xf4, 0xf4),
            dim: rgb(0xb0, 0xb0, 0xb0),
            prompt: rgb(0xff, 0xff, 0xff),
            user: rgb(0x9e, 0xd6, 0xff),
            tool: rgb(0xb0, 0xb0, 0xb0),
            warn: rgb(0xe6, 0xc1, 0x4d),
            plan: rgb(0x5e, 0xc8, 0xe6),
            architect: rgb(0xc4, 0x8a, 0xe6),
            build: rgb(0x7d, 0xcc, 0x7d),
            audit: rgb(0xe6, 0xc1, 0x4d),
        }
    }

    /// Load `name` from `home/themes/<name>.toml`, else a built-in.
    pub fn load_named(home: &Path, name: &str) -> Result<Self, String> {
        let file = home.join("themes").join(format!("{name}.toml"));
        if file.is_file() {
            return Self::from_file(&file);
        }
        match name {
            "default-16" | "default" => Ok(Self::default_16()),
            "dark" => Ok(Self::truecolor_dark()),
            other => Err(format!(
                "unknown theme {other} (try default-16, dark, or ~/.ryter/themes/{other}.toml)"
            )),
        }
    }

    /// Parse a theme TOML file.
    pub fn from_file(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::from_toml(&text)
    }

    /// Parse theme TOML from a string (tests).
    pub fn from_toml(text: &str) -> Result<Self, String> {
        let file: ThemeFile = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut t = Self::truecolor_dark();
        if let Some(s) = &file.bg {
            t.bg = parse_color(s)?;
        }
        if let Some(s) = &file.sidebar_bg {
            t.sidebar_bg = parse_color(s)?;
        }
        if let Some(s) = &file.composer_bg {
            t.composer_bg = parse_color(s)?;
        }
        if let Some(s) = &file.fg {
            t.fg = parse_color(s)?;
        }
        if let Some(s) = &file.dim {
            t.dim = parse_color(s)?;
        }
        if let Some(s) = &file.prompt {
            t.prompt = parse_color(s)?;
        }
        if let Some(s) = &file.user {
            t.user = parse_color(s)?;
        }
        if let Some(s) = &file.tool {
            t.tool = parse_color(s)?;
        }
        if let Some(s) = &file.warn {
            t.warn = parse_color(s)?;
        }
        if let Some(s) = &file.plan {
            t.plan = parse_color(s)?;
        }
        if let Some(s) = &file.architect {
            t.architect = parse_color(s)?;
        }
        if let Some(s) = &file.build {
            t.build = parse_color(s)?;
        }
        if let Some(s) = &file.audit {
            t.audit = parse_color(s)?;
        }
        Ok(t)
    }

    /// Built-ins plus `home/themes/*.toml` stems.
    pub fn list(home: &Path) -> Vec<String> {
        let mut v = vec!["default-16".into(), "dark".into()];
        if let Ok(rd) = fs::read_dir(home.join("themes")) {
            for ent in rd.flatten() {
                let p = ent.path();
                if p.extension().and_then(|e| e.to_str()) != Some("toml") {
                    continue;
                }
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    if !v.iter().any(|x| x == stem) {
                        v.push(stem.to_string());
                    }
                }
            }
        }
        v
    }

    /// Accent for a phase / specialist kind.
    pub fn phase(self, phase: Phase) -> Color {
        match phase {
            Phase::Plan => self.plan,
            Phase::Architect => self.architect,
            Phase::Build => self.build,
            Phase::Audit => self.audit,
        }
    }

    /// Style for body text.
    pub fn body(self) -> Style {
        Style::default().fg(self.fg).bg(self.bg)
    }

    /// Dim chrome.
    pub fn muted(self) -> Style {
        Style::default().fg(self.dim).bg(self.bg)
    }

    /// Sidebar body.
    pub fn side(self) -> Style {
        Style::default().fg(self.fg).bg(self.sidebar_bg)
    }

    /// Sidebar muted.
    pub fn side_muted(self) -> Style {
        Style::default().fg(self.dim).bg(self.sidebar_bg)
    }

    /// Composer body.
    pub fn compose(self) -> Style {
        Style::default().fg(self.fg).bg(self.composer_bg)
    }
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

fn parse_color(s: &str) -> Result<Color, String> {
    let s = s.trim().trim_matches('"');
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).map_err(|e| e.to_string())?;
            let g = u8::from_str_radix(&hex[2..4], 16).map_err(|e| e.to_string())?;
            let b = u8::from_str_radix(&hex[4..6], 16).map_err(|e| e.to_string())?;
            return Ok(Color::Rgb(r, g, b));
        }
        return Err(format!("invalid hex color {s}"));
    }
    Ok(match s.to_ascii_lowercase().as_str() {
        "reset" => Color::Reset,
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "darkgray" | "darkgrey" => Color::DarkGray,
        "gray" | "grey" => Color::Gray,
        "lightred" => Color::LightRed,
        "lightgreen" => Color::LightGreen,
        "lightyellow" => Color::LightYellow,
        "lightblue" => Color::LightBlue,
        "lightmagenta" => Color::LightMagenta,
        "lightcyan" => Color::LightCyan,
        other => return Err(format!("unknown color {other}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_and_hex() {
        let t = Theme::from_toml(
            "fg = \"#e8e8e8\"\ndim = \"darkgray\"\nbuild = \"green\"\nplan = \"#5ec8e6\"\n",
        )
        .unwrap();
        assert_eq!(t.fg, Color::Rgb(0xe8, 0xe8, 0xe8));
        assert_eq!(t.dim, Color::DarkGray);
        assert_eq!(t.build, Color::Green);
        assert_eq!(t.plan, Color::Rgb(0x5e, 0xc8, 0xe6));
    }

    #[test]
    fn load_named_builtins() {
        let t = Theme::load_named(Path::new("/no/such/home"), "dark").unwrap();
        assert!(matches!(t.fg, Color::Rgb(_, _, _)));
        assert!(Theme::load_named(Path::new("/no/such/home"), "nope").is_err());
    }
}
