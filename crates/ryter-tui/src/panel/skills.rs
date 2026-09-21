//! `/skills` — skills and user commands (`R-POP-56`, `R-POP-57`, `R-POP-60`).

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::text::Line;

use super::{Body, Outcome, Panel, widgets};
use crate::action::Action;
use crate::chat::wrap;
use crate::theme::Theme;
use crate::view::View;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Pane {
    Home,
    /// Arguments for a run.
    Args {
        name: String,
    },
    /// New skill: name → description → review.
    SkillName,
    SkillDesc {
        name: String,
    },
    SkillReview {
        name: String,
        description: String,
    },
    /// New command: name → review.
    CmdName,
    CmdReview {
        name: String,
    },
    ConfirmRemove(PathBuf),
}

/// Skills panel.
#[derive(Debug, Clone)]
pub struct Skills {
    pane: Pane,
    selected: usize,
    error: Option<String>,
}

impl Default for Skills {
    fn default() -> Self {
        Self {
            pane: Pane::Home,
            selected: 0,
            error: None,
        }
    }
}

impl Skills {
    fn home_len(view: &View) -> usize {
        view.catalog.skills.len() + view.catalog.commands.len() + 2
    }

    fn source_at(view: &View, i: usize) -> Option<PathBuf> {
        let ns = view.catalog.skills.len();
        if i < ns {
            return Some(view.catalog.skills[i].source.clone());
        }
        view.catalog.commands.get(i - ns).map(|c| c.source.clone())
    }

    fn name_at(view: &View, i: usize) -> Option<String> {
        let ns = view.catalog.skills.len();
        if i < ns {
            return Some(view.catalog.skills[i].name.clone());
        }
        view.catalog.commands.get(i - ns).map(|c| c.name.clone())
    }

    fn go(&mut self, view: &mut View, pane: Pane) {
        view.composer.clear();
        self.error = None;
        self.pane = pane;
    }

    fn step_of(pane: &Pane) -> Option<(usize, usize, &'static str)> {
        match pane {
            Pane::SkillName => Some((1, 3, "skill name")),
            Pane::SkillDesc { .. } => Some((2, 3, "one-line description")),
            Pane::SkillReview { .. } => Some((3, 3, "review")),
            Pane::CmdName => Some((1, 2, "command name")),
            Pane::CmdReview { .. } => Some((2, 2, "review")),
            _ => None,
        }
    }
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn short_path(p: &std::path::Path) -> String {
    let s = p.display().to_string();
    match std::env::var("HOME") {
        Ok(h) if s.starts_with(&h) => format!("~{}", &s[h.len()..]),
        _ => s,
    }
}

impl Panel for Skills {
    fn kind(&self) -> &'static str {
        "skills"
    }

    fn title(&self, _view: &View) -> String {
        match &self.pane {
            Pane::Home => "skills & commands".into(),
            Pane::Args { name } => format!("/{name} arguments"),
            Pane::SkillName | Pane::SkillDesc { .. } | Pane::SkillReview { .. } => {
                "new skill".into()
            }
            Pane::CmdName | Pane::CmdReview { .. } => "new command".into(),
            Pane::ConfirmRemove(_) => "remove".into(),
        }
    }

    fn status(&self, view: &View) -> String {
        match Self::step_of(&self.pane) {
            Some((s, of, _)) => format!("step {s} of {of}"),
            None if matches!(self.pane, Pane::Home) => {
                format!(
                    "{} skills · {} commands",
                    view.catalog.skills.len(),
                    view.catalog.commands.len()
                )
            }
            None => String::new(),
        }
    }

    fn legend(&self, _view: &View) -> String {
        match &self.pane {
            Pane::Home => "enter run · e edit in $EDITOR · d remove · esc".into(),
            Pane::Args { .. } => "type arguments (optional) · enter run · esc back".into(),
            Pane::SkillReview { .. } | Pane::CmdReview { .. } => {
                "enter write file · esc back".into()
            }
            Pane::ConfirmRemove(_) => "type `remove` · enter · esc".into(),
            _ => "type · enter next · esc back".into(),
        }
    }

    fn input(&self, _view: &View) -> Option<String> {
        match &self.pane {
            Pane::Home | Pane::SkillReview { .. } | Pane::CmdReview { .. } => None,
            Pane::Args { .. } => Some("arguments".into()),
            Pane::ConfirmRemove(_) => Some("confirm".into()),
            p => Self::step_of(p).map(|(_, _, t)| t.to_string()),
        }
    }

