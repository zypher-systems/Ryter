//! The audit popout: what the audit hat found, for the user to act on.
//!
//! An audit used to be words in the chat with a verdict at the end. Here it
//! is on a panel that owns the body, as a plan is: the verdict, the
//! findings worst first, what ran, what the tree says, and where it was
//! written. `y` hands it to the build hat as the repair brief.

use std::cell::{Cell, RefCell};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ryter_core::event::AgentEvent;

use super::modal::ENTER_GUARD_MS;
use super::{Body, ModalKind, Outcome, Panel, widgets};
use crate::action::Action;
use crate::chat::wrap;
use crate::theme::Theme;
use crate::view::View;

/// The popout's width, where the screen has it.
const WIDTH: u16 = 100;

/// An audit, as the `Audited` event carried it.
#[derive(Clone)]
pub struct AuditModal {
    pub model: String,
    pub verdict: Option<bool>,
    pub headline: String,
    pub summary: String,
    /// `mark n\ttitle\tright`, with `    detail` rows under a failure.
    pub rows: Vec<String>,
    pub ran: Vec<String>,
    pub file: Option<String>,
    pub restored: Vec<String>,
    pub checkpointed: bool,
    pub total_usd: Option<f64>,
    pub duration_ms: u64,
    top: usize,
    max_top: Cell<usize>,
    page: Cell<usize>,
    opened_ms: u64,
    laid: RefCell<Option<(usize, Vec<Line<'static>>)>>,
}

impl AuditModal {
    /// From the event, when it carries a filed audit.
    /// Whether the audit found nothing to repair: no failing finding (one
    /// not reached is not a failure).
    pub fn passed(&self) -> bool {
        self.verdict == Some(true)
    }

    pub fn from_event(ev: &AgentEvent, opened_ms: u64) -> Option<Self> {
        let AgentEvent::Audited {
            model,
            verdict,
            headline,
            summary,
            rows,
            ran,
            file,
            restored,
            checkpointed,
            total_usd,
            duration_ms,
            ..
        } = ev
        else {
            return None;
        };
        Some(Self {
            model: model.clone(),
            verdict: *verdict,
            headline: headline.clone(),
            summary: summary.clone(),
            rows: rows.clone(),
            ran: ran.clone(),
            file: file.clone(),
            restored: restored.clone(),
            checkpointed: *checkpointed,
            total_usd: *total_usd,
            duration_ms: *duration_ms,
            top: 0,
            max_top: Cell::new(0),
            page: Cell::new(1),
            opened_ms,
            laid: RefCell::new(None),
        })
    }

    /// `0:58`.
    fn clock(ms: u64) -> String {
        let s = ms / 1000;
        format!("{}:{:02}", s / 60, s % 60)
    }

    /// One row: the left part in `left_style`, the right part in the dim
    /// color against the far edge, the middle padded so the frame stays
    /// square whatever the words are.
    fn two_ends(
        left: Vec<Span<'static>>,
        right: &str,
        width: usize,
        theme: Theme,
    ) -> Line<'static> {
        let used: usize = left.iter().map(|s| wrap::width(&s.content)).sum();
        let right_w = wrap::width(right);
        let mut spans = left;
        if right_w > 0 {
            let gap = width.saturating_sub(used + right_w).max(2);
            spans.push(Span::styled(" ".repeat(gap), theme.panel()));
            spans.push(Span::styled(right.to_string(), theme.panel_muted()));
        }
        Line::from(spans)
    }

