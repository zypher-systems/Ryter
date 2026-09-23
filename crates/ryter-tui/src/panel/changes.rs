//! `/changes` — what changed in the files, file by file, with the diff; undo
//! one file; start a commit.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ryter_core::AgentEvent;
use ryter_core::review::{self, Status};

use super::{Body, Outcome, Panel, PanelEnv, widgets};
use crate::action::Action;
use crate::chat::wrap;
use crate::theme::Theme;
use crate::view::View;

/// Rows one PageUp / PageDown moves the diff.
const PAGE: usize = 12;

/// What the files are compared against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Since the last commit: what `/commit` would commit.
    Uncommitted,
    /// Since the checkpoint before the latest build turn.
    LastTurn,
}

/// Changes browser.
#[derive(Debug, Clone)]
pub struct Changes {
    env: PanelEnv,
    cwd: PathBuf,
    scope: Scope,
    data: Result<review::Changes, String>,
    selected: usize,
    diff: Vec<String>,
    scroll: usize,
    confirm_revert: bool,
    note: Option<String>,
}

impl Changes {
    /// Open on uncommitted changes.
    pub fn new(view: &View, env: &PanelEnv) -> Self {
        let mut c = Self {
            env: env.clone(),
            cwd: env.cwd.clone(),
            scope: Scope::Uncommitted,
            data: Err(String::new()),
            selected: 0,
            diff: Vec::new(),
            scroll: 0,
            confirm_revert: false,
            note: None,
        };
        c.reload(view);
        c
    }

    fn base(&self, view: &View) -> Result<String, String> {
        match self.scope {
            Scope::Uncommitted => Ok(review::head_base(&self.cwd)),
            Scope::LastTurn => view
                .last_checkpoint
                .clone()
                .ok_or_else(|| "no build turn has changed files in this session yet".to_string()),
        }
    }

    fn reload(&mut self, view: &View) {
        let keep = self.current().map(|f| f.path.clone());
        self.data = self
            .base(view)
            .and_then(|b| review::changes(&self.cwd, &b).map_err(|e| plain(&e)));
        let n = self.files().len();
        self.selected = keep
            .and_then(|p| self.files().iter().position(|f| f.path == p))
            .unwrap_or(self.selected)
            .min(n.saturating_sub(1));
        self.load_diff();
    }

    fn files(&self) -> &[review::FileChange] {
        self.data
            .as_ref()
            .map(|c| c.files.as_slice())
            .unwrap_or(&[])
    }

    fn current(&self) -> Option<&review::FileChange> {
        self.files().get(self.selected)
    }

    fn load_diff(&mut self) {
        self.scroll = 0;
        self.diff = match (&self.data, self.current()) {
            (Ok(c), Some(f)) => review::file_diff(&self.cwd, c, &f.path)
                .lines()
                // The file header repeats what the list shows.
                .skip_while(|l| !l.starts_with("@@") && !l.starts_with("Binary"))
                .map(wrap::expand_tabs)
                .collect(),
            _ => Vec::new(),
        };
    }

    fn select(&mut self, delta: isize) {
        let n = self.files().len();
        if n == 0 {
            return;
        }
        let next = (self.selected as isize + delta).clamp(0, n as isize - 1) as usize;
        if next != self.selected {
            self.selected = next;
            self.load_diff();
        }
    }

    fn scope_line(&self, w: usize, theme: Theme) -> Line<'static> {
        let tab = |label: &str, on: bool| {
            if on {
                Span::styled(
                    format!(" {label} "),
                    Style::default()
                        .fg(theme.panel_bg)
                        .bg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(format!(" {label} "), theme.panel_muted())
            }
        };
        let totals = match &self.data {
            Ok(c) => {
                let (a, r) = c.totals();
                let n = c.files.len();
                format!("{n} file{} · +{a} −{r} ", if n == 1 { "" } else { "s" })
            }
            Err(_) => String::new(),
        };
        let left = 1 + wrap::width(" uncommitted ") + 1 + wrap::width(" last turn ");
        let pad = w.saturating_sub(left + wrap::width(&totals));
        Line::from(vec![
            Span::styled(" ", theme.panel()),
            tab("uncommitted", self.scope == Scope::Uncommitted),
            Span::styled(" ", theme.panel()),
            tab("last turn", self.scope == Scope::LastTurn),
            Span::styled(" ".repeat(pad), theme.panel()),
            Span::styled(totals, theme.panel_muted()),
        ])
    }

    fn file_row(
        &self,
        f: &review::FileChange,
        selected: bool,
        w: usize,
        theme: Theme,
    ) -> Line<'static> {
        let color = match f.status {
            Status::Added => theme.success,
            Status::Modified => theme.warn,
            Status::Deleted => theme.error,
        };
        let stat = if f.binary {
            "binary".to_string()
        } else {
            format!("+{} −{}", f.added, f.removed)
        };
        widgets::list_row(
            &f.status.letter().to_string(),
            &f.path,
            "",
            &stat,
            selected,
            w,
            theme,
            Some(color),
        )
    }

    fn diff_row(line: &str, w: usize, theme: Theme) -> Line<'static> {
        let color = if line.starts_with("@@") {
            theme.accent
        } else if line.starts_with('+') {
            theme.success
        } else if line.starts_with('-') {
            theme.error
        } else if line.starts_with('\\') {
            theme.dim
        } else {
            theme.fg
        };
        Line::from(Span::styled(
            format!(" {}", wrap::truncate(line, w.saturating_sub(2))),
            theme.on_panel(color),
        ))
    }
}

/// An error as a sentence, without the `io:` category.
pub(super) fn plain(e: &ryter_core::Error) -> String {
    let s = e.to_string();
    s.strip_prefix("io: ").unwrap_or(&s).to_string()
}

