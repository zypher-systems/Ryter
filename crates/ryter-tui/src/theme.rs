//! Extended palette, derivation, degradation, and loadable `~/.ryter/themes/*.toml`.
//!
//! A theme file may set any subset of keys. Missing base keys fall back to the
//! `dark` palette; missing extended keys derive from the resolved base so a
//! one-key file still produces a coherent palette (`R-THEME-01`).

use std::fs;
use std::path::Path;

use ratatui::style::{Color, Modifier, Style};
use ryter_core::Phase;
use serde::Deserialize;

/// How many colors the terminal can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    /// 24-bit RGB.
    TrueColor,
    /// xterm 256 cube; RGB values are quantized.
    Ansi256,
    /// 16 named colors; the `default-16` palette is used.
    Ansi16,
    /// `NO_COLOR`: glyph, weight, and layout only.
    Mono,
}

impl ColorMode {
    /// Resolve from `[ui] colors` plus the environment.
    pub fn detect(setting: &str, env: impl Fn(&str) -> Option<String>) -> Self {
        if env("NO_COLOR").is_some_and(|v| !v.is_empty()) {
            return Self::Mono;
        }
        match setting.trim().to_ascii_lowercase().as_str() {
            "truecolor" | "24bit" => return Self::TrueColor,
            "256" => return Self::Ansi256,
            "16" => return Self::Ansi16,
            "mono" | "none" => return Self::Mono,
            _ => {}
        }
        let colorterm = env("COLORTERM").unwrap_or_default().to_ascii_lowercase();
        if colorterm == "truecolor" || colorterm == "24bit" {
            return Self::TrueColor;
        }
        let term = env("TERM").unwrap_or_default().to_ascii_lowercase();
        if term.contains("256color") || term.contains("direct") {
            return Self::Ansi256;
        }
        if term.is_empty()
            || term == "dumb"
            || term == "linux"
            || term == "vt100"
            || term == "vt220"
            || term == "xterm"
            || term == "screen"
            || term == "ansi"
        {
            return Self::Ansi16;
        }
        Self::Ansi256
    }
}

/// Palette. Borders and color carry meaning (ownership, grouping, state).
#[derive(Debug, Clone, Copy, PartialEq)]
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
    /// Composer prompt and cursor.
    pub prompt: Color,
    /// User speaker name and left rule.
    pub user: Color,
    /// Tool rows.
    pub tool: Color,
    /// Warnings, budget amber, detached scroll thumb.
    pub warn: Color,
    /// Plan accent.
    pub plan: Color,
    /// Architect accent.
    pub architect: Color,
    /// Build accent.
    pub build: Color,
    /// Audit accent.
    pub audit: Color,
    /// Assistant speaker name.
    pub assistant: Color,
    /// Selection, spinner, scrollbar thumb, focus.
    pub accent: Color,
    /// OK states, gauges under threshold, diff `+`.
    pub success: Color,
    /// Errors, over-budget, diff `-`.
    pub error: Color,
    /// Popout and card background.
    pub panel_bg: Color,
    /// Popout and card border.
    pub panel_border: Color,
    /// Popout title text.
    pub panel_title: Color,
    /// Highlighted row background.
    pub selection_bg: Color,
    /// Code block / inline code background.
    pub code_bg: Color,
    /// Code default foreground.
    pub code_fg: Color,
    /// Link text.
    pub link: Color,
    /// Sticky turn header background.
    pub sticky_bg: Color,
    /// Unfilled gauge cells.
    pub gauge_track: Color,
    /// Syntax: comments (italic).
    pub syn_comment: Color,
    /// Syntax: strings.
    pub syn_string: Color,
    /// Syntax: numbers and language constants.
    pub syn_number: Color,
    /// Syntax: keywords and storage (bold).
    pub syn_keyword: Color,
    /// Syntax: function names.
    pub syn_function: Color,
    /// Syntax: types and classes.
    pub syn_type: Color,
    /// Syntax: parameters and members.
    pub syn_variable: Color,
    /// Syntax: punctuation.
    pub syn_punct: Color,
    /// Syntax: tags, attributes, annotations.
    pub syn_attr: Color,
    /// Bumped on every load; part of the render-cache key (`R-THEME-02`).
    pub generation: u32,
    /// Active color mode (for tests and degradation-aware widgets).
    pub mode: ColorMode,
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
    assistant: Option<String>,
    accent: Option<String>,
    success: Option<String>,
    error: Option<String>,
    panel_bg: Option<String>,
    panel_border: Option<String>,
    panel_title: Option<String>,
    selection_bg: Option<String>,
    code_bg: Option<String>,
    code_fg: Option<String>,
    link: Option<String>,
    sticky_bg: Option<String>,
    gauge_track: Option<String>,
    syn_comment: Option<String>,
    syn_string: Option<String>,
    syn_number: Option<String>,
    syn_keyword: Option<String>,
    syn_function: Option<String>,
    syn_type: Option<String>,
    syn_variable: Option<String>,
    syn_punct: Option<String>,
    syn_attr: Option<String>,
}