    fn lines(&self, width: usize, theme: Theme) -> Vec<Line<'static>> {
        let w = width.saturating_sub(2).max(20);
        let bold = |color| theme.on_panel(color).add_modifier(Modifier::BOLD);
        let mut out: Vec<Line<'static>> = Vec::new();
        let (word, color) = match self.verdict {
            Some(true) => ("VERDICT: PASS", theme.success),
            Some(false) => ("VERDICT: FAIL", theme.error),
            None => ("NO VERDICT", theme.warn),
        };
        out.push(Line::from(vec![
            Span::styled(format!(" {word}"), bold(color)),
            Span::styled(format!("   {}", self.summary), theme.panel()),
        ]));
        out.push(widgets::blank(theme));
        for row in &self.rows {
            if let Some(detail) = row.strip_prefix("    ") {
                let (label, text) = match detail.strip_prefix("saw: ") {
                    Some(saw) => ("saw: ", saw),
                    None => ("", detail),
                };
                for (i, part) in wrap::wrap_plain(text, w.saturating_sub(10))
                    .iter()
                    .enumerate()
                {
                    let lead = if i == 0 { label } else { "" };
                    out.push(Line::from(vec![
                        Span::styled(format!("      {lead:<5}"), theme.panel_muted()),
                        Span::styled(part.clone(), theme.panel_muted()),
                    ]));
                }
                continue;
            }
            let mut parts = row.splitn(3, '\t');
            let mark = parts.next().unwrap_or("").to_string();
            let title = parts.next().unwrap_or("").to_string();
            let right = parts.next().unwrap_or("");
            let mark_color = match mark.chars().next() {
                Some('✗') => theme.error,
                Some('✓') => theme.success,
                _ => theme.dim,
            };
            let left = vec![
                Span::styled(format!(" {mark:<5}"), bold(mark_color)),
                Span::styled(title, theme.panel()),
            ];
            out.push(Self::two_ends(left, right, w, theme));
        }
        out.push(widgets::blank(theme));
        if !self.ran.is_empty() {
            let joined = self.ran.join(" · ");
            for (i, part) in wrap::wrap_plain(&joined, w.saturating_sub(9))
                .iter()
                .enumerate()
            {
                let label = if i == 0 { "ran" } else { "" };
                out.push(Line::from(vec![
                    Span::styled(format!(" {label:<7}"), theme.panel_muted()),
                    Span::styled(part.clone(), theme.panel_muted()),
                ]));
            }
        }
        let tree: Vec<(String, ratatui::style::Color)> = if !self.checkpointed {
            vec![(
                "no checkpoint: not a git repository, so the audit ran read-only commands".into(),
                theme.warn,
            )]
        } else if self.restored.is_empty() {
            vec![("changed nothing · checkpoint kept".into(), theme.success)]
        } else {
            let mut v = vec![(
                format!(
                    "restored {} file{} the audit had changed:",
                    self.restored.len(),
                    if self.restored.len() == 1 { "" } else { "s" }
                ),
                theme.warn,
            )];
            v.extend(self.restored.iter().map(|p| (format!("  {p}"), theme.warn)));
            v
        };
        for (i, (text, color)) in tree.into_iter().enumerate() {
            let label = if i == 0 { "tree" } else { "" };
            out.push(Line::from(vec![
                Span::styled(format!(" {label:<7}"), theme.panel_muted()),
                Span::styled(text, theme.on_panel(color)),
            ]));
        }
        out.push(Line::from(vec![
            Span::styled(" written", theme.panel_muted()),
            Span::styled(
                format!(
                    " {}",
                    self.file
                        .as_deref()
                        .unwrap_or("nowhere: the audit could not be saved")
                ),
                theme.panel_muted(),
            ),
        ]));
        out
    }
}

