//! `/settings` — grouped settings form (`R-POP-47..50`).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ryter_core::UiConfig;

use super::widgets::{Field, Form, Kind};
use super::wrap;
use super::{Body, Outcome, Panel};
use crate::action::Action;
use crate::theme::Theme;
use crate::view::View;

/// Settings form.
#[derive(Debug, Clone)]
pub struct Settings {
    form: Form,
    /// Editing the focused text/number field via the composer.
    editing: bool,
    /// `Esc` with edits asks first (`R-POP-50`).
    confirm_discard: bool,
}

fn origin<T: PartialEq>(cur: &T, default: &T) -> &'static str {
    if cur == default { "default" } else { "config" }
}

#[allow(clippy::too_many_arguments)]
fn num(
    id: &'static str,
    label: &str,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
    int: bool,
    default: f64,
) -> Field {
    Field::new(
        id,
        label,
        Kind::Number {
            value,
            min,
            max,
            step,
            int,
        },
    )
    .origin(origin(&value, &default))
}

/// One specialist's step limit, with its default in the label (short: the
/// label column is 25 wide, and "architect steps (default 30)" lost its
/// number there).
fn steps_field(id: &'static str, who: &str, value: u32, step: f64) -> Field {
    use ryter_core::config::Steps;
    let d = Steps::default();
    let default = match who {
        "builder" => d.builder,
        "architect" => d.architect,
        _ => d.auditor,
    };
    num(
        id,
        &format!("{who} steps (def {default})"),
        f64::from(value),
        f64::from(Steps::MIN),
        f64::from(Steps::MAX),
        step,
        true,
        f64::from(default),
    )
}

fn select(id: &'static str, label: &str, options: &[&str], cur: &str, default: &str) -> Field {
    let idx = options.iter().position(|o| *o == cur).unwrap_or(0);
    Field::new(
        id,
        label,
        Kind::Select {
            options: options.iter().map(|s| (*s).to_string()).collect(),
            idx,
        },
    )
    .origin(origin(&cur, &default))
}

