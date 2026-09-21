//! Form widget kit (`R-POP-10..18`): Toggle, Select, TextField, SecretField,
//! NumberField, ListRow, Gauge, Table.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::chat::wrap;
use crate::theme::Theme;

/// Field payload (`R-POP-10..16`). `Secret`/`Action` are part of the kit even
/// where no shipped panel uses them yet.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum Kind {
    /// `[ on ]` / `[ off ]`.
    Toggle(bool),
    /// Cycles with `←`/`→`.
    Select {
        /// Choices.
        options: Vec<String>,
        /// Current.
        idx: usize,
    },
    /// Edited via the composer.
    Text(String),
    /// Masked; only presence is shown.
    Secret {
        /// A value exists.
        set: bool,
    },
    /// `←`/`→` adjusts by `step`; typed entry validated.
    Number {
        /// Value.
        value: f64,
        /// Lower bound.
        min: f64,
        /// Upper bound.
        max: f64,
        /// Step.
        step: f64,
        /// Render without decimals.
        int: bool,
    },
    /// A row that does something on Enter.
    Action,
    /// Non-selectable group header.
    Header,
}

/// One form row.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// Stable id.
    pub id: &'static str,
    /// Label.
    pub label: String,
    /// Payload.
    pub kind: Kind,
    /// Dim right-side note (`default`, `config`, `project`) (`R-POP-48`).
    pub origin: String,
    /// Inline validation message (`R-POP-14`, `R-POP-49`).
    pub error: Option<String>,
}

impl Field {
    /// Build.
    pub fn new(id: &'static str, label: impl Into<String>, kind: Kind) -> Self {
        Self {
            id,
            label: label.into(),
            kind,
            origin: String::new(),
            error: None,
        }
    }

    /// Set the origin note.
    pub fn origin(mut self, o: impl Into<String>) -> Self {
        self.origin = o.into();
        self
    }

    fn selectable(&self) -> bool {
        !matches!(self.kind, Kind::Header)
    }

    /// Current value as text.
    pub fn value_text(&self) -> String {
        match &self.kind {
            Kind::Toggle(on) => if *on { "[ on ]" } else { "[ off ]" }.into(),
            Kind::Select { options, idx } => options.get(*idx).cloned().unwrap_or_default(),
            Kind::Text(s) => s.clone(),
            Kind::Secret { set } => if *set {
                "••••••••"
            } else {
                "(unset)"
            }
            .into(),
            Kind::Number { value, int, .. } => {
                if *int {
                    format!("{}", *value as i64)
                } else {
                    format!("{value:.2}")
                }
            }
            Kind::Action | Kind::Header => String::new(),
        }
    }
}

/// A navigable set of fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    /// Rows.
    pub fields: Vec<Field>,
    /// Focused row.
    pub selected: usize,
    /// Any edit since open (`R-POP-50`).
    pub dirty: bool,
}

impl Form {
    /// Build with focus on the first selectable row.
    pub fn new(fields: Vec<Field>) -> Self {
        let selected = fields.iter().position(Field::selectable).unwrap_or(0);
        Self {
            fields,
            selected,
            dirty: false,
        }
    }

    /// Focused field.
    pub fn current(&self) -> Option<&Field> {
        self.fields.get(self.selected)
    }

    /// Focused field, mutable.
    #[allow(dead_code)]
    pub fn current_mut(&mut self) -> Option<&mut Field> {
        self.fields.get_mut(self.selected)
    }