impl Panel for AuditModal {
    fn kind(&self) -> &'static str {
        "audit"
    }

    fn title(&self, _view: &View) -> String {
        let cost = self.total_usd.map_or_else(String::new, |usd| {
            format!(" · {}", crate::chat::turn_usd(usd))
        });
        format!(
            "audit · {} · {}{cost} · {}",
            crate::chat::short_model(&self.model),
            self.headline,
            Self::clock(self.duration_ms)
        )
    }

    fn legend(&self, _view: &View) -> String {
        // A passed audit has nothing to repair: Enter closes it. A failed
        // one goes to the build hat on Enter or `y`.
        let mut keys: Vec<&str> = if self.passed() {
            vec!["⏎ close", "n close"]
        } else {
            vec!["⏎ repair in build", "y repair in build", "n close"]
        };
        if self.file.is_some() {
            keys.push("o open audit.md");
        }
        keys.push("↑↓ scroll");
        keys.join(" · ")
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        let rows = self
            .lines(usize::from(WIDTH) - 2, Theme::truecolor_dark())
            .len();
        (WIDTH, (rows + 1).clamp(8, 200) as u16)
    }

    fn modal(&self) -> Option<ModalKind> {
        Some(ModalKind::Ask)
    }

    fn render(&self, _view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let h = usize::from(height).max(1);
        let mut laid = self.laid.borrow_mut();
        if laid.as_ref().is_none_or(|(at, _)| *at != w) {
            *laid = Some((w, self.lines(w, theme)));
        }
        let all = laid.as_ref().map_or(&[][..], |(_, rows)| rows);
        let total = all.len();
        let max_top = total.saturating_sub(h);
        let top = self.top.min(max_top);
        self.max_top.set(max_top);
        self.page.set(h);
        Body {
            lines: all.iter().skip(top).take(h).cloned().collect(),
            scroll: (total > h).then_some((top, total)),
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        let max = self.max_top.get();
        let top = self.top.min(max);
        let page = self.page.get().saturating_sub(1).max(1);
        // A chord is not one of the card's keys.
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return Outcome::Stay;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.top = top.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.top = (top + 1).min(max),
            KeyCode::PageUp => self.top = top.saturating_sub(page),
            KeyCode::PageDown | KeyCode::Char(' ') => self.top = (top + page).min(max),
            KeyCode::Home => self.top = 0,
            KeyCode::End => self.top = max,
            // A passed audit has nothing to repair: Enter closes it, and
            // `y` is not one of its keys.
            KeyCode::Enter if self.passed() => return Outcome::Close,
            KeyCode::Char('y' | 'Y') if self.passed() => {}
            // A `y` or an Enter in the moment the panel appeared was typed
            // at something else.
            KeyCode::Char('y' | 'Y') | KeyCode::Enter
                if view.now_ms < self.opened_ms + ENTER_GUARD_MS => {}
            KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
                return Outcome::CloseAct(Action::RepairFromAudit {
                    file: self.file.clone(),
                });
            }
            KeyCode::Char('n' | 'N') | KeyCode::Esc => return Outcome::Close,
            KeyCode::Char('o' | 'O') => {
                if let Some(file) = &self.file {
                    return Outcome::Act(Action::OpenAuditFile(file.clone()));
                }
            }
            _ => {}
        }
        Outcome::Stay
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn view() -> View {
        let mut v = View::new("c".into(), "m".into(), "/tmp".into());
        v.now_ms = ENTER_GUARD_MS * 4;
        v
    }

    fn event() -> AgentEvent {
        AgentEvent::Audited {
            model: "kimi-k3".into(),
            verdict: Some(false),
            headline: "✗ 1 of 3 failed".into(),
            summary: "the upload limit; the rest holds".into(),
            rows: vec![
                "✗ 1\tSize limit checked after the write\tapp/images.py:41".into(),
                "    A 6 MB upload lands in uploads/ and is refused only then.".into(),
                "    saw: curl -F picture=@big.png → 200".into(),
                "✓ 2\tPhysics tests\t6 passed".into(),
                "○ 3\tPause keys\tnot reached: headless run".into(),
            ],
            ran: vec![
                "run_project test".into(),
                "curl -F picture=@big.png localhost:8001/items/new".into(),
            ],
            file: Some(".ryter/audit.md".into()),
            restored: vec![],
            checkpointed: true,
            filed: true,
            total_usd: Some(0.07),
            duration_ms: 58_000,
        }
    }

    fn passed_event() -> AgentEvent {
        match event() {
            AgentEvent::Audited {
                model,
                file,
                ran,
                restored,
                checkpointed,
                filed,
                total_usd,
                duration_ms,
                ..
            } => AgentEvent::Audited {
                model,
                verdict: Some(true),
                headline: "✓ 3 of 3 passed".into(),
                summary: "nothing blocking".into(),
                rows: vec![
                    "✓ 1\tSize limit\tapp/images.py".into(),
                    "✓ 2\tPhysics tests\t6 passed".into(),
                    "○ 3\tPause keys\tnot reached: headless run".into(),
                ],
                ran,
                file,
                restored,
                checkpointed,
                filed,
                total_usd,
                duration_ms,
            },
            other => other,
        }
    }

    /// A passed audit has nothing to repair: Enter and `n` close it, `y`
    /// does nothing, and the legend says so. A failed one repairs on
    /// Enter as on `y`.
    #[test]
    fn enter_closes_a_pass_and_repairs_a_failure() {
        let mut v = view();
        let mut pass = AuditModal::from_event(&passed_event(), 0).unwrap();
        assert_eq!(
            pass.legend(&v),
            "⏎ close · n close · o open audit.md · ↑↓ scroll"
        );
        assert!(matches!(
            press(&mut pass, &mut v, KeyCode::Char('y')),
            Outcome::Stay
        ));
        assert!(matches!(
            press(&mut pass, &mut v, KeyCode::Enter),
            Outcome::Close
        ));
        let mut fail = AuditModal::from_event(&event(), 0).unwrap();
        assert_eq!(
            fail.legend(&v),
            "⏎ repair in build · y repair in build · n close · o open audit.md · ↑↓ scroll"
        );
        assert!(matches!(
            press(&mut fail, &mut v, KeyCode::Enter),
            Outcome::CloseAct(Action::RepairFromAudit { file: Some(f) }) if f == ".ryter/audit.md"
        ));
        // Too soon after it opened, Enter is a key typed at something else.
        let mut fresh = AuditModal::from_event(&event(), v.now_ms).unwrap();
        assert!(matches!(
            press(&mut fresh, &mut v, KeyCode::Enter),
            Outcome::Stay
        ));
    }

    fn press(p: &mut AuditModal, v: &mut View, code: KeyCode) -> Outcome {
        p.key(KeyEvent::new(code, KeyModifiers::NONE), v)
    }

    /// `Ctrl+Y` is not `y`: a chord does nothing on the card.
    #[test]
    fn a_chord_is_not_one_of_the_cards_keys() {
        let mut v = View::new("c".into(), "m".into(), "p".into());
        v.now_ms = 10_000;
        let mut p = AuditModal::from_event(&event(), 0).unwrap();
        for code in [KeyCode::Char('y'), KeyCode::Char('n'), KeyCode::Char('o')] {
            assert!(matches!(
                p.key(KeyEvent::new(code, KeyModifiers::CONTROL), &mut v),
                Outcome::Stay
            ));
        }
    }

    fn text(p: &AuditModal, v: &View, width: u16, height: u16) -> Vec<String> {
        p.render(v, width, height, Theme::truecolor_dark())
            .lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    /// The audit reads as its card: verdict, findings with their places at
    /// the right edge, detail under a failure, what ran, the tree, the file.
    #[test]
    fn the_audit_is_read_on_its_own_panel() {
        let v = view();
        let p = AuditModal::from_event(&event(), 0).unwrap();
        assert_eq!(
            p.title(&v),
            "audit · kimi-k3 · ✗ 1 of 3 failed · $0.070 · 0:58"
        );
        let (w, h) = p.size(&v);
        let rows = text(&p, &v, w, h);
        assert!(
            rows[0].starts_with(" VERDICT: FAIL   the upload limit"),
            "{}",
            rows[0]
        );
        let first = rows
            .iter()
            .find(|r| r.contains("Size limit checked"))
            .unwrap();
        assert!(first.ends_with("app/images.py:41"), "{first}");
        assert_eq!(
            wrap::width(first),
            usize::from(w) - 2,
            "padded to the frame"
        );
        assert!(
            rows.iter().any(|r| r.contains("saw: curl -F picture")),
            "{rows:?}"
        );
        assert!(rows.iter().any(|r| r.contains("not reached: headless run")));
        assert!(
            rows.iter()
                .any(|r| r.contains("ran") && r.contains("run_project test"))
        );
        assert!(
            rows.iter()
                .any(|r| r.contains("changed nothing · checkpoint kept"))
        );
        assert!(rows.iter().any(|r| r.contains("written .ryter/audit.md")));
        assert!(p.legend(&v).contains("o open audit.md"));
    }

    /// `y` hands the audit to the build hat, `n` and `esc` close, `o`
    /// opens the file, and the rows scroll by key.
    #[test]
    fn the_keys() {
        let mut v = view();
        let mut p = AuditModal::from_event(&event(), 0).unwrap();
        assert!(matches!(
            press(&mut p, &mut v, KeyCode::Char('y')),
            Outcome::CloseAct(Action::RepairFromAudit { file: Some(f) }) if f == ".ryter/audit.md"
        ));
        assert!(matches!(
            press(&mut p, &mut v, KeyCode::Char('n')),
            Outcome::Close
        ));
        assert!(matches!(
            press(&mut p, &mut v, KeyCode::Esc),
            Outcome::Close
        ));
        assert!(matches!(
            press(&mut p, &mut v, KeyCode::Char('o')),
            Outcome::Act(Action::OpenAuditFile(f)) if f == ".ryter/audit.md"
        ));
        // Too soon after it opened, `y` is a key typed at something else.
        let mut fresh = AuditModal::from_event(&event(), v.now_ms).unwrap();
        assert!(matches!(
            press(&mut fresh, &mut v, KeyCode::Char('y')),
            Outcome::Stay
        ));
        // Scrolling: a short window moves through the rows.
        let _ = p.render(&v, WIDTH, 4, Theme::truecolor_dark());
        let before = text(&p, &v, WIDTH, 4)[0].clone();
        press(&mut p, &mut v, KeyCode::Down);
        let after = text(&p, &v, WIDTH, 4)[0].clone();
        assert_ne!(before, after);
        press(&mut p, &mut v, KeyCode::End);
        let last = text(&p, &v, WIDTH, 4);
        assert!(last.iter().any(|r| r.contains("written")));
        press(&mut p, &mut v, KeyCode::Home);
        assert!(text(&p, &v, WIDTH, 4)[0].contains("VERDICT"));
    }

    /// A restored tree and a missing checkpoint are said in the card.
    #[test]
    fn the_tree_line() {
        let v = view();
        let mut ev = event();
        if let AgentEvent::Audited { restored, .. } = &mut ev {
            *restored = vec!["app/x.py".into(), "tests/test_x.py".into()];
        }
        let p = AuditModal::from_event(&ev, 0).unwrap();
        let rows = text(&p, &v, WIDTH, 60);
        assert!(
            rows.iter()
                .any(|r| r.contains("restored 2 files the audit had changed"))
        );
        assert!(rows.iter().any(|r| r.contains("  app/x.py")));
        if let AgentEvent::Audited {
            checkpointed,
            restored,
            ..
        } = &mut ev
        {
            *checkpointed = false;
            restored.clear();
        }
        let p = AuditModal::from_event(&ev, 0).unwrap();
        let rows = text(&p, &v, WIDTH, 60);
        assert!(
            rows.iter()
                .any(|r| r.contains("no checkpoint: not a git repository"))
        );
    }
}