impl Settings {
    /// Build from the live view.
    pub fn new(view: &View) -> Self {
        let d = UiConfig::default();
        let mut theme_opts: Vec<&str> = view.theme_names.iter().map(String::as_str).collect();
        if theme_opts.is_empty() {
            theme_opts = vec!["dark"];
        }
        let fields = vec![
            Field::new("g_spend", "spend", Kind::Header),
            num(
                "budget",
                "budget usd (0 = off)",
                view.budget_usd,
                0.0,
                10_000.0,
                0.5,
                false,
                0.0,
            ),
            num(
                "warn",
                "warn usd",
                view.warn_usd,
                0.0,
                10_000.0,
                0.25,
                false,
                1.0,
            ),
            Field::new("g_agents", "agents", Kind::Header),
            num(
                "max",
                "subagents max",
                f64::from(view.max_crew),
                1.0,
                16.0,
                1.0,
                true,
                4.0,
            ),
            Field::new("auditor", "auditor", Kind::Toggle(view.auditor_on))
                .origin(origin(&view.auditor_on, &true)),
            // A step is one call to the specialist's model; the last is for
            // writing up. The defaults are in the labels: a crew stuck at a
            // limit is the reason to come here.
            steps_field("steps_builder", "builder", view.steps.builder, 5.0),
            steps_field("steps_architect", "architect", view.steps.architect, 5.0),
            steps_field("steps_auditor", "auditor", view.steps.auditor, 2.0),
            Field::new("g_tools", "tools", Kind::Header),
            select(
                "perm",
                "permission mode",
                &["ask", "always"],
                &view.perm_mode,
                "ask",
            ),
            Field::new("web", "features.web", Kind::Toggle(view.web))
                .origin(origin(&view.web, &false)),
            Field::new("g_sandbox", "sandbox", Kind::Header),
            select(
                "sandbox",
                "profile",
                &["off", "workspace", "read-only"],
                &view.sandbox_profile,
                "off",
            ),
            Field::new("g_mcp", "mcp", Kind::Header),
            Field::new("inbound", "inbound", Kind::Toggle(view.mcp_inbound))
                .origin(origin(&view.mcp_inbound, &true)),
            Field::new(
                "bind",
                "bind address",
                Kind::Text(view.mcp_bind.clone().unwrap_or_default()),
            )
            .origin(if view.mcp_bind.is_none() {
                "default"
            } else {
                "config"
            }),
            Field::new("g_update", "updates", Kind::Header),
            select(
                "update",
                "on launch",
                &["install", "notify", "off"],
                view.update_mode.as_str(),
                ryter_core::config::UpdateMode::default().as_str(),
            ),
            Field::new("g_ui", "ui", Kind::Header),
            select("theme", "theme", &theme_opts, &view.theme_name, &d.theme),
            select(
                "layout",
                "layout",
                &["ledger", "classic"],
                &view.ui.layout,
                &d.layout,
            ),
            Field::new("username", "username", Kind::Text(view.ui.username.clone()))
                .origin(origin(&view.ui.username, &d.username)),
            select(
                "reasoning",
                "reasoning",
                &["collapsed", "expanded", "off"],
                &view.ui.reasoning,
                &d.reasoning,
            ),
            Field::new("mouse", "mouse", Kind::Toggle(view.ui.mouse))
                .origin(origin(&view.ui.mouse, &d.mouse)),
            Field::new("panel", "panel · rail", Kind::Toggle(view.ui.panel))
                .origin(origin(&view.ui.panel, &d.panel)),
            Field::new("timestamps", "timestamps", Kind::Toggle(view.ui.timestamps))
                .origin(origin(&view.ui.timestamps, &d.timestamps)),
            Field::new(
                "line_numbers",
                "line numbers",
                Kind::Toggle(view.ui.line_numbers),
            )
            .origin(origin(&view.ui.line_numbers, &d.line_numbers)),
            Field::new(
                "offer_audit",
                "audit offers",
                Kind::Toggle(view.ui.offer_audit),
            )
            .origin(origin(&view.ui.offer_audit, &d.offer_audit)),
            Field::new(
                "open_pages",
                "pages in browser",
                Kind::Toggle(view.ui.open_pages),
            )
            .origin(origin(&view.ui.open_pages, &d.open_pages)),
        ];
        Self {
            form: Form::new(fields),
            editing: false,
            confirm_discard: false,
        }
    }