    fn size(&self, view: &View) -> (u16, u16) {
        (78, (Self::home_len(view) + 4).clamp(8, 22) as u16)
    }

    fn render(&self, view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let h = usize::from(height).max(1);
        let mut lines: Vec<Line<'static>> = Vec::new();
        match &self.pane {
            Pane::Home | Pane::ConfirmRemove(_) => {
                let n = Self::home_len(view);
                let first = super::window(self.selected, n, h.saturating_sub(1).max(1));
                let ns = view.catalog.skills.len();
                for i in first..n.min(first + h) {
                    let sel = i == self.selected;
                    if i < ns {
                        let s = &view.catalog.skills[i];
                        lines.push(widgets::list_row(
                            "◆",
                            &format!("/{}", s.name),
                            &s.description,
                            &short_path(&s.source),
                            sel,
                            w,
                            theme,
                            Some(theme.accent),
                        ));
                    } else if i < ns + view.catalog.commands.len() {
                        let c = &view.catalog.commands[i - ns];
                        lines.push(widgets::list_row(
                            "◇",
                            &format!("/{}", c.name),
                            &c.description,
                            &short_path(&c.source),
                            sel,
                            w,
                            theme,
                            None,
                        ));
                    } else if i == ns + view.catalog.commands.len() {
                        lines.push(widgets::list_row(
                            "+",
                            "new skill",
                            "SKILL.md with a body the model reads",
                            "",
                            sel,
                            w,
                            theme,
                            Some(theme.accent),
                        ));
                    } else {
                        lines.push(widgets::list_row(
                            "+",
                            "new command",
                            "template with $ARGUMENTS",
                            "",
                            sel,
                            w,
                            theme,
                            Some(theme.accent),
                        ));
                    }
                }
                if let Pane::ConfirmRemove(p) = &self.pane {
                    lines.push(widgets::blank(theme));
                    lines.push(widgets::colored(
                        &format!("delete {}?", short_path(p)),
                        theme.warn,
                        theme,
                    ));
                }
                return Body {
                    lines,
                    scroll: (n > h).then_some((first, n)),
                };
            }
            Pane::Args { name } => {
                lines.push(widgets::text(
                    &format!("/{name} runs as the next user turn"),
                    theme,
                ));
                lines.push(widgets::note(
                    "arguments are appended (skills) or replace $ARGUMENTS (commands)",
                    theme,
                ));
            }
            Pane::SkillName | Pane::SkillDesc { .. } | Pane::CmdName => {
                if let Some((s, of, t)) = Self::step_of(&self.pane) {
                    lines.push(widgets::step_line(s, of, t, theme));
                }
                lines.push(widgets::blank(theme));
                lines.push(widgets::text(
                    match &self.pane {
                        Pane::SkillName | Pane::CmdName => "lowercase; letters, digits, - and _",
                        _ => "shown in the palette next to the name",
                    },
                    theme,
                ));
            }
            Pane::SkillReview { name, description } => {
                lines.push(widgets::step_line(3, 3, "review", theme));
                lines.push(widgets::blank(theme));
                lines.push(widgets::note(
                    &format!("will write ~/.ryter/skills/{name}/SKILL.md:"),
                    theme,
                ));
                for row in [
                    "---".to_string(),
                    format!("name: {name}"),
                    format!("description: {description}"),
                    "user_invocable: true".into(),
                    "---".into(),
                    "(body: edit after writing)".into(),
                ] {
                    lines.push(widgets::colored(
                        &wrap::truncate(&row, w.saturating_sub(2)),
                        theme.code_fg,
                        theme,
                    ));
                }
            }
            Pane::CmdReview { name } => {
                lines.push(widgets::step_line(2, 2, "review", theme));
                lines.push(widgets::blank(theme));
                lines.push(widgets::note(
                    &format!("will write ~/.ryter/commands/{name}.md"),
                    theme,
                ));
                lines.push(widgets::colored(
                    "# $ARGUMENTS is replaced on invoke",
                    theme.code_fg,
                    theme,
                ));
            }
        }
        if let Some(e) = &self.error {
            lines.push(widgets::blank(theme));
            lines.push(widgets::colored(&format!("✕ {e}"), theme.error, theme));
        }
        Body {
            lines,
            scroll: None,
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        match self.pane.clone() {
            Pane::Home => {
                let n = Self::home_len(view);
                let ns = view.catalog.skills.len();
                let nc = view.catalog.commands.len();
                match key.code {
                    KeyCode::Esc => Outcome::Close,
                    KeyCode::Up => {
                        self.selected = super::step(self.selected, -1, n);
                        Outcome::Stay
                    }
                    KeyCode::Down => {
                        self.selected = super::step(self.selected, 1, n);
                        Outcome::Stay
                    }
                    KeyCode::Enter => {
                        if self.selected < ns + nc {
                            let name = Self::name_at(view, self.selected).unwrap_or_default();
                            self.go(view, Pane::Args { name });
                        } else if self.selected == ns + nc {
                            self.go(view, Pane::SkillName);
                        } else {
                            self.go(view, Pane::CmdName);
                        }
                        Outcome::Stay
                    }
                    KeyCode::Char('e') => match Self::source_at(view, self.selected) {
                        Some(p) => Outcome::Act(Action::EditCatalog(p)),
                        None => Outcome::Stay,
                    },
                    KeyCode::Char('d') => {
                        if let Some(p) = Self::source_at(view, self.selected) {
                            self.go(view, Pane::ConfirmRemove(p));
                        }
                        Outcome::Stay
                    }
                    _ => Outcome::Stay,
                }
            }
            Pane::ConfirmRemove(p) => match key.code {
                KeyCode::Esc => {
                    self.go(view, Pane::Home);
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    if view.composer.text().trim() == "remove" {
                        self.go(view, Pane::Home);
                        self.selected = 0;
                        Outcome::Act(Action::RemoveCatalog(p))
                    } else {
                        Outcome::Stay
                    }
                }
                _ => {
                    super::edit_field(&mut view.composer, key);
                    Outcome::Stay
                }
            },
            Pane::Args { name } => match key.code {
                KeyCode::Esc => {
                    self.go(view, Pane::Home);
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    let args = view.composer.text().trim().to_string();
                    let Some(expanded) = view.catalog.expand(&name, &args) else {
                        self.error = Some("not found".into());
                        return Outcome::Stay;
                    };
                    let shown = if args.is_empty() {
                        format!("/{name}")
                    } else {
                        format!("/{name} {args}")
                    };
                    view.composer.clear();
                    Outcome::CloseAct(view.submit_user(shown, expanded))
                }
                _ => {
                    super::edit_field(&mut view.composer, key);
                    Outcome::Stay
                }
            },
            Pane::SkillName => match key.code {
                KeyCode::Esc => {
                    self.go(view, Pane::Home);
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    let name = view.composer.text().trim().to_ascii_lowercase();
                    if !valid_name(&name) {
                        self.error = Some("name: letters, digits, - and _".into());
                        return Outcome::Stay;
                    }
                    self.go(view, Pane::SkillDesc { name });
                    Outcome::Stay
                }
                _ => {
                    super::edit_field(&mut view.composer, key);
                    self.error = None;
                    Outcome::Stay
                }
            },
            Pane::SkillDesc { name } => match key.code {
                KeyCode::Esc => {
                    self.go(view, Pane::SkillName);
                    view.composer.set_text(&name);
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    let description = view.composer.text().trim().to_string();
                    self.go(view, Pane::SkillReview { name, description });
                    Outcome::Stay
                }
                _ => {
                    super::edit_field(&mut view.composer, key);
                    Outcome::Stay
                }
            },
            Pane::SkillReview { name, description } => match key.code {
                KeyCode::Esc => {
                    self.go(view, Pane::SkillDesc { name });
                    view.composer.set_text(&description);
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    self.go(view, Pane::Home);
                    Outcome::Act(Action::WriteSkill { name, description })
                }
                _ => Outcome::Stay,
            },
            Pane::CmdName => match key.code {
                KeyCode::Esc => {
                    self.go(view, Pane::Home);
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    let name = view.composer.text().trim().to_ascii_lowercase();
                    if !valid_name(&name) {
                        self.error = Some("name: letters, digits, - and _".into());
                        return Outcome::Stay;
                    }
                    self.go(view, Pane::CmdReview { name });
                    Outcome::Stay
                }
                _ => {
                    super::edit_field(&mut view.composer, key);
                    self.error = None;
                    Outcome::Stay
                }
            },
            Pane::CmdReview { name } => match key.code {
                KeyCode::Esc => {
                    self.go(view, Pane::CmdName);
                    view.composer.set_text(&name);
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    self.go(view, Pane::Home);
                    Outcome::Act(Action::WriteCommand { name })
                }
                _ => Outcome::Stay,
            },
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}
