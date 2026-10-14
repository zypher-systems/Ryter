//! `/permissions` — durable gate rules: `edit` and `[permissions.bash]`.
//!
//! Saved to `~/.ryter/permissions.toml` (not a rewrite of `config.toml`). A
//! rule moves an ask either way and never opens a refusal (secrets, sudo).
//! Session `/tools` ask/always/yolo is a different panel.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ryter_core::permissions::{Answer, Permissions as Rules};

use super::{Body, Outcome, Panel, widgets};
use crate::action::Action;
use crate::chat::wrap;
use crate::theme::Theme;
use crate::view::View;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Pane {
    Home,
    /// Typing a new bash pattern.
    Add,
    /// Asked whether to remove bash rule at index `.0`.
    ConfirmRemove(usize),
}

/// The `/permissions` panel.
#[derive(Debug, Clone)]
pub struct Permissions {
    /// Row 0 is `edit`; the rest are bash patterns in sorted order.
    selected: usize,
    pane: Pane,
    error: Option<String>,
    /// Opened from an allow card with this draft pattern.
    draft: Option<String>,
}

impl Permissions {
    /// Build from the live view. `draft` opens the add field with that text.
    pub fn new(view: &View, draft: Option<String>) -> Self {
        let mut panel = Self {
            selected: 0,
            pane: Pane::Home,
            error: None,
            draft: None,
        };
        if let Some(d) = draft.filter(|s| !s.trim().is_empty()) {
            panel.draft = Some(d);
            panel.pane = Pane::Add;
        }
        let _ = view; // reserved for loading from disk later
        panel
    }

    fn bash_keys(rules: &Rules) -> Vec<String> {
        rules.bash.keys().cloned().collect()
    }

    fn rows(rules: &Rules) -> usize {
        1 + rules.bash.len()
    }

    fn edit_label(edit: Option<Answer>) -> &'static str {
        match edit {
            None => "default (allow)",
            Some(Answer::Allow) => "allow",
            Some(Answer::Ask) => "ask",
            Some(Answer::Deny) => "deny",
        }
    }

    fn answer_label(a: Answer) -> &'static str {
        match a {
            Answer::Allow => "allow",
            Answer::Ask => "ask",
            Answer::Deny => "deny",
        }
    }

    fn cycle_edit(cur: Option<Answer>) -> Option<Answer> {
        match cur {
            None => Some(Answer::Ask),
            Some(Answer::Ask) => Some(Answer::Allow),
            Some(Answer::Allow) => None,
            Some(Answer::Deny) => None,
        }
    }

    fn cycle_answer(a: Answer) -> Answer {
        match a {
            Answer::Allow => Answer::Ask,
            Answer::Ask => Answer::Deny,
            Answer::Deny => Answer::Allow,
        }
    }

    fn save() -> Outcome {
        Outcome::Act(Action::SavePermissions)
    }

    fn parse_add(typed: &str) -> Result<(String, Answer), String> {
        let typed = typed.trim();
        if typed.is_empty() {
            return Err("type a pattern, then enter".into());
        }
        let (pattern, answer) = if let Some((p, a)) = typed.rsplit_once('=') {
            let a = a.trim().to_ascii_lowercase();
            let answer = match a.as_str() {
                "allow" | "yes" => Answer::Allow,
                "ask" => Answer::Ask,
                "deny" | "no" => Answer::Deny,
                _ => {
                    return Err("answer is allow, ask, or deny (after =)".into());
                }
            };
            (p.trim().to_string(), answer)
        } else {
            (typed.to_string(), Answer::Allow)
        };
        if pattern.is_empty() {
            return Err("type a pattern, then enter".into());
        }
        if pattern.contains('\n') {
            return Err("one pattern per rule".into());
        }
        Ok((pattern, answer))
    }
}