    /// Copy form values back into the view (the loop persists).
    fn apply(&self, view: &mut View) {
        let f = &self.form;
        let number = |id: &str| -> Option<f64> {
            match f.get(id).map(|x| &x.kind) {
                Some(Kind::Number { value, .. }) => Some(*value),
                _ => None,
            }
        };
        let toggle = |id: &str| -> Option<bool> {
            match f.get(id).map(|x| &x.kind) {
                Some(Kind::Toggle(b)) => Some(*b),
                _ => None,
            }
        };
        let text = |id: &str| -> Option<String> {
            match f.get(id).map(|x| &x.kind) {
                Some(Kind::Text(s)) => Some(s.clone()),
                _ => None,
            }
        };
        let sel = |id: &str| -> Option<String> { f.get(id).map(Field::value_text) };
        if let Some(v) = number("budget") {
            view.budget_usd = v;
        }
        if let Some(v) = number("warn") {
            view.warn_usd = v;
        }
        if let Some(v) = number("max") {
            view.max_crew = v.round().clamp(1.0, 16.0) as u32;
        }
        if let Some(v) = toggle("auditor") {
            view.auditor_on = v;
        }
        let steps = |id: &str, cur: u32| {
            number(id).map_or(cur, |v| {
                use ryter_core::config::Steps;
                v.round()
                    .clamp(f64::from(Steps::MIN), f64::from(Steps::MAX)) as u32
            })
        };
        view.steps = ryter_core::config::Steps {
            builder: steps("steps_builder", view.steps.builder),
            architect: steps("steps_architect", view.steps.architect),
            auditor: steps("steps_auditor", view.steps.auditor),
        };
        if let Some(v) = sel("perm") {
            view.perm_mode = v;
        }
        if let Some(v) = toggle("web") {
            view.web = v;
        }
        if let Some(v) = sel("sandbox") {
            view.sandbox_profile = v;
        }
        if let Some(v) = toggle("inbound") {
            view.mcp_inbound = v;
        }
        if let Some(v) = text("bind") {
            view.mcp_bind = (!v.trim().is_empty()).then(|| v.trim().to_string());
        }
        if let Some(v) = sel("update").and_then(|v| ryter_core::config::UpdateMode::parse(&v)) {
            view.update_mode = v;
        }
        if let Some(v) = sel("theme") {
            view.ui.theme = v;
        }
        if let Some(v) = sel("layout") {
            view.ui.layout = v;
        }
        if let Some(v) = text("username") {
            view.ui.username = v;
        }
        if let Some(v) = sel("reasoning") {
            view.ui.reasoning = v;
        }
        if let Some(v) = toggle("mouse") {
            view.ui.mouse = v;
        }
        if let Some(v) = toggle("panel") {
            view.ui.panel = v;
        }
        if let Some(v) = toggle("timestamps") {
            view.ui.timestamps = v;
        }
        if let Some(v) = toggle("line_numbers") {
            view.ui.line_numbers = v;
        }
        if let Some(v) = toggle("offer_audit") {
            view.ui.offer_audit = v;
        }
        if let Some(v) = toggle("open_pages") {
            view.ui.open_pages = v;
        }
    }
}

impl Settings {
    /// The row field `index` is drawn on, as [`Form::render`] lays them out:
    /// a header takes a blank row above it (but for the first), and an
    /// error takes a row under its field.
    fn row_of(&self, index: usize) -> usize {
        let before: usize = self
            .form
            .fields
            .iter()
            .take(index)
            .enumerate()
            .map(|(i, f)| match f.kind {
                Kind::Header => 1 + usize::from(i > 0),
                _ => 1 + usize::from(f.error.is_some()),
            })
            .sum();
        // The field's own blank row, when it is a header.
        let own = self
            .form
            .fields
            .get(index)
            .is_some_and(|f| matches!(f.kind, Kind::Header) && index > 0);
        before + usize::from(own)
    }
}

/// What each sandbox profile lets the model's commands reach, side by side,
/// with the chosen one picked out. The facts are `ryter_core::sandbox`'s:
/// its module documentation lists what a profile grants.
///
/// `off` keeps your keys by rule only: Ryter refuses to read them, and
/// nothing stops a command that tries. Under the other two the system
/// refuses.
const SANDBOX_ROWS: &[(&str, [&str; 3])] = &[
    ("project files", ["read, write", "read, write", "read"]),
    ("rest of home", ["read, write", "no", "no"]),
    ("your tools", ["yes", "yes", "yes"]),
    ("your keys", ["rule only †", "never", "never"]),
    ("/tmp", ["read, write", "no", "no"]),
    ("network", ["yes", "yes", "yes"]),
    ("docker", ["yes", "yes *", "yes *"]),
];

/// When each profile is the one to use, in two short lines.
const SANDBOX_WHEN: [[&str; 2]; 3] = [
    ["you watch", "each step"],
    ["a crew runs", "unattended"],
    ["you only want", "a review"],
];

const SANDBOX_PROFILES: [&str; 3] = ["off", "workspace", "read-only"];