    /// Field by id.
    pub fn get(&self, id: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.id == id)
    }

    /// Field by id, mutable.
    #[allow(dead_code)]
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Field> {
        self.fields.iter_mut().find(|f| f.id == id)
    }

    /// Move focus by `delta`, skipping headers, wrapping.
    pub fn step(&mut self, delta: i32) {
        let n = self.fields.len();
        if n == 0 {
            return;
        }
        let mut i = self.selected;
        for _ in 0..n {
            i = (i as i32 + delta).rem_euclid(n as i32) as usize;
            if self.fields[i].selectable() {
                self.selected = i;
                return;
            }
        }
    }

    /// `Space` / `Enter` on a toggle; `←`/`→` on a select or number.
    pub fn adjust(&mut self, delta: i32) -> bool {
        let Some(f) = self.fields.get_mut(self.selected) else {
            return false;
        };
        match &mut f.kind {
            Kind::Toggle(on) => {
                *on = !*on;
            }
            Kind::Select { options, idx } => {
                if options.is_empty() {
                    return false;
                }
                *idx = (*idx as i32 + delta).rem_euclid(options.len() as i32) as usize;
            }
            Kind::Number {
                value,
                min,
                max,
                step,
                ..
            } => {
                let v = *value + *step * f64::from(delta);
                *value = v.clamp(*min, *max);
            }
            _ => return false,
        }
        f.error = None;
        self.dirty = true;
        true
    }

    /// Apply typed text to the focused field, validating numbers.
    pub fn set_from_text(&mut self, text: &str) -> bool {
        let Some(f) = self.fields.get_mut(self.selected) else {
            return false;
        };
        match &mut f.kind {
            Kind::Text(s) => {
                *s = text.trim().to_string();
                f.error = None;
                self.dirty = true;
                true
            }
            Kind::Number {
                value,
                min,
                max,
                int,
                ..
            } => match text.trim().parse::<f64>() {
                Ok(v) if v >= *min && v <= *max => {
                    *value = if *int { v.round() } else { v };
                    f.error = None;
                    self.dirty = true;
                    true
                }
                Ok(_) => {
                    f.error = Some(format!("must be between {min} and {max}"));
                    false
                }
                Err(_) => {
                    f.error = Some("not a number".into());
                    false
                }
            },
            _ => false,
        }
    }

    /// Any field with an error.
    pub fn has_errors(&self) -> bool {
        self.fields.iter().any(|f| f.error.is_some())
    }

    /// Render rows; `focused` controls the highlight.
    pub fn render(&self, width: usize, theme: Theme, focused: bool) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        let label_w = self
            .fields
            .iter()
            .filter(|f| f.selectable())
            .map(|f| wrap::width(&f.label))
            .max()
            .unwrap_or(8)
            .min(24);
        for (i, f) in self.fields.iter().enumerate() {
            if matches!(f.kind, Kind::Header) {
                if i > 0 {
                    out.push(Line::from(Span::styled("", theme.panel())));
                }
                out.push(Line::from(Span::styled(
                    format!(" {}", f.label.to_ascii_uppercase()),
                    theme.panel_muted().add_modifier(Modifier::BOLD),
                )));
                continue;
            }
            let sel = focused && i == self.selected;
            let bg = if sel {
                theme.selection_bg
            } else {
                theme.panel_bg
            };
            let base = Style::default().fg(theme.fg).bg(bg);
            let dim = Style::default().fg(theme.dim).bg(bg);
            let mark = if sel { "›" } else { " " };
            let value = f.value_text();
            let value_style = match &f.kind {
                Kind::Toggle(true) => Style::default().fg(theme.success).bg(bg),
                Kind::Toggle(false) => dim,
                Kind::Action => Style::default().fg(theme.accent).bg(bg),
                _ => base,
            };
            let mut spans = vec![
                Span::styled(mark, Style::default().fg(theme.accent).bg(bg)),
                Span::styled(
                    format!(" {}", wrap::pad_right(&f.label, label_w)),
                    if matches!(f.kind, Kind::Action) {
                        value_style
                    } else {
                        base
                    },
                ),
                Span::styled("  ", base),
            ];
            let mut used = 1 + 1 + label_w + 2;
            let arrows = matches!(f.kind, Kind::Select { .. } | Kind::Number { .. });
            let shown = if arrows {
                format!("‹ {value} ›")
            } else {
                value
            };
            let shown = wrap::truncate(
                &shown,
                width.saturating_sub(used + wrap::width(&f.origin) + 3),
            );
            used += wrap::width(&shown);
            spans.push(Span::styled(shown, value_style));
            if !f.origin.is_empty() {
                let pad = width.saturating_sub(used + wrap::width(&f.origin) + 1);
                spans.push(Span::styled(" ".repeat(pad), base));
                spans.push(Span::styled(f.origin.clone(), dim));
            }
            out.push(Line::from(spans));
            if let Some(e) = &f.error {
                out.push(Line::from(Span::styled(
                    format!("   ✕ {e}"),
                    Style::default().fg(theme.error).bg(theme.panel_bg),
                )));
            }
        }
        out
    }
}

