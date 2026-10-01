//! The plan popout: a plan the model wants to carry out, for the user to
//! read and answer.
//!
//! A plan used to be written into the chat, with a yes/no card about
//! switching hats under it. Here the plan itself is on a panel that scrolls,
//! and the user approves it, says what to change, or rejects it.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ryter_core::user_io::PlanAnswer;

use super::modal::ENTER_GUARD_MS;
use super::{Body, ModalKind, Outcome, Panel, widgets};
use crate::action::Action;
use crate::chat::markdown;
use crate::theme::Theme;
use crate::view::View;

/// The popout's width, where the screen has it.
const WIDTH: u16 = 96;

/// The text of a Markdown heading line, outside a code block.
fn heading(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix('#')?;
    let rest = rest.trim_start_matches('#');
    rest.starts_with(' ').then(|| rest.trim())
}

/// The plan as rows `width` wide: each heading a bold line of its own with
/// its section straight under it, and a blank row between sections. The
/// chat draws a heading with its `##` and a rule under it, which is right
/// for a long answer and heavy for a plan of five short sections.
fn plan_rows(plan: &str, width: usize, theme: Theme) -> Vec<Line<'static>> {
    // The panel's own ground, so the plan reads as part of it.
    let on_panel = Theme {
        bg: theme.panel_bg,
        ..theme
    };
    let md = markdown::MdOptions {
        width: width.saturating_sub(2).max(8),
        line_numbers: false,
        lang_hint: None,
    };
    let pad = |l: Line<'static>| {
        let mut spans = vec![Span::styled(" ", theme.panel())];
        spans.extend(l.spans);
        Line::from(spans)
    };
    let blank = |l: &Line<'static>| l.spans.iter().all(|s| s.content.trim().is_empty());
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut body = String::new();
    let flush = |body: &mut String, out: &mut Vec<Line<'static>>| {
        if !body.trim().is_empty() {
            let mut rows = markdown::render(body.trim_matches('\n'), &md, on_panel);
            while rows.last().is_some_and(blank) {
                rows.pop();
            }
            let lead = rows.iter().take_while(|l| blank(l)).count();
            out.extend(rows.into_iter().skip(lead).map(pad));
        }
        body.clear();
    };
    let mut fenced = false;
    for line in plan.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        match heading(line).filter(|_| !fenced) {
            Some(title) => {
                flush(&mut body, &mut out);
                if !out.is_empty() {
                    out.push(Line::from(Span::styled("", theme.panel())));
                }
                out.push(Line::from(Span::styled(
                    format!(" {title}"),
                    Style::default()
                        .fg(theme.accent)
                        .bg(theme.panel_bg)
                        .add_modifier(Modifier::BOLD),
                )));
            }
            None => {
                body.push_str(line);
                body.push('\n');
            }
        }
    }
    flush(&mut body, &mut out);
    out
}

/// What the popout is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pane {
    /// Reading the plan.
    Read,
    /// Typing what to change.
    Adjust,
}

/// The plan popout.
#[derive(Debug, Clone)]
pub struct PlanModal {
    /// A few words: the panel's title.
    title: String,
    /// The plan, in Markdown.
    plan: String,
    pane: Pane,
    /// First row shown.
    top: usize,
    /// The furthest `top` can go, and the rows that fit, as last drawn.
    max_top: std::cell::Cell<usize>,
    page: std::cell::Cell<usize>,
    /// The plan as rows, for the width it was last drawn at.
    rows: std::cell::RefCell<Option<(usize, Vec<Line<'static>>)>>,
    /// `view.now_ms` when it opened: a `y` typed as it appeared isn't an answer.
    opened_ms: u64,
    error: Option<&'static str>,
}

impl PlanModal {
    /// Build.
    pub fn new(title: String, plan: String, opened_ms: u64) -> Self {
        Self {
            title,
            plan,
            pane: Pane::Read,
            top: 0,
            max_top: std::cell::Cell::new(0),
            page: std::cell::Cell::new(1),
            rows: std::cell::RefCell::new(None),
            opened_ms,
            error: None,
        }
    }

    fn answer(answer: PlanAnswer) -> Outcome {
        Outcome::CloseAct(Action::PlanReply(answer))
    }
}

impl Panel for PlanModal {
    fn kind(&self) -> &'static str {
        "plan"
    }

    fn title(&self, _view: &View) -> String {
        format!("plan · {}", self.title)
    }

    fn legend(&self, _view: &View) -> String {
        match self.pane {
            Pane::Read => "↑↓ scroll · y approve · e adjust · n reject".into(),
            Pane::Adjust => "type what to change · enter send · esc back".into(),
        }
    }

    fn input(&self, _view: &View) -> Option<String> {
        matches!(self.pane, Pane::Adjust).then(|| "change".into())
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        // As tall as the plan is when drawn, up to what the screen gives.
        let rows = plan_rows(&self.plan, usize::from(WIDTH) - 2, Theme::truecolor_dark()).len();
        // And a row of air above the keys.
        (WIDTH, (rows + 1).clamp(10, 200) as u16)
    }

