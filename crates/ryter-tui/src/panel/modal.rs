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

/// Permission prompt (`R-POP-76`, `R-POP-77`).
#[derive(Debug, Clone)]
pub struct PermissionModal {
    /// Tool name.
    pub tool: String,
    /// Argument preview.
    pub summary: String,
    /// `a` pressed once; the warning is showing.
    arm_always: bool,
}

impl PermissionModal {
    /// Build.
    pub fn new(tool: String, summary: String) -> Self {
        Self {
            tool,
            summary,
            arm_always: false,
        }
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
    /// The model asking to switch hats (`request_hat`): a yes/no question,
    /// with no "allow all", which would approve every later tool call.
    fn is_hat(&self) -> bool {
        self.tool == "switch hat"
    }
}

impl Panel for PermissionModal {
    fn kind(&self) -> &'static str {
        "permission"
    }

    fn title(&self, _view: &View) -> String {
        if self.is_hat() {
            "switch hat?".into()
        } else {
            format!("permission · {}", self.tool)
        }
    }

    fn legend(&self, _view: &View) -> String {
        if self.is_hat() {
            "y switch · n stay".into()
        } else {
            "y allow once · n deny · a allow all this session".into()
        }
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        let rows = self.summary.lines().count().clamp(1, 12) + 5;
        (76, rows as u16)
    }

    fn modal(&self) -> Option<ModalKind> {
        Some(ModalKind::Permission)
    }

    fn render(&self, _view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let h = usize::from(height).max(3);
        let mut lines: Vec<Line<'static>> = Vec::new();
        if self.is_hat() {
            for row in wrap::wrap_plain(&self.summary, w.saturating_sub(2)) {
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
                Span::styled("· switch   ", theme.panel()),
                Span::styled(
                    "n ",
                    Style::default()
                        .fg(theme.error)
                        .bg(theme.panel_bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("· stay", theme.panel()),
            ]));
            return Body {
                lines,
                scroll: None,
            };
        }
        lines.push(Line::from(vec![
            Span::styled(" tool  ", theme.panel_muted()),
            Span::styled(
                self.tool.clone(),
                theme.panel().add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(widgets::blank(theme));
        let body: Vec<String> = self.summary.lines().map(str::to_string).collect();
        let lang = lang_for(&self.tool, &self.summary);
        let hl = highlight::highlight(lang.as_deref(), None, &body, theme);
        let max_preview = h.saturating_sub(5).max(1);
        for row in hl.rows.iter().take(max_preview) {
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
        if body.len() > max_preview {
            lines.push(widgets::note(
                &format!("… {} more lines", body.len() - max_preview),
                theme,
            ));
        }
        lines.push(widgets::blank(theme));
        if self.arm_always {
            lines.push(widgets::colored(
                "allow all: every further tool call this session runs without asking, including destructive ones. press a again to confirm.",
                theme.warn,
                theme,
            ));
        } else {
            lines.push(Line::from(vec![
                Span::styled(
                    " y ",
                    Style::default()
                        .fg(theme.success)
                        .bg(theme.panel_bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("· allow   ", theme.panel()),
                Span::styled(
                    "n ",
                    Style::default()
                        .fg(theme.error)
                        .bg(theme.panel_bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("· deny   ", theme.panel()),
                Span::styled(
                    "a ",
                    Style::default()
                        .fg(theme.warn)
                        .bg(theme.panel_bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("· allow all this session", theme.panel()),
            ]));
        }
        Body {
            lines,
            scroll: None,
        }
    }

    fn key(&mut self, key: KeyEvent, _view: &mut View) -> Outcome {
        match key.code {
            // `Enter` is deliberately not an alias for allow: it is the send
            // key in the composer, so a reflex press must never approve a
            // destructive call. Only `y` allows (`R-POP-75`).
            KeyCode::Char('y' | 'Y') => {
                Outcome::CloseAct(Action::PermissionReply(Permission::Allow))
            }
            KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                Outcome::CloseAct(Action::PermissionReply(Permission::Deny))
            }
            KeyCode::Char('a' | 'A') if self.is_hat() => Outcome::Stay,
            KeyCode::Char('a' | 'A') => {
                if self.arm_always {
                    Outcome::CloseAct(Action::PermissionReply(Permission::Always))
                } else {
                    self.arm_always = true;
                    Outcome::Stay
                }
            }
            _ => {
                self.arm_always = false;
                Outcome::Stay
            }
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
}

impl AskModal {
    /// Build.
    pub fn new(question: String, options: Vec<String>) -> Self {
        Self {
            question,
            options,
            selected: 0,
        }
    }
}

impl Panel for AskModal {
    fn kind(&self) -> &'static str {
        "ask"
    }

    fn title(&self, _view: &View) -> String {
        "the agent asks".into()
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
        // Tool permissions keep allow-all.
        let mut t = PermissionModal::new("bash".into(), "rm -rf target".into());
        assert!(t.legend(&v).contains("allow all"));
        press(&mut t, 'a');
        assert!(matches!(
            press(&mut t, 'a'),
            Outcome::CloseAct(Action::PermissionReply(Permission::Always))
        ));
    }
}