/// `ListRow`: icon, primary, secondary dim text, right-aligned status (`R-POP-15`).
#[allow(clippy::too_many_arguments)]
pub fn list_row(
    icon: &str,
    primary: &str,
    secondary: &str,
    status: &str,
    selected: bool,
    width: usize,
    theme: Theme,
    accent: Option<Color>,
) -> Line<'static> {
    let bg = if selected {
        theme.selection_bg
    } else {
        theme.panel_bg
    };
    let base = Style::default().fg(theme.fg).bg(bg);
    let dim = Style::default().fg(theme.dim).bg(bg);
    let mark = if selected { "›" } else { " " };
    let icon_style = Style::default().fg(accent.unwrap_or(theme.fg)).bg(bg);
    let status_w = wrap::width(status);
    let mut used = 1 + wrap::width(icon) + 1;
    let room = width.saturating_sub(used + status_w + 2);
    let primary_t = wrap::truncate(primary, room);
    used += wrap::width(&primary_t);
    let mut spans = vec![
        Span::styled(mark, Style::default().fg(theme.accent).bg(bg)),
        Span::styled(icon.to_string(), icon_style),
        Span::styled(" ", base),
        Span::styled(
            primary_t,
            if selected {
                base.add_modifier(Modifier::BOLD)
            } else {
                base
            },
        ),
    ];
    if !secondary.is_empty() {
        let room = width.saturating_sub(used + status_w + 3);
        let sec = wrap::truncate(secondary, room);
        if !sec.is_empty() {
            used += 2 + wrap::width(&sec);
            spans.push(Span::styled(format!("  {sec}"), dim));
        }
    }
    let pad = width.saturating_sub(used + status_w + 1);
    spans.push(Span::styled(" ".repeat(pad), base));
    spans.push(Span::styled(status.to_string(), dim));
    spans.push(Span::styled(" ", base));
    Line::from(spans)
}

/// Threshold coloring for gauges (`R-PANEL-03`).
pub fn gauge_color(frac: f64, theme: Theme) -> Color {
    if frac >= 0.85 {
        theme.error
    } else if frac >= 0.60 {
        theme.warn
    } else {
        theme.success
    }
}

/// `Gauge`: `label ████░░░░  38%  value` (`R-POP-16`).
pub fn gauge(
    label: &str,
    frac: f64,
    value: &str,
    cells: usize,
    bg: Color,
    theme: Theme,
    color: Color,
) -> Line<'static> {
    let frac = frac.clamp(0.0, 1.0);
    let filled = (frac * cells as f64).round() as usize;
    let mut spans = Vec::new();
    if !label.is_empty() {
        spans.push(Span::styled(
            format!(" {label} "),
            Style::default().fg(theme.dim).bg(bg),
        ));
    } else {
        spans.push(Span::styled(" ", Style::default().bg(bg)));
    }
    spans.push(Span::styled(
        "█".repeat(filled),
        Style::default().fg(color).bg(bg),
    ));
    spans.push(Span::styled(
        "░".repeat(cells.saturating_sub(filled)),
        Style::default().fg(theme.gauge_track).bg(bg),
    ));
    spans.push(Span::styled(
        format!("  {value}"),
        Style::default().fg(theme.fg).bg(bg),
    ));
    Line::from(spans)
}

/// Column alignment for [`table`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Al {
    /// Left.
    L,
    /// Right.
    R,
}