/// Names of the shipped palettes.
pub const BUILTIN_THEMES: &[&str] = &["dark", "light", "default-16"];

impl Theme {
    /// Default 16-color theme (explicit light-on-dark; never `Reset`).
    pub fn default_16() -> Self {
        let base = Base {
            bg: Color::Black,
            sidebar_bg: Color::Black,
            composer_bg: Color::Black,
            fg: Color::White,
            dim: Color::Gray,
            prompt: Color::White,
            user: Color::LightBlue,
            tool: Color::Gray,
            warn: Color::Yellow,
            plan: Color::Cyan,
            architect: Color::Magenta,
            build: Color::Green,
            audit: Color::Yellow,
        };
        let mut t = derive(base, &ThemeFile::default()).unwrap_or_else(|_| unreachable());
        t.error = Color::LightRed;
        t.success = Color::LightGreen;
        t.accent = Color::LightCyan;
        t.selection_bg = Color::DarkGray;
        t.code_bg = Color::Black;
        t.panel_bg = Color::Black;
        t.sticky_bg = Color::Black;
        t.link = Color::LightBlue;
        // Six syntax slots (`R-SYN-05`): comment, string, keyword, type, function, default.
        t.syn_number = t.code_fg;
        t.syn_variable = t.code_fg;
        t.syn_punct = t.code_fg;
        t.syn_attr = t.syn_type;
        t.mode = ColorMode::Ansi16;
        t
    }

    /// Shipped truecolor dark theme. This is what the TUI starts with.
    pub fn truecolor_dark() -> Self {
        derive(dark_base(), &ThemeFile::default()).unwrap_or_else(|_| unreachable())
    }