fn sandbox_table(chosen: &str, width: usize, theme: Theme) -> Vec<Line<'static>> {
    const LABEL: usize = 16;
    const COL: usize = 14;
    let picked = SANDBOX_PROFILES.iter().position(|p| *p == chosen);
    let dim = theme.panel_muted();
    let plain = theme.panel();
    let strong = Style::default()
        .fg(theme.accent)
        .bg(theme.panel_bg)
        .add_modifier(Modifier::BOLD);
    let cell_style = |col: usize| if picked == Some(col) { strong } else { plain };
    let row = |label: &str, cells: [String; 3], head: bool| -> Line<'static> {
        let mut spans = vec![Span::styled(
            format!(" {}", wrap::pad_right(label, LABEL)),
            dim,
        )];
        for (i, cell) in cells.into_iter().enumerate() {
            let style = if head && picked != Some(i) {
                dim
            } else {
                cell_style(i)
            };
            spans.push(Span::styled(wrap::pad_right(&cell, COL), style));
        }
        Line::from(spans)
    };
    let note = |text: &str| -> Vec<Line<'static>> {
        wrap::wrap_plain(text, width.saturating_sub(2))
            .into_iter()
            .map(|l| Line::from(Span::styled(format!(" {l}"), dim)))
            .collect()
    };
    let blank = || Line::from(Span::styled("", plain));
    let mut out = vec![blank()];
    // The chosen profile's name stands out: capitals, as well as colour,
    // for a terminal with none.
    let names: [String; 3] = std::array::from_fn(|i| {
        if picked == Some(i) {
            SANDBOX_PROFILES[i].to_ascii_uppercase()
        } else {
            SANDBOX_PROFILES[i].to_string()
        }
    });
    out.push(row("", names, true));
    for (label, cells) in SANDBOX_ROWS {
        out.push(row(label, cells.map(str::to_string), false));
    }
    out.push(blank());
    for line in 0..2 {
        let label = if line == 0 { "use it when" } else { "" };
        out.push(row(
            label,
            std::array::from_fn(|i| SANDBOX_WHEN[i][line].to_string()),
            false,
        ));
    }
    out.push(blank());
    out.extend(note(
        "* docker can reach the whole machine; the sandbox does not stop it.",
    ));
    out.extend(note(
        "† Ryter refuses to read your keys, but nothing stops a command that tries.",
    ));
    if cfg!(target_os = "linux") {
        out.extend(note("A change applies the next time Ryter starts."));
    } else {
        out.extend(note(
            "workspace and read-only need Linux. On this system Ryter won't start with either.",
        ));
    }
    out.push(blank());
    out
}

