//! Modal interrupts: permission, ask_user, trust (`R-POP-75..80`).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ryter_core::Permission;

use super::{Body, ModalKind, Outcome, Panel, widgets};
use crate::action::Action;
use crate::chat::{highlight, wrap};
use crate::theme::Theme;
use crate::view::View;

/// Enter answers yes only after the prompt has been up this long, so a
/// press meant to send a message can't approve what appeared under it.
pub const ENTER_GUARD_MS: u64 = 500;

/// Permission prompt (`R-POP-76`, `R-POP-77`): a card docked above the
/// composer, where the eyes already are, saying what the call does, why the
/// model says it wants it, what's at risk, and whether `/undo` reaches it.
#[derive(Debug, Clone)]
pub struct PermissionModal {
    /// Tool name.
    pub tool: String,
    /// Argument preview.
    pub summary: String,
    /// For an edit: the change it would make, as tinted diff rows.
    pub preview: Option<Box<ryter_core::diff::FileDiff>>,
    /// Only `y` answers yes: destruction an undo may not reach.
    pub strict: bool,
    /// What `a` allows for the session, in words; `None`: no `a`.
    pub scope: Option<String>,
    /// What the model said just before asking.
    pub why: Option<String>,
    /// `view.now_ms` when the prompt opened, for the Enter guard.
    pub opened_ms: u64,
    /// Enter was pressed where it can't answer: say what can.
    nudge: Option<&'static str>,
}

impl PermissionModal {
    /// Build.
    pub fn new(tool: String, summary: String) -> Self {
        Self {
            tool,
            summary,
            preview: None,
            strict: false,
            scope: None,
            why: None,
            opened_ms: 0,
            nudge: None,
        }
    }

    /// Show the change an edit would make, not just its path.
    #[must_use]
    pub fn with_preview(mut self, preview: Option<Box<ryter_core::diff::FileDiff>>) -> Self {
        self.preview = preview;
        self
    }

    /// How it may be answered: `strict` takes only `y`; `scope` is what `a`
    /// allows for the session.
    #[must_use]
    pub fn with_answers(mut self, strict: bool, scope: Option<String>) -> Self {
        self.strict = strict;
        self.scope = scope;
        self
    }

    /// The model's words just before it asked, and when the prompt opened.
    #[must_use]
    pub fn with_context(mut self, why: Option<String>, opened_ms: u64) -> Self {
        self.why = why.filter(|w| !w.trim().is_empty());
        self.opened_ms = opened_ms;
        self
    }
}

fn lang_for(tool: &str, summary: &str) -> Option<String> {
    match tool {
        "bash" | "shell" | "run" => Some("bash".into()),
        "search_replace" | "apply_patch" | "propose_edit" => Some("diff".into()),
        _ => {
            let path = summary.split_whitespace().last().unwrap_or("");
            if path.contains('/') || path.contains('.') {
                highlight::lang_from_path(path)
            } else {
                None
            }
        }
    }
}

impl PermissionModal {
    /// A yes/no question, with no "allow for this session": the model asking
    /// to switch hats (`request_hat`), or Ryter asking before an audit spends
    /// money. Its title and the words for yes and no.
    fn question(&self) -> Option<(&'static str, &'static str, &'static str)> {
        match self.tool.as_str() {
            "switch hat" => Some(("switch hat?", "switch", "stay")),
            "audit" => Some(("audit?", "audit", "not now")),
            "audit offer" => Some(("audit this work?", "audit", "not now")),
            _ => None,
        }
    }

    /// Ryter offering an audit after a build turn: `s` stops the offers.
    fn is_offer(&self) -> bool {
        self.tool == "audit offer"
    }

    fn is_hat(&self) -> bool {
        self.question().is_some()
    }

    /// A write outside the project: asked every time, so no `a`.
    fn is_outside(&self) -> bool {
        self.tool.ends_with(ryter_core::tools::OUTSIDE)
    }

    /// Only `y` answers yes.
    fn y_only(&self) -> bool {
        self.strict || self.is_outside()
    }

    fn can_allow_session(&self) -> bool {
        !self.is_hat() && !self.y_only() && self.scope.is_some()
    }