/// `Table`: headers, aligned columns, optional selection (`R-POP-17`).
pub fn table(
    headers: &[&str],
    rows: &[Vec<String>],
    aligns: &[Al],
    selected: Option<usize>,
    width: usize,
    theme: Theme,
) -> Vec<Line<'static>> {
    let cols = headers.len();
    let mut widths: Vec<usize> = headers.iter().map(|h| wrap::width(h)).collect();
    for r in rows {
        for (i, c) in r.iter().enumerate().take(cols) {
            widths[i] = widths[i].max(wrap::width(c));
        }
    }
    // Shrink the widest left-aligned column until it fits.
    let frame = |w: &[usize]| w.iter().sum::<usize>() + 2 * w.len() + 1;
    while frame(&widths) > width {
        let Some((i, _)) = widths
            .iter()
            .enumerate()
            .filter(|(i, _)| aligns.get(*i) != Some(&Al::R))
            .max_by_key(|(_, w)| **w)
        else {
            break;
        };
        if widths[i] <= 3 {
            break;
        }
        widths[i] -= 1;
    }
    let cell = |s: &str, i: usize| -> String {
        let s = wrap::truncate(s, widths[i]);
        match aligns.get(i).copied().unwrap_or(Al::L) {
            Al::L => wrap::pad_right(&s, widths[i]),
            Al::R => wrap::pad_left(&s, widths[i]),
        }
    };
    let mut out = Vec::new();
    let head: String = headers
        .iter()
        .enumerate()
        .map(|(i, h)| cell(h, i))
        .collect::<Vec<_>>()
        .join("  ");
    out.push(Line::from(Span::styled(
        format!(" {head}"),
        theme.panel_muted().add_modifier(Modifier::BOLD),
    )));
    for (ri, r) in rows.iter().enumerate() {
        let sel = selected == Some(ri);
        let bg = if sel {
            theme.selection_bg
        } else {
            theme.panel_bg
        };
        let text: String = (0..cols)
            .map(|i| cell(r.get(i).map(String::as_str).unwrap_or(""), i))
            .collect::<Vec<_>>()
            .join("  ");
        out.push(Line::from(vec![
            Span::styled(
                if sel { "›" } else { " " },
                Style::default().fg(theme.accent).bg(bg),
            ),
            Span::styled(text, Style::default().fg(theme.fg).bg(bg)),
        ]));
    }
    out
}

/// Dim single-line note.
pub fn note(text: &str, theme: Theme) -> Line<'static> {
    Line::from(Span::styled(format!(" {text}"), theme.panel_muted()))
}

/// Blank panel row.
pub fn blank(theme: Theme) -> Line<'static> {
    Line::from(Span::styled("", theme.panel()))
}

/// Body text row.
pub fn text(text: &str, theme: Theme) -> Line<'static> {
    Line::from(Span::styled(format!(" {text}"), theme.panel()))
}

/// Colored text row.
pub fn colored(text: &str, color: Color, theme: Theme) -> Line<'static> {
    Line::from(Span::styled(format!(" {text}"), theme.on_panel(color)))
}

/// Wizard step indicator (`R-POP-57`).
pub fn step_line(step: usize, of: usize, title: &str, theme: Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!(" step {step} of {of}  "), theme.panel_muted()),
        Span::styled(
            title.to_string(),
            theme.panel().add_modifier(Modifier::BOLD),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_navigation_skips_headers_and_validates_numbers() {
        let mut f = Form::new(vec![
            Field::new("h", "spend", Kind::Header),
            Field::new(
                "b",
                "budget",
                Kind::Number {
                    value: 5.0,
                    min: 0.0,
                    max: 1000.0,
                    step: 1.0,
                    int: false,
                },
            ),
            Field::new("t", "auditor", Kind::Toggle(true)),
            Field::new("h2", "ui", Kind::Header),
            Field::new(
                "s",
                "theme",
                Kind::Select {
                    options: vec!["dark".into(), "light".into()],
                    idx: 0,
                },
            ),
        ]);
        assert_eq!(f.current().unwrap().id, "b");
        f.step(1);
        assert_eq!(f.current().unwrap().id, "t");
        f.step(1);
        assert_eq!(f.current().unwrap().id, "s");
        f.adjust(1);
        assert_eq!(f.current().unwrap().value_text(), "light");
        f.step(1);
        assert_eq!(f.current().unwrap().id, "b");
        assert!(!f.set_from_text("abc"));
        assert!(f.has_errors());
        assert!(!f.set_from_text("5000"));
        assert!(f.set_from_text("7.5"));
        assert!(!f.has_errors());
        assert!(f.dirty);
        let rows = f.render(60, Theme::truecolor_dark(), true);
        let joined: String = rows
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("SPEND"), "{joined}");
        assert!(joined.contains("‹ 7.50 ›"), "{joined}");
        assert!(joined.contains("[ on ]"), "{joined}");
    }

    #[test]
    fn table_fits_width() {
        let rows = vec![vec![
            "orchestrator".to_string(),
            "12".into(),
            "$0.30".into(),
        ]];
        let out = table(
            &["role", "calls", "usd"],
            &rows,
            &[Al::L, Al::R, Al::R],
            None,
            30,
            Theme::truecolor_dark(),
        );
        for l in out {
            let t: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(wrap::width(&t) <= 30, "{t}");
        }
    }
}