impl Panel for Changes {
    fn kind(&self) -> &'static str {
        "changes"
    }

    fn title(&self, _view: &View) -> String {
        "changes".into()
    }

    fn status(&self, _view: &View) -> String {
        match self.current() {
            Some(_) => format!("{} of {}", self.selected + 1, self.files().len()),
            None => String::new(),
        }
    }

    fn legend(&self, _view: &View) -> String {
        if self.confirm_revert {
            "y put it back · n keep".into()
        } else {
            "↑↓ file · pgup/pgdn diff · tab scope · x undo file · c commit · r refresh · esc".into()
        }
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        if self.files().is_empty() {
            (90, 5)
        } else {
            (120, 60)
        }
    }

    fn render(&self, _view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let h = usize::from(height).max(4);
        let mut lines = vec![self.scope_line(w, theme)];
        let files = self.files();
        match &self.data {
            Err(e) if !e.is_empty() => {
                lines.push(widgets::blank(theme));
                lines.push(widgets::note(e, theme));
            }
            Ok(_) if files.is_empty() => {
                lines.push(widgets::blank(theme));
                lines.push(widgets::note(
                    match self.scope {
                        Scope::Uncommitted => {
                            "nothing uncommitted: the files match the last commit"
                        }
                        Scope::LastTurn => "nothing changed since the last build turn began",
                    },
                    theme,
                ));
            }
            _ => {}
        }
        if !files.is_empty() {
            // The list gets up to a third of the height; the diff the rest.
            let list_h = files.len().min((h / 3).max(3));
            let first = super::window(self.selected, files.len(), list_h);
            for (i, f) in files.iter().enumerate().skip(first).take(list_h) {
                lines.push(self.file_row(f, i == self.selected, w, theme));
            }
            let diff_h = h.saturating_sub(
                lines.len() + 1 + usize::from(self.note.is_some() || self.confirm_revert),
            );
            let total = self.diff.len();
            let scroll = self.scroll.min(total.saturating_sub(diff_h));
            let path = self.current().map(|f| f.path.clone()).unwrap_or_default();
            let range = if total > diff_h {
                format!(
                    " · {}–{} of {total}",
                    scroll + 1,
                    (scroll + diff_h).min(total)
                )
            } else {
                String::new()
            };
            let label = format!("── {}{range} ", wrap::truncate(&path, w.saturating_sub(30)));
            let rule = format!(
                "{label}{}",
                "─".repeat(w.saturating_sub(wrap::width(&label) + 1))
            );
            lines.push(widgets::note(&rule, theme));
            for l in self.diff.iter().skip(scroll).take(diff_h) {
                lines.push(Self::diff_row(l, w, theme));
            }
        }
        let footer = if self.confirm_revert {
            self.current().map(|f| {
                let when = match self.scope {
                    Scope::Uncommitted => "at the last commit",
                    Scope::LastTurn => "before the last build turn",
                };
                let what = if f.status == Status::Added {
                    format!("delete {} (it didn't exist {when})?", f.path)
                } else {
                    format!("put {} back as it was {when}?", f.path)
                };
                widgets::colored(&format!("{what}  y yes · n no"), theme.warn, theme)
            })
        } else {
            self.note
                .as_ref()
                .map(|n| widgets::colored(n, theme.warn, theme))
        };
        if let Some(f) = footer {
            while lines.len() < h - 1 {
                lines.push(widgets::blank(theme));
            }
            lines.truncate(h - 1);
            lines.push(f);
        }
        lines.truncate(h);
        Body {
            lines,
            scroll: None,
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        self.note = None;
        if self.confirm_revert {
            self.confirm_revert = false;
            if matches!(key.code, KeyCode::Char('y' | 'Y')) {
                if let (Ok(base), Some(f)) = (self.base(view), self.current()) {
                    return Outcome::Act(Action::Revert {
                        base,
                        path: f.path.clone(),
                    });
                }
            }
            return Outcome::Stay;
        }
        match key.code {
            KeyCode::Esc => return Outcome::Close,
            KeyCode::Up | KeyCode::Char('k') => self.select(-1),
            KeyCode::Down | KeyCode::Char('j') => self.select(1),
            KeyCode::PageDown | KeyCode::Char(' ') => {
                self.scroll = (self.scroll + PAGE).min(self.diff.len().saturating_sub(1));
            }
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(PAGE),
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll = self.diff.len().saturating_sub(PAGE),
            KeyCode::Tab | KeyCode::BackTab => {
                self.scope = match self.scope {
                    Scope::Uncommitted => Scope::LastTurn,
                    Scope::LastTurn => Scope::Uncommitted,
                };
                self.selected = 0;
                self.reload(view);
            }
            KeyCode::Char('r') => self.reload(view),
            KeyCode::Char('x') if self.current().is_some() => {
                if view.busy {
                    self.note = Some("wait for the turn to finish before undoing a file".into());
                } else {
                    self.confirm_revert = true;
                }
            }
            KeyCode::Char('c') => {
                if view.busy {
                    self.note = Some("wait for the turn to finish before committing".into());
                } else {
                    let (panel, act) = super::commit::Commit::new(view, &self.env);
                    return Outcome::PushAct(Box::new(panel), act);
                }
            }
            _ => {}
        }
        Outcome::Stay
    }

    fn on_event(&mut self, ev: &AgentEvent, view: &mut View) {
        match ev {
            AgentEvent::Reverted { .. } | AgentEvent::TurnFinished { .. } => self.reload(view),
            AgentEvent::Checkpoint { .. } if self.scope == Scope::LastTurn => self.reload(view),
            _ => {}
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}