    fn base_tool(&self) -> &str {
        self.tool
            .strip_suffix(ryter_core::tools::OUTSIDE)
            .map_or(self.tool.as_str(), str::trim)
    }

    /// `edit app/server.js`, `run cargo test`.
    fn what(&self) -> String {
        let target = self.summary.lines().next().unwrap_or("").trim().to_string();
        let verb = match self.base_tool() {
            "write" if self.preview.as_ref().is_some_and(|d| d.created) => "create",
            "write" => "rewrite",
            "search_replace" => "edit",
            "propose_edit" => "proposed edit",
            "bash" => "run",
            other => other,
        };
        format!("{verb}  {target}")
    }

    /// What's at stake, in plain words, and whether it's a warning.
    fn risk(&self) -> (String, bool) {
        if self.is_outside() {
            return ("writes outside the project · asked every time".into(), true);
        }
        match self.base_tool() {
            "bash" if self.strict => (
                "deletes, moves, or discards files · /undo may not reach it".into(),
                true,
            ),
            "bash" => (
                "runs a command that can change things · /undo covers files in the project".into(),
                false,
            ),
            "write" if self.preview.as_ref().is_some_and(|d| d.created) => (
                "creates a file in your project · /undo removes it".into(),
                false,
            ),
            "write" | "search_replace" => (
                "changes a file in your project · /undo puts it back".into(),
                false,
            ),
            "propose_edit" => ("edits a file; your yes is the sign-off".into(), false),
            _ => (String::new(), false),
        }
    }

    fn row(label: &str, value: Vec<Span<'static>>, theme: Theme) -> Line<'static> {
        let mut spans = vec![Span::styled(format!(" {label:<8}"), theme.panel_muted())];
        spans.extend(value);
        Line::from(spans)
    }
}

impl Panel for PermissionModal {
    fn kind(&self) -> &'static str {
        "permission"
    }

    fn title(&self, _view: &View) -> String {
        if let Some((title, _, _)) = self.question() {
            title.into()
        } else if self.y_only() {
            "approve? · press y".into()
        } else {
            "approve?".into()
        }
    }

    fn legend(&self, _view: &View) -> String {
        if let Some((_, yes, no)) = self.question() {
            if self.is_offer() {
                format!("⏎ {yes} · n {no} · s stop offering")
            } else {
                format!("⏎ {yes} · n {no}")
            }
        } else if self.y_only() {
            "y allow once · n deny".into()
        } else if let (true, Some(scope)) = (self.can_allow_session(), &self.scope) {
            format!("⏎ allow · a allow {scope} this session · n deny")
        } else {
            "⏎ allow · n deny".into()
        }
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        let mut rows = 2 + usize::from(self.why.is_some()) + usize::from(self.nudge.is_some());
        if let Some(d) = self.preview.as_deref().filter(|d| !d.hunks.is_empty()) {
            rows += 1 + (d.len() + d.hunks.len() - 1).clamp(1, 16);
        } else if !self.is_hat() {
            rows += 1 + self.summary.lines().count().clamp(1, 8);
        } else {
            rows = self.summary.lines().count().clamp(1, 12);
        }
        (110, rows as u16)
    }

    fn modal(&self) -> Option<ModalKind> {
        Some(ModalKind::Permission)
    }

    fn docked(&self) -> bool {
        true
    }

