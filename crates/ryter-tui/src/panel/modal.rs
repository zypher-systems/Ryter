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

/// The narrowest card that can show a change whole: a line number, its
/// sign, and text beside them.
const MIN_WHOLE_WIDTH: usize = 16;

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
    /// Of a script of several commands, the one that asked: the title and
    /// the `what` row name it, and the body keeps the whole script.
    pub asks: Option<String>,
    /// `view.now_ms` when the prompt opened, for the Enter guard.
    pub opened_ms: u64,
    /// The card is the only view of the change there will be (the user's
    /// rules: the file isn't in the project, so `/changes` never has it).
    /// All of it is here to scroll through, and `y` answers only once its
    /// end has been on screen.
    pub whole: bool,
    /// First row of the change asked for, when it is longer than the card.
    /// Where the window really starts is this, held to `seen`.
    top: usize,
    /// The furthest `top` can go, and the rows of the change that fit, as
    /// last drawn.
    max_top: std::cell::Cell<usize>,
    page: std::cell::Cell<usize>,
    /// Rows of the change drawn so far, counted from its first: rows
    /// `0..seen` have each been in a frame. Only drawing moves it. Keys
    /// arrive in batches between frames, so a count kept by the keys would
    /// run ahead of what was shown.
    seen: std::cell::Cell<usize>,
    /// Every row of the change has been drawn.
    read: std::cell::Cell<bool>,
    /// The whole change as rows, for the width it was last drawn at: a long
    /// one is not laid out again on every frame.
    rows: std::cell::RefCell<Option<(usize, Vec<Line<'static>>)>>,
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
            asks: None,
            opened_ms: 0,
            whole: false,
            top: 0,
            max_top: std::cell::Cell::new(0),
            page: std::cell::Cell::new(1),
            seen: std::cell::Cell::new(0),
            read: std::cell::Cell::new(false),
            rows: std::cell::RefCell::new(None),
            nudge: None,
        }
    }

    /// The card is the only view of the change: show all of it, and take
    /// `y` only once its end has been shown.
    #[must_use]
    pub fn showing_whole(mut self, whole: bool) -> Self {
        self.whole = whole;
        self
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
    /// to switch hats (`request_hat`), or Ryter asking before a review spends
    /// money. Its title and the words for yes and no.
    fn question(&self) -> Option<(&'static str, &'static str, &'static str)> {
        match self.tool.as_str() {
            "switch hat" => Some(("switch hat?", "switch", "stay")),
            "audit" => Some(("audit?", "audit", "not now")),
            _ => None,
        }
    }

    fn is_hat(&self) -> bool {
        self.question().is_some()
    }

    /// A write outside the project: asked every time, so no `a`.
    fn is_outside(&self) -> bool {
        self.tool.ends_with(ryter_core::tools::OUTSIDE)
    }

    /// A write to the project's own `.env`: asked every time, so no `a`.
    fn is_secret(&self) -> bool {
        self.tool.ends_with(ryter_core::tools::SECRET)
    }

    /// Only `y` answers yes.
    fn y_only(&self) -> bool {
        self.strict || self.is_outside() || self.is_secret()
    }

    fn can_allow_session(&self) -> bool {
        !self.is_hat() && !self.y_only() && self.scope.is_some()
    }

    fn base_tool(&self) -> &str {
        self.tool
            .strip_suffix(ryter_core::tools::OUTSIDE)
            .or_else(|| self.tool.strip_suffix(ryter_core::tools::SECRET))
            .map_or(self.tool.as_str(), str::trim)
    }

    /// The command of a script that asked, named first (`R-POP-76`): the
    /// title read `run set -e` for a script whose `rm` three lines down
    /// was the question.
    pub fn asking(mut self, asks: Option<String>) -> Self {
        self.asks = asks.filter(|a| !a.trim().is_empty());
        self
    }

    /// `edit app/server.js`, `run cargo test`.
    fn what(&self) -> String {
        let target = self
            .asks
            .as_deref()
            .or_else(|| self.summary.lines().next())
            .unwrap_or("")
            .trim()
            .to_string();
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
        if self.is_secret() {
            return (
                "writes a secret file: shown to you here, never read by the model · asked every time"
                    .into(),
                true,
            );
        }
        match self.base_tool() {
            // A stack's data is in its volumes, and nothing brings it back.
            "bash" if self.strict && ryter_core::tools::removes_stack_data(&self.summary) => (
                "removes containers' data (volumes) · nothing undoes it".into(),
                true,
            ),
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
        } else if self.is_hat() {
            "allow?".into()
        } else if self.y_only() {
            format!("allow? · {} · press y", self.what())
        } else {
            format!("allow? · {}", self.what())
        }
    }

    /// Top right: the hat that is asking, and, while an Enter pressed
    /// with the card's opening would be ignored, that it is.
    fn status(&self, view: &View) -> String {
        let hat = self.offers().unwrap_or(view.mode).as_str().to_string();
        let guarded = self.question().is_none()
            && !self.y_only()
            && view.now_ms < self.opened_ms + ENTER_GUARD_MS;
        if guarded {
            format!("{hat} · ⏎ in 0.5s")
        } else {
            hat
        }
    }

    fn legend(&self, _view: &View) -> String {
        let mut keys = if let Some((_, yes, no)) = self.question() {
            format!("⏎ {yes} · n {no}")
        } else if self.y_only() {
            "y allow once · n deny".into()
        } else if let (true, Some(scope)) = (self.can_allow_session(), &self.scope) {
            // The kind of action in a word: `edits`, not `edits to files in
            // the project`, so the row holds all of its keys on a column
            // of sixty too. The card's risk line says the rest.
            let scope = scope
                .split(" to ")
                .next()
                .unwrap_or(scope)
                .split(" in ")
                .next()
                .unwrap_or(scope);
            format!("⏎ allow · a allow {scope} this session · n deny")
        } else {
            "⏎ allow · n deny".into()
        };
        if self.max_top.get() > 0 {
            keys.push_str(" · ↑↓ wheel more of the change");
        }
        keys
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        let mut rows = 2 + usize::from(self.why.is_some()) + usize::from(self.nudge.is_some());
        if let Some(d) = self.preview.as_deref().filter(|d| !d.hunks.is_empty()) {
            // A change that must be read whole gets all the room there is.
            let most = if self.whole { 400 } else { 16 };
            rows += 1 + (d.len() + d.hunks.len() - 1).clamp(1, most);
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

    /// Ryter asking before an audit spends money: the card is in the audit
    /// hat's color.
    fn offers(&self) -> Option<ryter_core::Role> {
        match self.tool.as_str() {
            "audit" => Some(ryter_core::Role::SoloAudit),
            _ => None,
        }
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
        // On a short card, a change that must be read whole gets the row
        // the model's reason would take.
        let short = self.whole && usize::from(height) < 6;
        let why_at = (self.why.is_some() && !short).then_some(lines.len());
        if let Some(why) = self.why.as_ref().filter(|_| !short) {
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
        let mut show_nudge = self.nudge.is_some();
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
            let diff_rows = {
                // Every row of it, long lines wrapped, and a window on that
                // when the card is shorter: the keys and the wheel move it.
                // A change that must be read whole is also gated on having
                // been seen to its end.
                let mut laid_out = self.rows.borrow_mut();
                if laid_out.as_ref().is_none_or(|(at, _)| *at != w) {
                    let rows = crate::chat::diff::render_folded(
                        diff,
                        usize::MAX,
                        w.saturating_sub(1),
                        on_panel,
                        "",
                    );
                    *laid_out = Some((w, rows));
                    // Rows are numbered by this layout. What was drawn at
                    // another width is read again from the top.
                    self.seen.set(0);
                }
                let all = laid_out.as_ref().map_or(&[][..], |(_, rows)| rows);
                let total = all.len();
                // The rows really left under the ones above: the height as
                // given. A row that doesn't fit isn't drawn, so it can't
                // count as shown.
                let avail = usize::from(height).saturating_sub(lines.len());
                // The reminder to read on gets a row only where that leaves
                // two for the change: it must not crowd out what it is
                // asking the user to read.
                show_nudge = show_nudge && avail > 2;
                let avail = avail - usize::from(show_nudge);
                // Too narrow for a line number, a sign and some text: the
                // rows would run off the side, so none is shown or counted.
                let avail = if w < MIN_WHOLE_WIDTH { 0 } else { avail };
                if avail == 0 && self.whole && !self.read.get() {
                    // No room for any of it: say so where the path was,
                    // since `y` will do nothing here.
                    lines.truncate(usize::from(height).saturating_sub(1));
                    lines.push(widgets::colored(
                        "no room to show the change · make the window bigger to read it",
                        theme.warn,
                        theme,
                    ));
                } else if avail == 0 {
                    // No row of the change fits: what, risk and the
                    // change's summary stay, and the model's words go
                    // first when something must. Nothing scrolls.
                    if lines.len() > usize::from(height) {
                        if let Some(i) = why_at.filter(|i| *i < lines.len()) {
                            lines.remove(i);
                        }
                    }
                    lines.truncate(usize::from(height));
                }
                let shown = if total <= avail {
                    total
                } else if avail >= 2 {
                    // One row says how much is left.
                    avail - 1
                } else {
                    avail
                };
                // With no row of the change on the card there is nothing to
                // scroll, and the legend must not offer it.
                let max_top = if avail == 0 {
                    0
                } else {
                    total.saturating_sub(shown.max(1))
                };
                // The window starts no further down than what has been
                // drawn, however many keys were pressed since the last
                // frame: nothing is passed over unseen.
                let top = if self.whole {
                    self.top.min(max_top).min(self.seen.get())
                } else {
                    self.top.min(max_top)
                };
                self.max_top.set(max_top);
                self.page.set(shown);
                self.seen.set(self.seen.get().max(top + shown));
                if total > 0 && self.seen.get() >= total {
                    self.read.set(true);
                }
                let mut rows: Vec<Line<'static>> =
                    all.iter().skip(top).take(shown).cloned().collect();
                if total > shown && avail >= 2 {
                    let below = total - (top + shown);
                    let note = match (below > 0, self.whole) {
                        (true, true) => format!(
                            "↓ {below} more row{} · read to the end (↓ PgDn), then y",
                            if below == 1 { "" } else { "s" }
                        ),
                        (true, false) => format!(
                            "↓ {below} more row{} · ↑↓ wheel · /changes shows it whole once it's made",
                            if below == 1 { "" } else { "s" }
                        ),
                        (false, _) => "the end of the change · ↑ PgUp to go back".to_string(),
                    };
                    rows.push(widgets::note(&note, theme));
                }
                rows
            };
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
        if let Some(n) = self.nudge.filter(|_| show_nudge) {
            lines.push(widgets::colored(n, theme.warn, theme));
        }
        // Exactly what fits: the rows of the change were counted to.
        lines.truncate(usize::from(height));
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
            // A change that must be read whole: move through it, a row at a
            // time or a page less a row. Several of these can arrive before
            // the next frame, so none goes past what has been drawn: the
            // next frame starts there at the furthest.
            KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown
                if self.max_top.get() > 0 =>
            {
                let furthest = if self.whole {
                    self.max_top.get().min(self.seen.get())
                } else {
                    self.max_top.get()
                };
                let top = self.top.min(furthest);
                let page = self.page.get().saturating_sub(1).max(1);
                self.top = match key.code {
                    KeyCode::Up => top.saturating_sub(1),
                    KeyCode::Down => top + 1,
                    KeyCode::PageUp => top.saturating_sub(page),
                    _ => top + page,
                }
                .min(furthest);
                self.nudge = None;
                Outcome::Stay
            }
            // And `y` is a yes to all of it, so it answers only once every
            // row has been drawn, and not in the moment the card appeared
            // under whatever was being typed.
            KeyCode::Char('y' | 'Y') if self.whole && !self.read.get() => {
                self.nudge =
                    Some("There's more of this change below: read to its end (↓ PgDn), then y");
                Outcome::Stay
            }
            KeyCode::Char('y' | 'Y')
                if self.whole && view.now_ms < self.opened_ms + ENTER_GUARD_MS =>
            {
                Outcome::Stay
            }
            KeyCode::Enter | KeyCode::Char('y' | 'Y') => reply(Permission::Allow),
            KeyCode::Char('n' | 'N') | KeyCode::Esc => reply(Permission::Deny),
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

/// "Stop the project?": asked on quit while a product Ryter started for a
/// test is still up. Left running, it holds its ports and its containers
/// after Ryter has gone; stopped, it is stopped with the project's own
/// command.
#[derive(Debug, Clone, Default)]
pub struct StopModal;

impl Panel for StopModal {
    fn kind(&self) -> &'static str {
        "stop-product"
    }

    fn title(&self, _view: &View) -> String {
        "stop the project?".into()
    }

    fn legend(&self, _view: &View) -> String {
        "⏎ stop it · n leave it running · esc stay".into()
    }

    fn size(&self, view: &View) -> (u16, u16) {
        let cmd = view
            .product
            .as_ref()
            .and_then(|p| p.stop.as_deref())
            .map_or(0, wrap::width);
        ((cmd + 6).clamp(56, 96) as u16, 4)
    }

    fn modal(&self) -> Option<ModalKind> {
        Some(ModalKind::Ask)
    }

    fn render(&self, view: &View, _width: u16, _height: u16, theme: Theme) -> Body {
        let mut lines: Vec<Line<'static>> = Vec::new();
        if let Some(p) = &view.product {
            let time = p.at.rsplit(' ').next().unwrap_or(&p.at);
            lines.push(widgets::text(
                &format!("Ryter started it for the test at {time}."),
                theme,
            ));
            lines.push(match &p.stop {
                Some(cmd) => widgets::note(cmd, theme),
                None => widgets::note("no stop command: its start command is ended", theme),
            });
        }
        Body {
            lines,
            scroll: None,
        }
    }

    fn key(&mut self, key: KeyEvent, _view: &mut View) -> Outcome {
        match key.code {
            KeyCode::Enter | KeyCode::Char('y' | 'Y') => {
                Outcome::CloseAct(Action::QuitAnswer { stop: true })
            }
            KeyCode::Char('n' | 'N') => Outcome::CloseAct(Action::QuitAnswer { stop: false }),
            KeyCode::Esc => Outcome::Close,
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

    /// Asked for with /audit: yes or no, with no "allow for the session",
    /// since each audit spends money.
    #[test]
    fn an_audit_asks_yes_or_no_only() {
        use crossterm::event::{KeyEvent, KeyModifiers};
        let mut v = crate::view::View::new("openrouter".into(), "m".into(), "/tmp".into());
        let mut m = PermissionModal::new("audit".into(), "x".into());
        assert_eq!(m.title(&v), "audit?");
        assert_eq!(m.legend(&v), "⏎ audit · n not now");
        let press = |m: &mut PermissionModal, v: &mut crate::view::View, c: char| {
            m.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE), v)
        };
        assert!(matches!(press(&mut m, &mut v, 'a'), Outcome::Stay));
        assert!(matches!(press(&mut m, &mut v, 's'), Outcome::Stay));
        assert_eq!(m.offers(), Some(ryter_core::Role::SoloAudit));
    }
    use crossterm::event::KeyModifiers;

    fn view() -> View {
        View::new("c".into(), "m".into(), "/tmp".into())
    }

    fn press(m: &mut PermissionModal, c: char) -> Outcome {
        m.key(
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
            &mut view(),
        )
    }

    /// The title and the `what` row name the command that asked, not the
    /// script's first word: `run set -e` stood for an `rm` two lines down.
    /// The body keeps the whole script.
    #[test]
    fn the_title_names_the_command_that_asked() {
        let v = view();
        let script = "set -e\n.venv/bin/pytest -q\nrm -f \"$TASKS_FILE\"";
        let m = PermissionModal::new("bash".into(), script.into())
            .with_answers(true, None)
            .asking(Some("rm -f \"$TASKS_FILE\"".into()));
        assert_eq!(m.title(&v), "allow? · run  rm -f \"$TASKS_FILE\" · press y");
        assert_eq!(m.summary, script);
        // Nothing named: the first line, as before.
        let first = PermissionModal::new("bash".into(), script.into())
            .with_answers(true, None)
            .asking(Some("  ".into()));
        assert_eq!(first.title(&v), "allow? · run  set -e · press y");
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
        // The project's own `.env`: the same, and the card says what it is.
        let mut e = PermissionModal::new(
            format!("write {}", ryter_core::tools::SECRET),
            ".env".into(),
        );
        assert!(!e.legend(&v).contains("allow all"));
        assert!(matches!(press(&mut e, 'a'), Outcome::Stay));
        assert!(e.risk().0.contains("secret file"), "{}", e.risk().0);
        assert!(e.what().starts_with("rewrite  .env"), "{}", e.what());
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
        let mut v = crate::view::View::new("spacexai".into(), "grok-4.6".into(), "/tmp".into());
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
    fn text(body: &Body) -> Vec<String> {
        body.lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    /// A change to the user's rules, as `update_rules` sends it: the whole
    /// difference, to be read on this card and nowhere else.
    fn rules_card(new: &str, opened_ms: u64) -> PermissionModal {
        let diff =
            ryter_core::diff::FileDiff::whole("~/.ryter/RYTER.md", Some("- Be brief.\n"), new)
                .unwrap();
        PermissionModal::new("update_rules".into(), "change your rules".into())
            .with_preview(Some(Box::new(diff)))
            .with_answers(true, None)
            .showing_whole(true)
            .with_context(None, opened_ms)
    }

    fn key(m: &mut PermissionModal, v: &mut View, code: KeyCode) -> Outcome {
        m.key(KeyEvent::new(code, crossterm::event::KeyModifiers::NONE), v)
    }

    fn is_yes(o: &Outcome) -> bool {
        matches!(
            o,
            Outcome::CloseAct(Action::PermissionReply(Permission::Allow))
        )
    }

    /// A long change to the user's rules is all on the card, and `y` saves
    /// it only once its end has been shown. The card used to fold it to
    /// sixteen rows and point at `/changes`, which never has this file: a
    /// `y` saved lines nobody had been shown.
    #[test]
    fn a_long_rules_change_is_read_to_the_end_before_y() {
        let theme = Theme::truecolor_dark();
        let mut v = view();
        v.now_ms = 10_000;
        let long = format!("- {}the-last-word", "word ".repeat(60));
        let new: String = (0..40)
            .map(|i| format!("- rule number {i};\n"))
            .chain([format!("{long}\n")])
            .collect();
        let mut m = rules_card(&new, 0);
        // Not drawn yet: nothing has been shown.
        assert!(!is_yes(&key(&mut m, &mut v, KeyCode::Char('y'))));
        let (width, height) = (70, 12);
        let first = text(&m.render(&v, width, height, theme));
        assert!(
            first
                .iter()
                .any(|l| l.contains("more rows · read to the end")),
            "{first:?}"
        );
        assert!(!first.iter().any(|l| l.contains("/changes")), "{first:?}");
        assert!(first.len() <= usize::from(height), "{first:?}");
        // `y` at the top answers nothing, and says why.
        assert!(!is_yes(&key(&mut m, &mut v, KeyCode::Char('y'))));
        let nudged = text(&m.render(&v, width, height, theme));
        assert!(
            nudged.iter().any(|l| l.contains("read to its end")),
            "{nudged:?}"
        );
        // Paging to the end passes every row of the change on the way.
        let mut seen: Vec<String> = Vec::new();
        for _ in 0..200 {
            for row in text(&m.render(&v, width, height, theme)) {
                if !seen.contains(&row) {
                    seen.push(row);
                }
            }
            if m.read.get() {
                break;
            }
            assert!(!is_yes(&key(&mut m, &mut v, KeyCode::Char('y'))));
            key(&mut m, &mut v, KeyCode::PageDown);
        }
        assert!(m.read.get(), "the end was never reached");
        let all = seen.join("\n");
        for i in 0..40 {
            assert!(
                all.contains(&format!("- rule number {i};")),
                "rule {i} was skipped:\n{all}"
            );
        }
        // The long line is wrapped, not cut: its last word is there.
        assert!(all.contains("the-last-word"), "{all}");
        assert!(all.contains("the end of the change"), "{all}");
        assert!(!all.contains("/changes"), "{all}");
        assert!(is_yes(&key(&mut m, &mut v, KeyCode::Char('y'))));
    }

    /// A short change is all there at once, and `y` answers, though not in
    /// the moment the card appeared under whatever was being typed.
    #[test]
    fn a_short_rules_change_takes_y_once_it_has_been_up_a_moment() {
        let theme = Theme::truecolor_dark();
        let mut v = view();
        v.now_ms = 10_000;
        let mut m = rules_card("- Be brief.\n- Use British spelling.\n", 10_000);
        let rows = text(&m.render(&v, 90, 12, theme));
        assert!(
            rows.iter().any(|l| l.contains("Use British spelling")),
            "{rows:?}"
        );
        assert!(!rows.iter().any(|l| l.contains("more row")), "{rows:?}");
        v.now_ms = 10_100;
        assert!(
            !is_yes(&key(&mut m, &mut v, KeyCode::Char('y'))),
            "too soon"
        );
        // Enter never answers this card.
        v.now_ms = 11_000;
        assert!(!is_yes(&key(&mut m, &mut v, KeyCode::Enter)));
        assert!(is_yes(&key(&mut m, &mut v, KeyCode::Char('y'))));
        assert!(matches!(
            key(&mut m, &mut v, KeyCode::Char('n')),
            Outcome::CloseAct(Action::PermissionReply(Permission::Deny))
        ));
    }

    /// An edit in the project scrolls on the card, with `/changes` named
    /// for the whole of it, and `y` answers at once: nothing gates it on
    /// having read to the end.
    #[test]
    fn a_project_edit_scrolls_and_answers_at_once() {
        let theme = Theme::truecolor_dark();
        let mut v = view();
        v.now_ms = 10_000;
        let new: String = (0..40).map(|i| format!("line {i}\n")).collect();
        let diff = ryter_core::diff::FileDiff::new("src/a.rs", Some(""), &new);
        let mut m = PermissionModal::new("write".into(), "src/a.rs".into())
            .with_preview(Some(Box::new(diff)))
            .with_answers(false, Some("edits to files in the project".into()))
            .with_context(None, 0);
        let rows = text(&m.render(&v, 90, 12, theme));
        assert!(
            rows.iter().any(|l| l.contains("/changes shows it whole")),
            "{rows:?}"
        );
        // Paging moves the window, and `y` answers wherever it is.
        key(&mut m, &mut v, KeyCode::PageDown);
        let paged = text(&m.render(&v, 90, 12, theme));
        assert_ne!(paged, rows, "{paged:?}");
        assert!(m.legend(&v).contains("wheel more of the change"));
        assert!(is_yes(&key(&mut m, &mut v, KeyCode::Char('y'))));
    }
    /// A card too short for any row of the change keeps what, risk and the
    /// change's summary (the model's words go first), and offers no scroll;
    /// one with two spare rows shows the change and scrolls it.
    #[test]
    fn a_short_card_keeps_the_risk_and_offers_no_scroll() {
        let theme = Theme::truecolor_dark();
        let mut v = view();
        v.now_ms = 10_000;
        let new: String = (0..40).map(|i| format!("line {i}\n")).collect();
        let diff = ryter_core::diff::FileDiff::new("src/a.rs", Some(""), &new);
        let m = PermissionModal::new("write".into(), "src/a.rs".into())
            .with_preview(Some(Box::new(diff)))
            .with_answers(false, Some("edits to files in the project".into()))
            .with_context(Some("because the plan says so".into()), 0);
        // Three body rows: what, risk, change; the why is dropped.
        let rows = text(&m.render(&v, 90, 3, theme));
        let joined = rows.join("\n");
        assert!(joined.contains("what"), "{rows:?}");
        assert!(joined.contains("risk"), "{rows:?}");
        assert!(joined.contains("change"), "{rows:?}");
        assert!(!joined.contains("why"), "{rows:?}");
        assert!(!joined.contains("line 0"), "{rows:?}");
        assert!(
            !m.legend(&v).contains("more of the change"),
            "{}",
            m.legend(&v)
        );
        // Four rows: the why is back, still no change row, still no scroll.
        let rows = text(&m.render(&v, 90, 4, theme));
        assert!(rows.join("\n").contains("why"), "{rows:?}");
        assert!(!m.legend(&v).contains("more of the change"));
        // Six rows: two for the change, and it scrolls.
        let rows = text(&m.render(&v, 90, 6, theme));
        assert!(rows.join("\n").contains("line 0"), "{rows:?}");
        assert!(
            m.legend(&v).contains("more of the change"),
            "{}",
            m.legend(&v)
        );
    }

    fn hundred_rules() -> String {
        (0..100).map(|i| format!("- rule number {i};\n")).collect()
    }

    /// Which of the hundred rules a drawn card shows.
    fn rules_in(rows: &[String]) -> Vec<usize> {
        (0..100)
            .filter(|i| {
                rows.iter()
                    .any(|r| r.contains(&format!("- rule number {i};")))
            })
            .collect()
    }

    /// Keys arrive in batches: the screen takes every key that is waiting
    /// before it draws again. Thirty PgDn presses between two frames used
    /// to land on the last window, which counted as having read to the
    /// end, and `y` then saved the ninety rules in between unseen. The card
    /// counts only rows a frame has drawn, and never starts a window past
    /// them.
    #[test]
    fn keys_pressed_between_frames_skip_nothing() {
        let theme = Theme::truecolor_dark();
        let mut v = view();
        v.now_ms = 10_000;
        let mut m = rules_card(&hundred_rules(), 0);
        let (width, height) = (70, 12);
        let first = rules_in(&text(&m.render(&v, width, height, theme)));
        assert_eq!(first, (0..=7).collect::<Vec<_>>());
        // Thirty presses, and no frame between them.
        for _ in 0..30 {
            key(&mut m, &mut v, KeyCode::PageDown);
        }
        let second = rules_in(&text(&m.render(&v, width, height, theme)));
        assert_eq!(
            second.first(),
            Some(&8),
            "the next frame starts where the last one ended: {second:?}"
        );
        assert!(!is_yes(&key(&mut m, &mut v, KeyCode::Char('y'))));
        // Batches of keys all the way down: every rule is drawn before `y`
        // answers, and `y` answers nothing until then.
        let mut seen = [first, second].concat();
        for _ in 0..200 {
            if m.read.get() {
                break;
            }
            for _ in 0..30 {
                key(&mut m, &mut v, KeyCode::PageDown);
                key(&mut m, &mut v, KeyCode::Down);
                assert!(!is_yes(&key(&mut m, &mut v, KeyCode::Char('y'))));
            }
            seen.extend(rules_in(&text(&m.render(&v, width, height, theme))));
        }
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen, (0..100).collect::<Vec<_>>(), "a rule was never drawn");
        assert!(is_yes(&key(&mut m, &mut v, KeyCode::Char('y'))));
    }

    /// A card too short to show any of the change shows none of it, and
    /// counts none of it: `y` waits for a card that can.
    #[test]
    fn a_card_with_no_room_for_the_change_cannot_be_approved() {
        let theme = Theme::truecolor_dark();
        let mut v = view();
        v.now_ms = 10_000;
        let mut m = rules_card("- Be brief.\n- Use British spelling.\n", 0);
        for height in [0, 1, 2] {
            let rows = text(&m.render(&v, 70, height, theme));
            assert!(
                !rows.iter().any(|r| r.contains("British"))
                    || rows.iter().position(|r| r.contains("British")) >= Some(usize::from(height)),
                "{rows:?}"
            );
            for _ in 0..5 {
                key(&mut m, &mut v, KeyCode::PageDown);
            }
            assert!(
                !is_yes(&key(&mut m, &mut v, KeyCode::Char('y'))),
                "at {height} rows"
            );
        }
        let last = text(&m.render(&v, 70, 2, theme));
        assert!(
            last.iter()
                .any(|r| r.contains("no room to show the change")),
            "{last:?}"
        );
        assert!(last.len() <= 2, "{last:?}");
        // Nor one too narrow for a row of it.
        m.render(&v, 12, 20, theme);
        assert!(
            !is_yes(&key(&mut m, &mut v, KeyCode::Char('y'))),
            "12 columns"
        );
        // One row of it at a time is still all of it, in the end. The
        // reminder that `y` sets off doesn't take that row.
        let mut m = rules_card("- Be brief.\n- Use British spelling.\n- Ask first.\n", 0);
        let mut drawn = Vec::new();
        for _ in 0..10 {
            let rows = text(&m.render(&v, 70, 3, theme));
            assert!(rows.len() <= 3, "{rows:?}");
            drawn.extend(rows);
            if m.read.get() {
                break;
            }
            assert!(!is_yes(&key(&mut m, &mut v, KeyCode::Char('y'))));
            let nudged = text(&m.render(&v, 70, 3, theme));
            assert!(
                !nudged.iter().any(|r| r.contains("no room")) && nudged.len() == 3,
                "{nudged:?}"
            );
            key(&mut m, &mut v, KeyCode::Down);
        }
        let drawn = drawn.join("\n");
        assert!(
            drawn.contains("British") && drawn.contains("Ask first"),
            "{drawn}"
        );
        assert!(is_yes(&key(&mut m, &mut v, KeyCode::Char('y'))));
    }

    /// At another width the rows are different rows, so a change part-read
    /// is read again from the top. One read to the end stays read.
    #[test]
    fn a_resize_part_way_starts_the_reading_again() {
        let theme = Theme::truecolor_dark();
        let mut v = view();
        v.now_ms = 10_000;
        let mut m = rules_card(&hundred_rules(), 0);
        m.render(&v, 70, 12, theme);
        key(&mut m, &mut v, KeyCode::PageDown);
        let before = rules_in(&text(&m.render(&v, 70, 12, theme)));
        assert!(before.first() > Some(&0), "{before:?}");
        // Narrower: the window is back at the first rule.
        let after = rules_in(&text(&m.render(&v, 40, 12, theme)));
        assert_eq!(after.first(), Some(&0), "{after:?}");
        assert!(!is_yes(&key(&mut m, &mut v, KeyCode::Char('y'))));
        // Read to the end, then resized: it has all been shown.
        for _ in 0..200 {
            if m.read.get() {
                break;
            }
            key(&mut m, &mut v, KeyCode::PageDown);
            m.render(&v, 40, 12, theme);
        }
        m.render(&v, 70, 12, theme);
        assert!(is_yes(&key(&mut m, &mut v, KeyCode::Char('y'))));
    }
}