    /// Shipped truecolor light theme.
    pub fn truecolor_light() -> Self {
        let base = Base {
            bg: rgb(0xfa, 0xfa, 0xf7),
            sidebar_bg: rgb(0xf1, 0xf1, 0xec),
            composer_bg: rgb(0xf4, 0xf4, 0xf0),
            fg: rgb(0x1e, 0x1e, 0x1e),
            dim: rgb(0x5c, 0x5c, 0x5c),
            prompt: rgb(0x00, 0x00, 0x00),
            user: rgb(0x0b, 0x5c, 0xad),
            tool: rgb(0x5c, 0x5c, 0x5c),
            warn: rgb(0x9a, 0x6b, 0x00),
            plan: rgb(0x00, 0x6e, 0x8a),
            architect: rgb(0x7a, 0x3d, 0xa8),
            build: rgb(0x1f, 0x7a, 0x2e),
            audit: rgb(0x9a, 0x6b, 0x00),
        };
        let mut t = derive(base, &ThemeFile::default()).unwrap_or_else(|_| unreachable());
        t.error = rgb(0xb3, 0x1d, 0x1d);
        t.code_bg = rgb(0xee, 0xee, 0xe9);
        t.sticky_bg = rgb(0xef, 0xef, 0xea);
        t.selection_bg = rgb(0xd6, 0xe6, 0xf2);
        t
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
            "light" => Ok(Self::truecolor_light()),
            other => Err(format!(
                "unknown theme {other} (try dark, light, default-16, or ~/.ryter/themes/{other}.toml)"
            )),
        }
    }

    /// Parse a theme TOML file.
    pub fn from_file(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::from_toml(&text)
    }

    /// Parse theme TOML from a string (tests). Base keys default to `dark`;
    /// extended keys derive from the resolved base unless set.
    pub fn from_toml(text: &str) -> Result<Self, String> {
        let file: ThemeFile = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut base = dark_base();
        set(&mut base.bg, &file.bg)?;
        set(&mut base.sidebar_bg, &file.sidebar_bg)?;
        set(&mut base.composer_bg, &file.composer_bg)?;
        set(&mut base.fg, &file.fg)?;
        set(&mut base.dim, &file.dim)?;
        set(&mut base.prompt, &file.prompt)?;
        set(&mut base.user, &file.user)?;
        set(&mut base.tool, &file.tool)?;
        set(&mut base.warn, &file.warn)?;
        set(&mut base.plan, &file.plan)?;
        set(&mut base.architect, &file.architect)?;
        set(&mut base.build, &file.build)?;
        set(&mut base.audit, &file.audit)?;
        derive(base, &file)
    }

    /// Built-ins plus `home/themes/*.toml` stems.
    pub fn list(home: &Path) -> Vec<String> {
        let mut v: Vec<String> = BUILTIN_THEMES.iter().map(|s| (*s).to_string()).collect();
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

    /// Reduce to what the terminal can show (`R-THEME-06..08`).
    pub fn degrade(self, mode: ColorMode) -> Self {
        match mode {
            ColorMode::TrueColor => Self { mode, ..self },
            ColorMode::Ansi256 => self.map_colors(quantize_256, mode),
            ColorMode::Ansi16 => Self {
                generation: self.generation,
                ..Self::default_16()
            },
            ColorMode::Mono => self.map_colors(|_| Color::Reset, mode),
        }
    }

    fn map_colors(self, f: impl Fn(Color) -> Color, mode: ColorMode) -> Self {
        Self {
            bg: f(self.bg),
            sidebar_bg: f(self.sidebar_bg),
            composer_bg: f(self.composer_bg),
            fg: f(self.fg),
            dim: f(self.dim),
            prompt: f(self.prompt),
            user: f(self.user),
            tool: f(self.tool),
            warn: f(self.warn),
            plan: f(self.plan),
            architect: f(self.architect),
            build: f(self.build),
            audit: f(self.audit),
            assistant: f(self.assistant),
            accent: f(self.accent),
            success: f(self.success),
            error: f(self.error),
            panel_bg: f(self.panel_bg),
            panel_border: f(self.panel_border),
            panel_title: f(self.panel_title),
            selection_bg: f(self.selection_bg),
            code_bg: f(self.code_bg),
            code_fg: f(self.code_fg),
            link: f(self.link),
            sticky_bg: f(self.sticky_bg),
            gauge_track: f(self.gauge_track),
            syn_comment: f(self.syn_comment),
            syn_string: f(self.syn_string),
            syn_number: f(self.syn_number),
            syn_keyword: f(self.syn_keyword),
            syn_function: f(self.syn_function),
            syn_type: f(self.syn_type),
            syn_variable: f(self.syn_variable),
            syn_punct: f(self.syn_punct),
            syn_attr: f(self.syn_attr),
            generation: self.generation,
            mode,
        }
    }

    /// Every color slot (for degradation tests).
    pub fn colors(&self) -> Vec<Color> {
        vec![
            self.bg,
            self.sidebar_bg,
            self.composer_bg,
            self.fg,
            self.dim,
            self.prompt,
            self.user,
            self.tool,
            self.warn,
            self.plan,
            self.architect,
            self.build,
            self.audit,
            self.assistant,
            self.accent,
            self.success,
            self.error,
            self.panel_bg,
            self.panel_border,
            self.panel_title,
            self.selection_bg,
            self.code_bg,
            self.code_fg,
            self.link,
            self.sticky_bg,
            self.gauge_track,
            self.syn_comment,
            self.syn_string,
            self.syn_number,
            self.syn_keyword,
            self.syn_function,
            self.syn_type,
            self.syn_variable,
            self.syn_punct,
            self.syn_attr,
        ]
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

    /// Accent for a specialist role name.
    pub fn role(self, role: &str) -> Color {
        match role {
            "planner" => self.plan,
            "architect" => self.architect,
            "auditor" => self.audit,
            _ => self.build,
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

    /// Body text in `color` on the canvas.
    pub fn on_bg(self, color: Color) -> Style {
        Style::default().fg(color).bg(self.bg)
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

    /// Popout body.
    pub fn panel(self) -> Style {
        Style::default().fg(self.fg).bg(self.panel_bg)
    }

    /// Popout muted.
    pub fn panel_muted(self) -> Style {
        Style::default().fg(self.dim).bg(self.panel_bg)
    }

    /// Popout text in `color`.
    pub fn on_panel(self, color: Color) -> Style {
        Style::default().fg(color).bg(self.panel_bg)
    }

    /// Highlighted row in a popout.
    pub fn selected(self) -> Style {
        let mut s = Style::default().fg(self.fg).bg(self.selection_bg);
        if self.mode == ColorMode::Mono {
            s = s.add_modifier(Modifier::BOLD | Modifier::REVERSED);
        } else {
            s = s.add_modifier(Modifier::BOLD);
        }
        s
    }

    /// Code block text.
    pub fn code(self) -> Style {
        Style::default().fg(self.code_fg).bg(self.code_bg)
    }
}

/// The thirteen base keys every theme resolves first.
#[derive(Debug, Clone, Copy)]
struct Base {
    bg: Color,
    sidebar_bg: Color,
    composer_bg: Color,
    fg: Color,
    dim: Color,
    prompt: Color,
    user: Color,
    tool: Color,
    warn: Color,
    plan: Color,
    architect: Color,
    build: Color,
    audit: Color,
}

fn dark_base() -> Base {
    Base {
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

fn unreachable() -> Theme {
    // `derive` only fails on unparsable color strings; the built-in bases pass none.
    panic!("built-in theme derivation cannot fail")
}

/// Fill the extended keys from `file` or derive them from `base` (`R-THEME-01`).
fn derive(base: Base, file: &ThemeFile) -> Result<Theme, String> {
    let pick = |explicit: &Option<String>, derived: Color| -> Result<Color, String> {
        match explicit {
            Some(s) => parse_color(s),
            None => Ok(derived),
        }
    };
    let assistant = pick(&file.assistant, base.fg)?;
    let accent = pick(&file.accent, base.plan)?;
    let success = pick(&file.success, base.build)?;
    let error = pick(&file.error, shift_red(base.warn))?;
    let panel_bg = pick(&file.panel_bg, base.sidebar_bg)?;
    let panel_border = pick(&file.panel_border, base.dim)?;
    let panel_title = pick(&file.panel_title, base.fg)?;
    let selection_bg = pick(&file.selection_bg, blend(panel_bg, accent, 0.20))?;
    let code_bg = pick(&file.code_bg, lighten(base.bg, 0.04))?;
    let code_fg = pick(&file.code_fg, base.fg)?;
    let link = pick(&file.link, base.plan)?;
    let sticky_bg = pick(&file.sticky_bg, lighten(base.bg, 0.03))?;
    let gauge_track = pick(&file.gauge_track, base.dim)?;
    Ok(Theme {
        bg: base.bg,
        sidebar_bg: base.sidebar_bg,
        composer_bg: base.composer_bg,
        fg: base.fg,
        dim: base.dim,
        prompt: base.prompt,
        user: base.user,
        tool: base.tool,
        warn: base.warn,
        plan: base.plan,
        architect: base.architect,
        build: base.build,
        audit: base.audit,
        assistant,
        accent,
        success,
        error,
        panel_bg,
        panel_border,
        panel_title,
        selection_bg,
        code_bg,
        code_fg,
        link,
        sticky_bg,
        gauge_track,
        syn_comment: pick(&file.syn_comment, base.dim)?,
        syn_string: pick(&file.syn_string, base.build)?,
        syn_number: pick(&file.syn_number, base.architect)?,
        syn_keyword: pick(&file.syn_keyword, base.plan)?,
        syn_function: pick(&file.syn_function, assistant)?,
        syn_type: pick(&file.syn_type, base.architect)?,
        syn_variable: pick(&file.syn_variable, base.fg)?,
        syn_punct: pick(&file.syn_punct, base.dim)?,
        syn_attr: pick(&file.syn_attr, base.audit)?,
        generation: 0,
        mode: ColorMode::TrueColor,
    })
}

fn set(slot: &mut Color, value: &Option<String>) -> Result<(), String> {
    if let Some(s) = value {
        *slot = parse_color(s)?;
    }
    Ok(())
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

/// Approximate sRGB for named ANSI colors (xterm defaults), for math and contrast.
pub fn to_rgb(c: Color) -> Option<(u8, u8, u8)> {
    Some(match c {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Black => (0x00, 0x00, 0x00),
        Color::Red => (0xcd, 0x00, 0x00),
        Color::Green => (0x00, 0xcd, 0x00),
        Color::Yellow => (0xcd, 0xcd, 0x00),
        Color::Blue => (0x00, 0x00, 0xee),
        Color::Magenta => (0xcd, 0x00, 0xcd),
        Color::Cyan => (0x00, 0xcd, 0xcd),
        Color::Gray => (0xe5, 0xe5, 0xe5),
        Color::DarkGray => (0x7f, 0x7f, 0x7f),
        Color::LightRed => (0xff, 0x00, 0x00),
        Color::LightGreen => (0x00, 0xff, 0x00),
        Color::LightYellow => (0xff, 0xff, 0x00),
        Color::LightBlue => (0x5c, 0x5c, 0xff),
        Color::LightMagenta => (0xff, 0x00, 0xff),
        Color::LightCyan => (0x00, 0xff, 0xff),
        Color::White => (0xff, 0xff, 0xff),
        Color::Indexed(i) => indexed_to_rgb(i),
        Color::Reset => return None,
    })
}

fn indexed_to_rgb(i: u8) -> (u8, u8, u8) {
    match i {
        0..=15 => {
            let named = [
                Color::Black,
                Color::Red,
                Color::Green,
                Color::Yellow,
                Color::Blue,
                Color::Magenta,
                Color::Cyan,
                Color::Gray,
                Color::DarkGray,
                Color::LightRed,
                Color::LightGreen,
                Color::LightYellow,
                Color::LightBlue,
                Color::LightMagenta,
                Color::LightCyan,
                Color::White,
            ];
            to_rgb(named[i as usize]).unwrap_or((0, 0, 0))
        }
        16..=231 => {
            let n = i - 16;
            let step = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            (step(n / 36), step((n / 6) % 6), step(n % 6))
        }
        _ => {
            let v = 8 + 10 * (i - 232);
            (v, v, v)
        }
    }
}

/// Nearest cell in the xterm 256 palette for an RGB color; named colors pass through.
pub fn quantize_256(c: Color) -> Color {
    let Color::Rgb(r, g, b) = c else {
        return c;
    };
    let cube = |v: u8| -> u8 {
        if v < 48 {
            0
        } else if v < 115 {
            1
        } else {
            ((u16::from(v) - 35) / 40).min(5) as u8
        }
    };
    let (cr, cg, cb) = (cube(r), cube(g), cube(b));
    let cube_idx = 16 + 36 * cr + 6 * cg + cb;
    let (qr, qg, qb) = indexed_to_rgb(cube_idx);
    // Grayscale ramp candidate.
    let avg = (u16::from(r) + u16::from(g) + u16::from(b)) / 3;
    let gray_i = if avg < 8 {
        16
    } else if avg > 238 {
        231
    } else {
        232 + ((avg - 8) / 10).min(23) as u8
    };
    let (gr, gg, gb) = indexed_to_rgb(gray_i);
    let d = |a: (u8, u8, u8)| -> i32 {
        let dr = i32::from(a.0) - i32::from(r);
        let dg = i32::from(a.1) - i32::from(g);
        let db = i32::from(a.2) - i32::from(b);
        dr * dr + dg * dg + db * db
    };
    if d((gr, gg, gb)) < d((qr, qg, qb)) {
        Color::Indexed(gray_i)
    } else {
        Color::Indexed(cube_idx)
    }
}

/// Darken an RGB color toward black by `amount` (0..1). Non-RGB colors pass through.
pub fn darken(c: Color, amount: f64) -> Color {
    match c {
        Color::Rgb(r, g, b) => {
            let f =
                |v: u8| -> u8 { (f64::from(v) * (1.0 - amount)).round().clamp(0.0, 255.0) as u8 };
            Color::Rgb(f(r), f(g), f(b))
        }
        _ => Color::Black,
    }
}

fn lighten(c: Color, amount: f64) -> Color {
    match to_rgb(c) {
        Some((r, g, b)) if matches!(c, Color::Rgb(..)) => {
            let f = |v: u8| -> u8 {
                let v = f64::from(v);
                (v + (255.0 - v) * amount).round().clamp(0.0, 255.0) as u8
            };
            Color::Rgb(f(r), f(g), f(b))
        }
        _ => c,
    }
}

fn blend(a: Color, b: Color, t: f64) -> Color {
    match (to_rgb(a), to_rgb(b)) {
        (Some((ar, ag, ab)), Some((br, bg, bb))) if matches!(a, Color::Rgb(..)) => {
            let m = |x: u8, y: u8| -> u8 {
                (f64::from(x) * (1.0 - t) + f64::from(y) * t)
                    .round()
                    .clamp(0.0, 255.0) as u8
            };
            Color::Rgb(m(ar, br), m(ag, bg), m(ab, bb))
        }
        _ => Color::DarkGray,
    }
}

/// Push a warning hue toward red for the `error` slot.
fn shift_red(c: Color) -> Color {
    match c {
        Color::Rgb(r, g, b) => {
            let r = r.saturating_add(20).max(0xd0);
            let g = (f64::from(g) * 0.45) as u8;
            let b = (f64::from(b) * 0.55) as u8;
            Color::Rgb(r, g, b)
        }
        Color::Yellow | Color::LightYellow => Color::LightRed,
        _ => Color::Red,
    }
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

/// WCAG relative luminance.
#[cfg(test)]
fn luminance(c: Color) -> Option<f64> {
    let (r, g, b) = to_rgb(c)?;
    let lin = |v: u8| -> f64 {
        let v = f64::from(v) / 255.0;
        if v <= 0.039_28 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    Some(0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b))
}

/// WCAG contrast ratio between two colors (`None` for `Reset`).
#[cfg(test)]
pub fn contrast_ratio(a: Color, b: Color) -> Option<f64> {
    let (la, lb) = (luminance(a)?, luminance(b)?);
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    Some((hi + 0.05) / (lo + 0.05))
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
        assert!(Theme::load_named(Path::new("/no/such/home"), "light").is_ok());
        assert!(Theme::load_named(Path::new("/no/such/home"), "default-16").is_ok());
        assert!(Theme::load_named(Path::new("/no/such/home"), "nope").is_err());
    }

    #[test]
    fn one_key_file_derives_a_full_palette() {
        let t = Theme::from_toml("plan = \"#ff0000\"\n").unwrap();
        // accent derives from plan, link from plan, keyword from plan.
        assert_eq!(t.accent, Color::Rgb(0xff, 0, 0));
        assert_eq!(t.link, Color::Rgb(0xff, 0, 0));
        assert_eq!(t.syn_keyword, Color::Rgb(0xff, 0, 0));
        // untouched derived keys are still concrete colors, never Reset.
        assert!(t.colors().iter().all(|c| *c != Color::Reset));
        // explicit extended key wins over derivation.
        let t = Theme::from_toml("accent = \"cyan\"\n").unwrap();
        assert_eq!(t.accent, Color::Cyan);
    }

    #[test]
    fn shipped_themes_meet_wcag_aa() {
        for (name, t) in [
            ("dark", Theme::truecolor_dark()),
            ("light", Theme::truecolor_light()),
            ("default-16", Theme::default_16()),
        ] {
            for (label, fg) in [("fg", t.fg), ("dim", t.dim), ("code_fg", t.code_fg)] {
                let bg = if label == "code_fg" { t.code_bg } else { t.bg };
                let ratio = contrast_ratio(fg, bg).unwrap();
                assert!(ratio >= 4.5, "{name}: {label} on bg is {ratio:.2}:1");
            }
            let ratio = contrast_ratio(t.fg, t.panel_bg).unwrap();
            assert!(ratio >= 4.5, "{name}: fg on panel_bg is {ratio:.2}:1");
        }
    }

    #[test]
    fn sixteen_color_degradation_has_no_rgb() {
        let t = Theme::truecolor_dark().degrade(ColorMode::Ansi16);
        assert!(t.colors().iter().all(|c| !matches!(c, Color::Rgb(..))));
        let t = Theme::truecolor_dark().degrade(ColorMode::Ansi256);
        assert!(t.colors().iter().all(|c| !matches!(c, Color::Rgb(..))));
        assert!(t.colors().iter().any(|c| matches!(c, Color::Indexed(_))));
        let t = Theme::truecolor_dark().degrade(ColorMode::Mono);
        assert!(t.colors().iter().all(|c| *c == Color::Reset));
    }

    #[test]
    fn quantize_hits_gray_ramp_and_cube() {
        assert_eq!(
            quantize_256(Color::Rgb(0x12, 0x12, 0x12)),
            Color::Indexed(233)
        );
        assert_eq!(
            quantize_256(Color::Rgb(0xff, 0x00, 0x00)),
            Color::Indexed(196)
        );
        assert_eq!(quantize_256(Color::Cyan), Color::Cyan);
    }

    #[test]
    fn mode_detection_prefers_no_color() {
        let env = |k: &str| match k {
            "NO_COLOR" => Some("1".to_string()),
            "COLORTERM" => Some("truecolor".to_string()),
            _ => None,
        };
        assert_eq!(ColorMode::detect("auto", env), ColorMode::Mono);
        let env = |k: &str| match k {
            "COLORTERM" => Some("truecolor".to_string()),
            _ => None,
        };
        assert_eq!(ColorMode::detect("auto", env), ColorMode::TrueColor);
        assert_eq!(ColorMode::detect("16", env), ColorMode::Ansi16);
        let env = |k: &str| match k {
            "TERM" => Some("xterm-256color".to_string()),
            _ => None,
        };
        assert_eq!(ColorMode::detect("auto", env), ColorMode::Ansi256);
        let env = |k: &str| match k {
            "TERM" => Some("linux".to_string()),
            _ => None,
        };
        assert_eq!(ColorMode::detect("auto", env), ColorMode::Ansi16);
    }
}