    fn modal(&self) -> Option<ModalKind> {
        Some(ModalKind::Ask)
    }

    fn render(&self, _view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let h = usize::from(height).max(1);
        let mut laid_out = self.rows.borrow_mut();
        if laid_out.as_ref().is_none_or(|(at, _)| *at != w) {
            *laid_out = Some((w, plan_rows(&self.plan, w, theme)));
        }
        let all = laid_out.as_ref().map_or(&[][..], |(_, rows)| rows);
        let foot = usize::from(self.error.is_some());
        let room = h.saturating_sub(foot).max(1);
        let total = all.len();
        let max_top = total.saturating_sub(room);
        let top = self.top.min(max_top);
        self.max_top.set(max_top);
        self.page.set(room);
        let mut lines: Vec<Line<'static>> = all.iter().skip(top).take(room).cloned().collect();
        if let Some(e) = self.error {
            lines.push(widgets::colored(e, theme.warn, theme));
        }
        Body {
            lines,
            scroll: (total > room).then_some((top, total)),
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        match self.pane {
            Pane::Read => {
                let max = self.max_top.get();
                let top = self.top.min(max);
                let page = self.page.get().saturating_sub(1).max(1);
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => self.top = top.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => self.top = (top + 1).min(max),
                    KeyCode::PageUp => self.top = top.saturating_sub(page),
                    KeyCode::PageDown | KeyCode::Char(' ') => self.top = (top + page).min(max),
                    KeyCode::Home => self.top = 0,
                    KeyCode::End => self.top = max,
                    // A `y` in the moment the panel appeared was typed at
                    // something else.
                    KeyCode::Char('y' | 'Y') if view.now_ms < self.opened_ms + ENTER_GUARD_MS => {}
                    KeyCode::Char('y' | 'Y') => return Self::answer(PlanAnswer::Approve),
                    KeyCode::Char('e' | 'E') => {
                        self.error = None;
                        view.composer.clear();
                        self.pane = Pane::Adjust;
                    }
                    KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                        return Self::answer(PlanAnswer::Reject);
                    }
                    // Enter starts work on nothing: `y` is the yes.
                    KeyCode::Enter => {
                        self.error =
                            Some("Enter doesn't approve a plan: y approves, e adjusts, n rejects");
                    }
                    _ => {}
                }
                if !matches!(key.code, KeyCode::Enter) {
                    self.error = None;
                }
                Outcome::Stay
            }
            Pane::Adjust => match key.code {
                KeyCode::Esc => {
                    view.composer.clear();
                    self.error = None;
                    self.pane = Pane::Read;
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    let what = view.composer.text().trim().to_string();
                    if what.is_empty() {
                        self.error = Some("type what should change, then enter");
                        return Outcome::Stay;
                    }
                    view.composer.clear();
                    Self::answer(PlanAnswer::Adjust(what))
                }
                _ => {
                    super::edit_field(&mut view.composer, key);
                    self.error = None;
                    Outcome::Stay
                }
            },
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    const PLAN: &str = "## Goal\nReports can be downloaded as CSV from the list.\n\n## Steps\n1. Add export_csv() to reports/service.py\n2. Add the /reports/export route\n3. Add the button to the list template\n4. Tests for empty and very large reports\n\n## Files\nreports/service.py · reports/views.py · list.html\n\n## Risks\nLarge reports: stream rows, don't build in memory\n\n## How to verify\npytest tests/test_export.py\n";

    fn view() -> View {
        let mut v = View::new(
            ryter_core::Phase::Build,
            "c".into(),
            "m".into(),
            "/tmp".into(),
        );
        v.now_ms = 10_000;
        v
    }

    fn press(p: &mut PlanModal, v: &mut View, code: KeyCode) -> Outcome {
        p.key(KeyEvent::new(code, KeyModifiers::NONE), v)
    }

    fn rows(p: &PlanModal, v: &View, height: u16) -> Vec<String> {
        p.render(v, 60, height, Theme::truecolor_dark())
            .lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    fn answered(o: &Outcome) -> Option<PlanAnswer> {
        match o {
            Outcome::CloseAct(Action::PlanReply(a)) => Some(a.clone()),
            _ => None,
        }
    }

    /// The plan is on the panel as the user was shown it: its title in the
    /// border, its sections and steps in order, and the three answers.
    #[test]
    fn the_plan_is_read_on_its_own_panel() {
        let v = view();
        let p = PlanModal::new("Add CSV export to reports".into(), PLAN.into(), 0);
        assert_eq!(p.title(&v), "plan · Add CSV export to reports");
        assert_eq!(p.legend(&v), "↑↓ scroll · y approve · e adjust · n reject");
        let shown = rows(&p, &v, 40).join("\n");
        let mut at = 0;
        for want in [
            "Goal",
            "Reports can be downloaded as CSV from the list.",
            "Steps",
            "Add export_csv() to reports/service.py",
            "Tests for empty and very large reports",
            "Files",
            "Risks",
            "How to verify",
            "pytest tests/test_export.py",
        ] {
            let i = shown[at..]
                .find(want)
                .unwrap_or_else(|| panic!("{want} is missing or out of order:\n{shown}"));
            at += i;
        }
        assert_eq!(p.modal(), Some(ModalKind::Ask));
        // As the user was shown it: a heading is a line of its own with its
        // section straight under it, and a blank row between sections.
        let rows = rows(&p, &v, 40);
        assert_eq!(
            &rows[..6],
            [
                " Goal",
                " Reports can be downloaded as CSV from the list.",
                "",
                " Steps",
                " 1. Add export_csv() to reports/service.py",
                " 2. Add the /reports/export route",
            ],
            "{rows:?}"
        );
        assert!(
            !rows.iter().any(|r| r.contains('#') || r.contains("───")),
            "{rows:?}"
        );
        // The panel asks for the room the plan takes: nothing to scroll.
        let (_, tall) = p.size(&v);
        assert_eq!(usize::from(tall), rows.len() + 1);
    }

    /// A plan longer than the panel scrolls, and every row is reachable.
    #[test]
    fn a_long_plan_scrolls() {
        let mut v = view();
        let plan: String = (1..=60)
            .map(|i| format!("{i}. step number {i}\n"))
            .collect();
        let mut p = PlanModal::new("Long".into(), plan, 0);
        let first = rows(&p, &v, 10);
        assert!(
            first.iter().any(|r| r.contains("step number 1")),
            "{first:?}"
        );
        assert!(!first.iter().any(|r| r.contains("step number 60")));
        let mut seen = first;
        for _ in 0..20 {
            press(&mut p, &mut v, KeyCode::PageDown);
            seen.extend(rows(&p, &v, 10));
        }
        for i in 1..=60 {
            assert!(
                seen.iter().any(|r| r.contains(&format!("step number {i}"))),
                "step {i} was never shown"
            );
        }
        press(&mut p, &mut v, KeyCode::Home);
        assert!(rows(&p, &v, 10).iter().any(|r| r.contains("step number 1")));
    }

    /// `y` approves, though not in the moment the panel appeared; `n` and
    /// Esc reject; Enter does neither.
    #[test]
    fn the_three_answers() {
        let mut v = view();
        let mut p = PlanModal::new("T".into(), PLAN.into(), 10_000);
        v.now_ms = 10_100;
        assert_eq!(answered(&press(&mut p, &mut v, KeyCode::Char('y'))), None);
        v.now_ms = 11_000;
        assert_eq!(answered(&press(&mut p, &mut v, KeyCode::Enter)), None);
        assert!(
            rows(&p, &v, 30)
                .iter()
                .any(|r| r.contains("Enter doesn't approve"))
        );
        assert_eq!(
            answered(&press(&mut p, &mut v, KeyCode::Char('y'))),
            Some(PlanAnswer::Approve)
        );
        for code in [KeyCode::Char('n'), KeyCode::Esc] {
            let mut p = PlanModal::new("T".into(), PLAN.into(), 0);
            assert_eq!(
                answered(&press(&mut p, &mut v, code)),
                Some(PlanAnswer::Reject)
            );
        }
    }

    /// `e` asks what to change, typed as a person types it, and sends that.
    /// Esc from there goes back to the plan, not out of it.
    #[test]
    fn adjust_sends_what_the_user_typed() {
        let mut v = view();
        let mut p = PlanModal::new("T".into(), PLAN.into(), 0);
        press(&mut p, &mut v, KeyCode::Char('e'));
        assert_eq!(p.input(&v).as_deref(), Some("change"));
        assert_eq!(p.legend(&v), "type what to change · enter send · esc back");
        // Nothing typed sends nothing.
        assert_eq!(answered(&press(&mut p, &mut v, KeyCode::Enter)), None);
        for c in "Stream the rows; skip the button for now".chars() {
            press(&mut p, &mut v, KeyCode::Char(c));
        }
        // Letters that are answers elsewhere are text here.
        assert_eq!(p.pane, Pane::Adjust);
        assert_eq!(
            answered(&press(&mut p, &mut v, KeyCode::Enter)),
            Some(PlanAnswer::Adjust(
                "Stream the rows; skip the button for now".into()
            ))
        );
        let mut p = PlanModal::new("T".into(), PLAN.into(), 0);
        press(&mut p, &mut v, KeyCode::Char('e'));
        assert_eq!(answered(&press(&mut p, &mut v, KeyCode::Esc)), None);
        assert_eq!(p.pane, Pane::Read);
    }
}