impl Panel for Settings {
    fn kind(&self) -> &'static str {
        "settings"
    }

    fn title(&self, _view: &View) -> String {
        "settings".into()
    }

    fn status(&self, _view: &View) -> String {
        if self.form.dirty {
            "unsaved".into()
        } else {
            String::new()
        }
    }

    fn legend(&self, _view: &View) -> String {
        if self.confirm_discard {
            "y save · n discard · esc keep editing".into()
        } else if self.editing {
            "type · enter apply · esc cancel".into()
        } else {
            "↑↓ move · space/←→ change · enter edit · esc done".into()
        }
    }

    fn input(&self, _view: &View) -> Option<String> {
        self.editing.then(|| {
            self.form
                .current()
                .map(|f| f.label.clone())
                .unwrap_or_else(|| "value".into())
        })
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        (72, 26)
    }

    fn render(&self, _view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let mut rows = self.form.render(usize::from(width), theme, true);
        // The sandbox profiles, compared, under the field that picks one.
        let sandbox = self.form.fields.iter().position(|f| f.id == "sandbox");
        let mut table_rows = 0;
        if let Some(at) = sandbox {
            let chosen = self.form.fields[at].value_text();
            let table = sandbox_table(&chosen, usize::from(width), theme);
            table_rows = table.len();
            let after = self.row_of(at) + 1 + usize::from(self.form.fields[at].error.is_some());
            let after = after.min(rows.len());
            rows.splice(after..after, table);
        }
        let total = rows.len();
        let h = usize::from(height).max(1);
        // Keep the selected row visible. On the sandbox field, that is the
        // table under it too: aim at its middle.
        let mut sel_row = self.row_of(self.form.selected);
        match sandbox {
            Some(at) if self.form.selected > at => sel_row += table_rows,
            Some(at) if self.form.selected == at => sel_row += table_rows.div_ceil(2),
            _ => {}
        }
        let mut first = super::window(sel_row, total, h);
        // On a short panel the field itself stays in view, above as much of
        // the table as fits.
        if let Some(at) = sandbox.filter(|at| *at == self.form.selected) {
            first = first.min(self.row_of(at));
        }
        let mut lines: Vec<_> = rows.into_iter().skip(first).take(h).collect();
        if self.confirm_discard {
            lines = super::widgets::save_prompt(lines, usize::from(width), h, theme);
        }
        Body {
            lines,
            scroll: (total > h).then_some((first, total)),
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        if self.confirm_discard {
            return match key.code {
                KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
                    if self.form.has_errors() {
                        self.confirm_discard = false;
                        return Outcome::Stay;
                    }
                    self.apply(view);
                    self.form.dirty = false;
                    Outcome::CloseAct(Action::SaveSettings)
                }
                KeyCode::Char('n' | 'N') => Outcome::Close,
                KeyCode::Esc => {
                    self.confirm_discard = false;
                    Outcome::Stay
                }
                _ => Outcome::Stay,
            };
        }
        if self.editing {
            return match key.code {
                KeyCode::Esc => {
                    self.editing = false;
                    view.composer.clear();
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    let text = view.composer.text().to_string();
                    if self.form.set_from_text(&text) {
                        self.editing = false;
                        view.composer.clear();
                    }
                    Outcome::Stay
                }
                _ => {
                    super::edit_field(&mut view.composer, key);
                    Outcome::Stay
                }
            };
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                if self.form.dirty {
                    self.confirm_discard = true;
                    Outcome::Stay
                } else {
                    Outcome::Close
                }
            }
            KeyCode::Char('s') if ctrl => {
                if self.form.has_errors() {
                    return Outcome::Stay;
                }
                self.apply(view);
                self.form.dirty = false;
                Outcome::Act(Action::SaveSettings)
            }
            KeyCode::Up | KeyCode::BackTab => {
                self.form.step(-1);
                Outcome::Stay
            }
            KeyCode::Down | KeyCode::Tab => {
                self.form.step(1);
                Outcome::Stay
            }
            KeyCode::Left => {
                self.form.adjust(-1);
                Outcome::Stay
            }
            KeyCode::Right | KeyCode::Char(' ') => {
                self.form.adjust(1);
                Outcome::Stay
            }
            KeyCode::Enter => {
                match self.form.current().map(|f| &f.kind) {
                    Some(Kind::Toggle(_)) => {
                        self.form.adjust(1);
                    }
                    Some(Kind::Text(s)) => {
                        view.composer.set_text(s);
                        self.editing = true;
                    }
                    Some(Kind::Number { .. }) => {
                        let v = self
                            .form
                            .current()
                            .map(Field::value_text)
                            .unwrap_or_default();
                        view.composer.set_text(&v);
                        self.editing = true;
                    }
                    Some(Kind::Select { .. }) => {
                        self.form.adjust(1);
                    }
                    _ => {}
                }
                Outcome::Stay
            }
            _ => Outcome::Stay,
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ryter_core::config::Steps;

    fn view() -> View {
        View::new(
            ryter_core::Phase::Build,
            "c".into(),
            "m".into(),
            "/tmp".into(),
        )
    }

    fn set(s: &mut Settings, id: &str, to: f64) {
        match &mut s.form.get_mut(id).unwrap().kind {
            Kind::Number { value, .. } => *value = to,
            other => panic!("{id} is not a number: {other:?}"),
        }
    }

    /// The crew's step limits are in `/settings`, with their defaults in
    /// the labels, and a saved value is kept within range.
    #[test]
    fn step_limits_are_set_here() {
        let mut v = view();
        v.steps = Steps {
            auditor: 20,
            ..Steps::default()
        };
        let mut s = Settings::new(&v);
        let labels: Vec<String> = ["steps_builder", "steps_architect", "steps_auditor"]
            .iter()
            .map(|id| s.form.get(id).unwrap().label.clone())
            .collect();
        assert_eq!(
            labels,
            [
                "builder steps (def 40)",
                "architect steps (def 30)",
                "auditor steps (def 12)"
            ]
        );
        assert!(labels.iter().all(|l| l.chars().count() <= 25), "{labels:?}");
        assert_eq!(s.form.get("steps_auditor").unwrap().value_text(), "20");
        // Untouched, applying changes nothing.
        s.apply(&mut v);
        assert_eq!(v.steps.auditor, 20);
        assert_eq!(v.steps.builder, 40);
        set(&mut s, "steps_builder", 120.0);
        set(&mut s, "steps_architect", 0.0);
        set(&mut s, "steps_auditor", 100_000.0);
        s.apply(&mut v);
        assert_eq!(
            v.steps,
            Steps {
                builder: 120,
                architect: Steps::MIN,
                auditor: Steps::MAX,
            }
        );
    }

    fn rows(s: &Settings, v: &View, width: u16, height: u16) -> Vec<String> {
        s.render(v, width, height, Theme::truecolor_dark())
            .lines
            .iter()
            .map(|l| l.spans.iter().map(|sp| sp.content.as_ref()).collect())
            .collect()
    }

    /// The sandbox profiles are compared under the field that picks one,
    /// with the chosen one picked out, and the table moves with the choice.
    #[test]
    fn sandbox_profiles_are_compared_under_the_field() {
        let mut v = view();
        v.sandbox_profile = "off".into();
        let mut s = Settings::new(&v);
        let at = s
            .form
            .fields
            .iter()
            .position(|f| f.id == "sandbox")
            .unwrap();
        s.form.selected = at;
        let shown = rows(&s, &v, 72, 26);
        let all = shown.join("\n");
        let field = shown.iter().position(|l| l.contains("‹ off ›")).unwrap();
        // The table is right under the field, all of it on screen.
        for (below, want) in [
            (2, "OFF           workspace     read-only"),
            (3, "project files   read, write   read, write   read"),
            (4, "rest of home    read, write   no            no"),
            (5, "your tools      yes           yes           yes"),
            (6, "your keys       rule only †   never         never"),
            (7, "/tmp            read, write   no            no"),
            (8, "network         yes           yes           yes"),
            (9, "docker          yes           yes *         yes *"),
            (
                11,
                "use it when     you watch     a crew runs   you only want",
            ),
            (12, "each step     unattended    a review"),
        ] {
            assert!(
                shown[field + below].contains(want),
                "row {below}: {:?}\n{all}",
                shown[field + below]
            );
        }
        assert!(
            all.contains("* docker can reach the whole machine"),
            "{all}"
        );
        assert!(all.contains("† Ryter refuses to read your keys"), "{all}");
        // Choosing another profile moves the mark.
        s.form.adjust(1);
        let all = rows(&s, &v, 72, 26).join("\n");
        assert!(
            all.contains("off           WORKSPACE     read-only"),
            "{all}"
        );
        s.form.adjust(1);
        let all = rows(&s, &v, 72, 26).join("\n");
        assert!(
            all.contains("off           workspace     READ-ONLY"),
            "{all}"
        );
        // Every row fits the panel, and a field below the table is still reached.
        assert!(rows(&s, &v, 72, 26).iter().all(|l| l.chars().count() <= 72));
        // On a short panel the field stays in view above the table.
        s.form.selected = at;
        let short = rows(&s, &v, 72, 12);
        assert!(short[0].contains("‹ read-only ›"), "{short:?}");
        assert!(
            short.iter().any(|l| l.contains("project files")),
            "{short:?}"
        );
        s.form.selected = s
            .form
            .fields
            .iter()
            .position(|f| f.id == "inbound")
            .unwrap();
        let below = rows(&s, &v, 72, 26).join("\n");
        assert!(below.contains("› inbound"), "{below}");
    }
}