impl Panel for Permissions {
    fn kind(&self) -> &'static str {
        "permissions"
    }

    fn title(&self, _view: &View) -> String {
        "permissions".into()
    }

    fn status(&self, view: &View) -> String {
        let n = view.permissions.bash.len();
        let edit = Self::edit_label(view.permissions.edit);
        if n == 0 {
            format!("edit {edit}")
        } else {
            format!("edit {edit} · {n} rule{}", if n == 1 { "" } else { "s" })
        }
    }

    fn legend(&self, _view: &View) -> String {
        match self.pane {
            Pane::Home => "↑↓ move · ←→/space answer · a add · d remove · esc close".into(),
            Pane::Add => "type pattern [= allow|ask|deny] · enter save · esc back".into(),
            Pane::ConfirmRemove(_) => "y remove · n keep".into(),
        }
    }

    fn input(&self, _view: &View) -> Option<String> {
        matches!(self.pane, Pane::Add).then(|| "pattern".into())
    }

    fn prefill(&self, _view: &View) -> Option<String> {
        match (&self.pane, &self.draft) {
            (Pane::Add, Some(d)) => Some(d.clone()),
            _ => None,
        }
    }

    fn size(&self, view: &View) -> (u16, u16) {
        let n = Self::rows(&view.permissions);
        (78, (n + 8).clamp(12, 22) as u16)
    }

    fn render(&self, view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let h = usize::from(height).max(1);
        let rules = &view.permissions;
        let dim = theme.panel_muted();
        let mut lines: Vec<Line<'static>> = Vec::new();

        lines.push(widgets::note(
            "durable allows without yolo · ~/.ryter/permissions.toml · can't open a refusal",
            theme,
        ));
        lines.push(widgets::blank(theme));

        let mut foot: Vec<Line<'static>> = Vec::new();
        if let Pane::ConfirmRemove(i) = self.pane {
            let keys = Self::bash_keys(rules);
            if let Some(pat) = keys.get(i) {
                foot.push(widgets::colored(
                    &wrap::truncate(&format!("remove \"{pat}\"?"), w.saturating_sub(2)),
                    theme.warn,
                    theme,
                ));
            }
        }
        if let Some(e) = &self.error {
            foot.push(widgets::colored(&format!("✕ {e}"), theme.error, theme));
        }

        let mut rows: Vec<Line<'static>> = Vec::new();
        // edit
        {
            let sel = self.selected == 0;
            let bg = if sel {
                theme.selection_bg
            } else {
                theme.panel_bg
            };
            let mark = if sel { "›" } else { " " };
            let label = Self::edit_label(rules.edit);
            let body = format!(" edit of project files          {label}");
            let used = 1 + wrap::width(&body);
            rows.push(Line::from(vec![
                Span::styled(mark, Style::default().fg(theme.accent).bg(bg)),
                Span::styled(
                    body,
                    Style::default()
                        .fg(theme.fg)
                        .bg(bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" ".repeat(w.saturating_sub(used)), Style::default().bg(bg)),
            ]));
        }

        let keys = Self::bash_keys(rules);
        if keys.is_empty() {
            rows.push(widgets::blank(theme));
            for row in wrap::wrap_plain(
                "No bash rules yet. a adds one (e.g. rm -rf target, or git push* = allow). \
                 The most specific pattern wins.",
                w.saturating_sub(4),
            ) {
                rows.push(Line::from(Span::styled(format!("  {row}"), dim)));
            }
        } else {
            rows.push(Line::from(Span::styled(
                "  bash patterns".to_string(),
                dim.add_modifier(Modifier::BOLD),
            )));
            for (i, pat) in keys.iter().enumerate() {
                let sel = self.selected == i + 1;
                let bg = if sel {
                    theme.selection_bg
                } else {
                    theme.panel_bg
                };
                let mark = if sel { "›" } else { " " };
                let ans = rules.bash.get(pat).copied().unwrap_or(Answer::Allow);
                let ans_s = Self::answer_label(ans);
                let color = match ans {
                    Answer::Allow => theme.success,
                    Answer::Ask => theme.warn,
                    Answer::Deny => theme.error,
                };
                let room = w.saturating_sub(2 + 1 + wrap::width(ans_s) + 2).max(8);
                let shown = wrap::truncate(pat, room);
                let body = format!(" {shown}");
                let pad = w.saturating_sub(1 + wrap::width(&body) + 1 + wrap::width(ans_s));
                rows.push(Line::from(vec![
                    Span::styled(mark, Style::default().fg(theme.accent).bg(bg)),
                    Span::styled(body, Style::default().fg(theme.fg).bg(bg)),
                    Span::styled(" ".repeat(pad.max(1)), Style::default().bg(bg)),
                    Span::styled(
                        ans_s.to_string(),
                        Style::default()
                            .fg(color)
                            .bg(bg)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(" ", Style::default().bg(bg)),
                ]));
            }
        }

        let room = h.saturating_sub(lines.len() + foot.len()).max(1);
        let total = rows.len();
        let at = if self.selected == 0 {
            0
        } else {
            // heading + edit + blank? keep selection near the bash row
            self.selected.min(total.saturating_sub(1))
        };
        let first = super::window(at, total, room);
        lines.extend(rows.into_iter().skip(first).take(room));
        lines.extend(foot);
        Body {
            lines,
            scroll: (total > room).then_some((first, total)),
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        match self.pane.clone() {
            Pane::Home => match key.code {
                KeyCode::Esc => Outcome::Close,
                KeyCode::Up => {
                    let n = Self::rows(&view.permissions);
                    self.selected = super::step(self.selected, -1, n);
                    Outcome::Stay
                }
                KeyCode::Down => {
                    let n = Self::rows(&view.permissions);
                    self.selected = super::step(self.selected, 1, n);
                    Outcome::Stay
                }
                KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') => {
                    self.error = None;
                    if self.selected == 0 {
                        view.permissions.edit = Self::cycle_edit(view.permissions.edit);
                        Self::save()
                    } else {
                        let keys = Self::bash_keys(&view.permissions);
                        let i = self.selected - 1;
                        if let Some(pat) = keys.get(i).cloned() {
                            if let Some(a) = view.permissions.bash.get_mut(&pat) {
                                *a = Self::cycle_answer(*a);
                            }
                            Self::save()
                        } else {
                            Outcome::Stay
                        }
                    }
                }
                KeyCode::Char('a' | 'A') => {
                    self.error = None;
                    self.draft = None;
                    view.composer.clear();
                    self.pane = Pane::Add;
                    Outcome::Stay
                }
                KeyCode::Char('d' | 'D') => {
                    self.error = None;
                    if self.selected == 0 {
                        if view.permissions.edit.is_some() {
                            view.permissions.edit = None;
                            return Self::save();
                        }
                        return Outcome::Stay;
                    }
                    let i = self.selected - 1;
                    if i < view.permissions.bash.len() {
                        self.pane = Pane::ConfirmRemove(i);
                    }
                    Outcome::Stay
                }
                _ => Outcome::Stay,
            },
            Pane::Add => match key.code {
                KeyCode::Esc => {
                    view.composer.clear();
                    self.draft = None;
                    self.pane = Pane::Home;
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    let typed = view.composer.text().to_string();
                    match Self::parse_add(&typed) {
                        Ok((pattern, answer)) => {
                            view.permissions.bash.insert(pattern.clone(), answer);
                            self.error = None;
                            view.composer.clear();
                            self.draft = None;
                            self.pane = Pane::Home;
                            // Select the new (or updated) rule.
                            let keys = Self::bash_keys(&view.permissions);
                            self.selected = keys
                                .iter()
                                .position(|k| k == &pattern)
                                .map(|i| i + 1)
                                .unwrap_or(0);
                            view.system(format!(
                                "permissions · {pattern} = {}",
                                Self::answer_label(answer)
                            ));
                            Self::save()
                        }
                        Err(e) => {
                            self.error = Some(e);
                            Outcome::Stay
                        }
                    }
                }
                _ => {
                    super::edit_field(&mut view.composer, key);
                    self.error = None;
                    Outcome::Stay
                }
            },
            Pane::ConfirmRemove(i) => match key.code {
                KeyCode::Char('y' | 'Y') => {
                    let keys = Self::bash_keys(&view.permissions);
                    if let Some(pat) = keys.get(i).cloned() {
                        view.permissions.bash.remove(&pat);
                        view.system(format!("permissions · removed {pat}"));
                        let n = Self::rows(&view.permissions);
                        self.selected = self.selected.min(n.saturating_sub(1));
                    }
                    self.pane = Pane::Home;
                    Self::save()
                }
                KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                    self.pane = Pane::Home;
                    Outcome::Stay
                }
                _ => Outcome::Stay,
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

    fn view() -> View {
        View::new("c".into(), "m".into(), "p".into())
    }

    fn press(p: &mut Permissions, v: &mut View, code: KeyCode) -> Outcome {
        p.key(KeyEvent::new(code, KeyModifiers::NONE), v)
    }

    fn text(p: &Permissions, v: &View) -> String {
        p.render(v, 72, 16, Theme::truecolor_dark())
            .lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn add_allow_rule_and_cycle_answer() {
        let mut v = view();
        let mut p = Permissions::new(&v, None);
        assert!(text(&p, &v).contains("No bash rules yet"));
        press(&mut p, &mut v, KeyCode::Char('a'));
        assert_eq!(p.input(&v).as_deref(), Some("pattern"));
        for c in "rm -rf target".chars() {
            press(&mut p, &mut v, KeyCode::Char(c));
        }
        let out = press(&mut p, &mut v, KeyCode::Enter);
        assert!(matches!(out, Outcome::Act(Action::SavePermissions)));
        assert_eq!(
            v.permissions.bash.get("rm -rf target"),
            Some(&Answer::Allow)
        );
        // selected on the new rule; space cycles to ask
        let out = press(&mut p, &mut v, KeyCode::Char(' '));
        assert!(matches!(out, Outcome::Act(Action::SavePermissions)));
        assert_eq!(v.permissions.bash.get("rm -rf target"), Some(&Answer::Ask));
    }

    #[test]
    fn draft_opens_add_with_prefill() {
        let v = view();
        let p = Permissions::new(&v, Some("git push*".into()));
        assert_eq!(p.input(&v).as_deref(), Some("pattern"));
        assert_eq!(p.prefill(&v).as_deref(), Some("git push*"));
    }

    #[test]
    fn parse_add_accepts_answer_suffix() {
        assert_eq!(
            Permissions::parse_add("git push* = deny").unwrap(),
            ("git push*".into(), Answer::Deny)
        );
        assert_eq!(
            Permissions::parse_add("rm -rf node_modules").unwrap(),
            ("rm -rf node_modules".into(), Answer::Allow)
        );
    }

    #[test]
    fn edit_cycles_default_ask_allow() {
        let mut v = view();
        let mut p = Permissions::new(&v, None);
        assert_eq!(v.permissions.edit, None);
        press(&mut p, &mut v, KeyCode::Char(' '));
        assert_eq!(v.permissions.edit, Some(Answer::Ask));
        press(&mut p, &mut v, KeyCode::Char(' '));
        assert_eq!(v.permissions.edit, Some(Answer::Allow));
        press(&mut p, &mut v, KeyCode::Char(' '));
        assert_eq!(v.permissions.edit, None);
    }
}