    fn render(&self, _view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let h = usize::from(height).max(3);
        let mut lines: Vec<Line<'static>> = Vec::new();
        if self.is_hat() {
            for row in self
                .summary
                .lines()
                .flat_map(|l| wrap::wrap_plain(l, w.saturating_sub(2)))
            {
                lines.push(widgets::text(&row, theme));
            }
            return Body {
                lines,
                scroll: None,
            };
        }
        let strong = theme.panel().add_modifier(Modifier::BOLD);
        lines.push(Self::row(
            "what",
            vec![Span::styled(
                wrap::truncate(&self.what(), w.saturating_sub(10)),
                strong,
            )],
            theme,
        ));
        if let Some(why) = &self.why {
            lines.push(Self::row(
                "why",
                vec![Span::styled(
                    wrap::truncate(why, w.saturating_sub(10)),
                    theme.panel_muted().add_modifier(Modifier::ITALIC),
                )],
                theme,
            ));
        }
        let (risk, warn) = self.risk();
        if !risk.is_empty() {
            let style = if warn {
                Style::default()
                    .fg(theme.warn)
                    .bg(theme.panel_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                theme.panel()
            };
            lines.push(Self::row("risk", vec![Span::styled(risk, style)], theme));
        }
        let room = h
            .saturating_sub(lines.len() + 1 + usize::from(self.nudge.is_some()))
            .max(1);
        if let Some(diff) = self.preview.as_deref().filter(|d| !d.hunks.is_empty()) {
            let what = if diff.created {
                format!("new file · {} lines", diff.added)
            } else {
                format!("+{} −{}", diff.added, diff.removed)
            };
            lines.push(Self::row(
                "change",
                vec![
                    Span::styled(diff.path.clone(), theme.panel()),
                    Span::styled(format!("  {what}"), theme.panel_muted()),
                ],
                theme,
            ));
            let on_panel = Theme {
                bg: theme.panel_bg,
                ..theme
            };
            let diff_rows = crate::chat::diff::render_folded(
                diff,
                room.saturating_sub(1).max(1),
                w.saturating_sub(1),
                on_panel,
                "/changes shows it whole once it's made",
            );
            for row in diff_rows {
                let mut spans = vec![Span::styled(" ", theme.panel())];
                spans.extend(row.spans);
                lines.push(Line::from(spans));
            }
        } else {
            let body: Vec<String> = self.summary.lines().map(str::to_string).collect();
            let lang = lang_for(self.base_tool(), &self.summary);
            let hl = highlight::highlight(lang.as_deref(), None, &body, theme);
            let max = room.saturating_sub(1).max(1);
            for row in hl.rows.iter().take(max) {
                let mut spans = vec![Span::styled("   ", Style::default().bg(theme.code_bg))];
                let mut used = 3;
                for c in row {
                    let t = wrap::truncate(&c.text, w.saturating_sub(used + 1));
                    used += wrap::width(&t);
                    spans.push(Span::styled(t, c.style.bg(theme.code_bg)));
                }
                spans.push(Span::styled(
                    " ".repeat(w.saturating_sub(used)),
                    Style::default().bg(theme.code_bg),
                ));
                lines.push(Line::from(spans));
            }
            if body.len() > max {
                lines.push(widgets::note(
                    &format!("… {} more lines", body.len() - max),
                    theme,
                ));
            }
        }
        if let Some(n) = self.nudge {
            lines.push(widgets::colored(n, theme.warn, theme));
        }
        Body {
            lines,
            scroll: None,
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        let reply = |p: Permission| Outcome::CloseAct(Action::PermissionReply(p));
        match key.code {
            // Enter answers yes, but not a press that arrived with the
            // prompt: Enter is also the send key, and a prompt can open
            // under a message being typed. Never for destruction an undo
            // may not reach.
            KeyCode::Enter if self.y_only() => {
                self.nudge = Some("Enter doesn't approve this one: press y to allow it, n to deny");
                Outcome::Stay
            }
            KeyCode::Enter if view.now_ms < self.opened_ms + ENTER_GUARD_MS => Outcome::Stay,
            KeyCode::Enter | KeyCode::Char('y' | 'Y') => reply(Permission::Allow),
            KeyCode::Char('n' | 'N') | KeyCode::Esc => reply(Permission::Deny),
            KeyCode::Char('s' | 'S') if self.is_offer() => {
                Outcome::CloseAct(Action::StopAuditOffers)
            }
            // `a` allows this kind of action for the session, named on the
            // card, in one press. It used to allow everything, and needed
            // a second press because of it.
            KeyCode::Char('a' | 'A') if self.can_allow_session() => reply(Permission::Always),
            _ => Outcome::Stay,
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}

/// `ask_user` prompt (`R-POP-78`).
#[derive(Debug, Clone)]
pub struct AskModal {
    /// Question text.
    pub question: String,
    /// Choices; empty = free text via the composer.
    pub options: Vec<String>,
    selected: usize,
    /// Who asks, when it is Ryter and not the model.
    title: Option<String>,
}

impl AskModal {
    /// Build.
    pub fn new(question: String, options: Vec<String>) -> Self {
        Self {
            question,
            options,
            selected: 0,
            title: None,
        }
    }

    /// Ryter's own question, under `title` (`task budget`).
    pub fn titled(mut self, title: Option<String>) -> Self {
        self.title = title;
        self
    }
}

impl Panel for AskModal {
    fn kind(&self) -> &'static str {
        "ask"
    }

    fn title(&self, _view: &View) -> String {
        match &self.title {
            Some(t) => format!("Ryter asks · {t}"),
            None => "the agent asks".into(),
        }
    }

    fn legend(&self, _view: &View) -> String {
        if self.options.is_empty() {
            "type your answer · enter send · esc send empty".into()
        } else {
            "↑↓ or 1-9 choose · enter send · esc send empty".into()
        }
    }

    fn input(&self, _view: &View) -> Option<String> {
        self.options.is_empty().then(|| "answer".to_string())
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        (
            72,
            (self.question.lines().count() + self.options.len() + 3).clamp(4, 18) as u16,
        )
    }

    fn modal(&self) -> Option<ModalKind> {
        Some(ModalKind::Ask)
    }

    fn render(&self, _view: &View, width: u16, _height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let mut lines: Vec<Line<'static>> = Vec::new();
        for para in self.question.lines() {
            for row in wrap::wrap_plain(para, w.saturating_sub(2)) {
                lines.push(widgets::text(&row, theme));
            }
        }
        lines.push(widgets::blank(theme));
        if self.options.is_empty() {
            lines.push(widgets::note("answer in the composer below", theme));
        }
        for (i, o) in self.options.iter().enumerate() {
            let n = if i < 9 {
                format!("{}", i + 1)
            } else {
                " ".into()
            };
            lines.push(widgets::list_row(
                &n,
                o,
                "",
                "",
                i == self.selected,
                w,
                theme,
                Some(theme.accent),
            ));
        }
        Body {
            lines,
            scroll: None,
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        let n = self.options.len();
        match key.code {
            KeyCode::Esc => {
                view.composer.clear();
                Outcome::CloseAct(Action::AskUserReply(String::new()))
            }
            KeyCode::Enter => {
                if n == 0 {
                    let text = view.composer.take();
                    Outcome::CloseAct(Action::AskUserReply(text))
                } else {
                    Outcome::CloseAct(Action::AskUserReply(self.options[self.selected].clone()))
                }
            }
            KeyCode::Up if n > 0 => {
                self.selected = super::step(self.selected, -1, n);
                Outcome::Stay
            }
            KeyCode::Down if n > 0 => {
                self.selected = super::step(self.selected, 1, n);
                Outcome::Stay
            }
            KeyCode::Char(c) if n > 0 && c.is_ascii_digit() && c != '0' => {
                let i = (c as u8 - b'1') as usize;
                if i < n {
                    return Outcome::CloseAct(Action::AskUserReply(self.options[i].clone()));
                }
                Outcome::Stay
            }
            _ => {
                if n == 0 {
                    super::edit_field(&mut view.composer, key);
                }
                Outcome::Stay
            }
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}

/// Generic `y/n` confirmation that runs an action on yes (`R-POP-09`).
#[derive(Debug, Clone)]
pub struct Confirm {
    title: String,
    prose: String,
    action: Action,
}

impl Confirm {
    /// Build.
    pub fn new(title: impl Into<String>, prose: impl Into<String>, action: Action) -> Self {
        Self {
            title: title.into(),
            prose: prose.into(),
            action,
        }
    }
}

impl Panel for Confirm {
    fn kind(&self) -> &'static str {
        "confirm"
    }

    fn title(&self, _view: &View) -> String {
        self.title.clone()
    }

    fn legend(&self, _view: &View) -> String {
        "y confirm · n / esc cancel".into()
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        (60, 4)
    }

    fn modal(&self) -> Option<ModalKind> {
        Some(ModalKind::Ask)
    }

    fn render(&self, _view: &View, width: u16, _height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let mut lines: Vec<Line<'static>> = Vec::new();
        for row in wrap::wrap_plain(&self.prose, w.saturating_sub(2)) {
            lines.push(widgets::text(&row, theme));
        }
        lines.push(widgets::blank(theme));
        lines.push(Line::from(vec![
            Span::styled(
                " y ",
                Style::default()
                    .fg(theme.success)
                    .bg(theme.panel_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("· yes   ", theme.panel()),
            Span::styled(
                "n ",
                Style::default()
                    .fg(theme.error)
                    .bg(theme.panel_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("· no", theme.panel()),
        ]));
        Body {
            lines,
            scroll: None,
        }
    }

    fn key(&mut self, key: KeyEvent, _view: &mut View) -> Outcome {
        match key.code {
            KeyCode::Char('y' | 'Y') | KeyCode::Enter => Outcome::CloseAct(self.action.clone()),
            KeyCode::Char('n' | 'N') | KeyCode::Esc => Outcome::Close,
            _ => Outcome::Stay,
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}

/// Trust-this-project prompt shown at startup when `.ryter/` is untrusted.
#[derive(Debug, Clone, Default)]
pub struct TrustModal {
    selected: usize,
}

impl Panel for TrustModal {
    fn kind(&self) -> &'static str {
        "trust"
    }

    fn title(&self, _view: &View) -> String {
        "trust this project's .ryter/?".into()
    }

    fn legend(&self, _view: &View) -> String {
        "y trust · n skip · enter choose".into()
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        (70, 7)
    }

    fn modal(&self) -> Option<ModalKind> {
        Some(ModalKind::Ask)
    }

    fn render(&self, view: &View, width: u16, _height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let mut lines: Vec<Line<'static>> = Vec::new();
        for row in wrap::wrap_plain(
            &format!(
                "{} contains a .ryter/ directory with skills, commands, or memory. trusted projects can add slash commands and inject RYTER.md into the system prompt.",
                view.cwd
            ),
            w.saturating_sub(2),
        ) {
            lines.push(widgets::text(&row, theme));
        }
        lines.push(widgets::blank(theme));
        lines.push(widgets::list_row(
            "✓",
            "trust",
            "load project skills and memory",
            "",
            self.selected == 0,
            w,
            theme,
            Some(theme.success),
        ));
        lines.push(widgets::list_row(
            "○",
            "skip",
            "user-level config only",
            "",
            self.selected == 1,
            w,
            theme,
            Some(theme.dim),
        ));
        Body {
            lines,
            scroll: None,
        }
    }

    fn key(&mut self, key: KeyEvent, _view: &mut View) -> Outcome {
        match key.code {
            KeyCode::Char('y' | 'Y') => Outcome::CloseAct(Action::TrustProject(true)),
            KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                Outcome::CloseAct(Action::TrustProject(false))
            }
            KeyCode::Up | KeyCode::Down | KeyCode::Tab => {
                self.selected = 1 - self.selected;
                Outcome::Stay
            }
            KeyCode::Enter => Outcome::CloseAct(Action::TrustProject(self.selected == 0)),
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

    /// Ryter's offer of an audit: `s` stops the offers; `a` (allow all)
    /// means nothing, since each audit spends money.
    #[test]
    fn an_audit_offer_can_stop_the_offers_and_has_no_allow_all() {
        use crossterm::event::{KeyEvent, KeyModifiers};
        let mut v = crate::view::View::new(
            ryter_core::Phase::Build,
            "openrouter".into(),
            "m".into(),
            "/tmp".into(),
        );
        let mut m = PermissionModal::new("audit offer".into(), "Audit this work?".into());
        assert!(m.legend(&v).contains("s stop offering"));
        let press = |m: &mut PermissionModal, v: &mut crate::view::View, c: char| {
            m.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE), v)
        };
        assert!(matches!(press(&mut m, &mut v, 'a'), Outcome::Stay));
        assert!(matches!(
            press(&mut m, &mut v, 's'),
            Outcome::CloseAct(Action::StopAuditOffers)
        ));
        let mut asked = PermissionModal::new("audit".into(), "x".into());
        assert!(!asked.legend(&v).contains("stop offering"));
        assert!(matches!(press(&mut asked, &mut v, 's'), Outcome::Stay));
    }
    use crossterm::event::KeyModifiers;

    fn view() -> View {
        View::new(
            ryter_core::Phase::Build,
            "c".into(),
            "m".into(),
            "/tmp".into(),
        )
    }

    fn press(m: &mut PermissionModal, c: char) -> Outcome {
        m.key(
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
            &mut view(),
        )
    }

    /// A hat switch is a yes/no question: no "allow all", which would
    /// approve every later tool call without asking.
    #[test]
    fn a_hat_switch_is_yes_or_no() {
        let v = view();
        let mut m = PermissionModal::new(
            "switch hat".into(),
            "switch to the build hat: carry out the plan".into(),
        );
        assert_eq!(m.title(&v), "switch hat?");
        assert!(!m.legend(&v).contains("allow all"));
        assert!(matches!(press(&mut m, 'a'), Outcome::Stay));
        assert!(
            matches!(press(&mut m, 'a'), Outcome::Stay),
            "no armed allow-all"
        );
        assert!(matches!(
            press(&mut m, 'y'),
            Outcome::CloseAct(Action::PermissionReply(Permission::Allow))
        ));
        // Outside the project: once or not at all.
        let mut o = PermissionModal::new(
            format!("write {}", ryter_core::tools::OUTSIDE),
            "/tmp/scratch.txt".into(),
        );
        assert!(!o.legend(&v).contains("allow all"));
        assert!(matches!(press(&mut o, 'a'), Outcome::Stay));
        assert!(matches!(press(&mut o, 'a'), Outcome::Stay));
        // An ordinary command: `a` allows that kind of command for the
        // session, in one press, and the card says which.
        let mut t = PermissionModal::new("bash".into(), "cargo test".into())
            .with_answers(false, Some("`cargo test` commands".into()));
        assert!(
            t.legend(&v)
                .contains("a allow `cargo test` commands this session")
        );
        assert!(matches!(
            press(&mut t, 'a'),
            Outcome::CloseAct(Action::PermissionReply(Permission::Always))
        ));
        // Destruction an undo may not reach: `y` only.
        let mut d =
            PermissionModal::new("bash".into(), "rm -rf target".into()).with_answers(true, None);
        assert_eq!(d.legend(&v), "y allow once · n deny");
        assert!(matches!(press(&mut d, 'a'), Outcome::Stay));
    }

    /// Enter approves, but not in the first half-second, when it may be a
    /// press meant to send a message; and never a strict prompt.
    #[test]
    fn enter_approves_after_a_moment_and_never_destruction() {
        use crossterm::event::KeyModifiers;
        let mut v = crate::view::View::new(
            ryter_core::Phase::Build,
            "spacexai".into(),
            "grok-4.6".into(),
            "/tmp".into(),
        );
        let enter = |m: &mut PermissionModal, v: &mut crate::view::View| {
            m.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), v)
        };
        v.now_ms = 10_000;
        let mut m = PermissionModal::new("search_replace".into(), "a.rs".into())
            .with_answers(false, Some("edits to files in the project".into()))
            .with_context(Some("Fixing the off-by-one.".into()), 10_000);
        v.now_ms = 10_200;
        assert!(matches!(enter(&mut m, &mut v), Outcome::Stay), "too soon");
        v.now_ms = 10_600;
        assert!(matches!(
            enter(&mut m, &mut v),
            Outcome::CloseAct(Action::PermissionReply(Permission::Allow))
        ));
        let mut d = PermissionModal::new("bash".into(), "git reset --hard".into())
            .with_answers(true, None)
            .with_context(None, 0);
        assert!(matches!(enter(&mut d, &mut v), Outcome::Stay));
        let shown: String = d
            .render(&v, 90, 12, Theme::truecolor_dark())
            .lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
                    + "\n"
            })
            .collect();
        assert!(shown.contains("press y to allow it"), "{shown}");
        assert!(shown.contains("/undo may not reach it"), "{shown}");
    }
}
