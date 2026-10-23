//! `/changes` — what changed in the files, file by file, with the diff taken
//! apart change by change; drop one change or undo a file; start a commit.

use std::cell::Cell;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ryter_core::AgentEvent;
use ryter_core::diff::FileDiff;
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

/// What `x` or `X` is about to put back, once the question is answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Revert {
    /// The whole file.
    File,
    /// One change of it (0-based), the file's other changes kept.
    Hunk(usize),
}

/// Changes browser.
#[derive(Debug, Clone)]
pub struct Changes {
    env: PanelEnv,
    cwd: PathBuf,
    scope: Scope,
    data: Result<review::Changes, String>,
    selected: usize,
    /// The selected file's diff, change by change, when it can be taken
    /// apart; the numbering is the one `revert_hunk` uses.
    diff: Option<FileDiff>,
    /// Git's own text for a file the diff cannot take apart (binary).
    text: Vec<String>,
    /// The selected change of the diff.
    hunk: usize,
    /// First diff row shown. Set while drawing when `follow` asks for the
    /// selected change, since only the drawing knows the rows.
    scroll: Cell<usize>,
    follow: Cell<bool>,
    /// Rows the diff had at the last drawing, for paging.
    rows: Cell<usize>,
    confirm: Option<Revert>,
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
            diff: None,
            text: Vec::new(),
            hunk: 0,
            scroll: Cell::new(0),
            follow: Cell::new(false),
            rows: Cell::new(0),
            confirm: None,
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

    /// The selected file's changes, as the diff counts them.
    fn hunks(&self) -> usize {
        self.diff.as_ref().map_or(0, |d| d.hunks.len())
    }

    /// Whether one change of the selected file can be put back on its own:
    /// the diff took it apart, and the file is still there to write.
    fn can_drop_hunk(&self) -> bool {
        self.hunks() > 0 && self.current().is_some_and(|f| f.status != Status::Deleted)
    }

    fn load_diff(&mut self) {
        self.scroll.set(0);
        self.follow.set(false);
        self.hunk = 0;
        self.diff = None;
        self.text = Vec::new();
        let (Ok(c), Some(f)) = (&self.data, self.current()) else {
            return;
        };
        if !f.binary {
            // The file as the base had it and as it is now; a deleted file
            // is every line removed.
            if let Ok((old, new)) = review::file_versions(&self.cwd, &c.base, &f.path) {
                let whole =
                    FileDiff::whole(f.path.clone(), old.as_deref(), new.as_deref().unwrap_or(""));
                if let Some(d) = whole.filter(|d| !d.hunks.is_empty()) {
                    self.diff = Some(d);
                    return;
                }
            }
        }
        self.text = review::file_diff(&self.cwd, c, &f.path)
            .lines()
            // The file header repeats what the list shows.
            .skip_while(|l| !l.starts_with("@@") && !l.starts_with("Binary"))
            .map(wrap::expand_tabs)
            .collect();
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

    fn select_hunk(&mut self, delta: isize) {
        let n = self.hunks();
        if n == 0 {
            return;
        }
        let next = (self.hunk as isize + delta).clamp(0, n as isize - 1) as usize;
        if next != self.hunk {
            self.hunk = next;
            self.follow.set(true);
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

    fn text_row(line: &str, w: usize, theme: Theme) -> Line<'static> {
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

    /// The change's heading: which it is, and the lines of the file it
    /// covers (the new file's where it has them, the old file's for lines
    /// that only went).
    fn hunk_heading(&self, i: usize, w: usize, theme: Theme) -> Line<'static> {
        let Some(d) = &self.diff else {
            return widgets::blank(theme);
        };
        let lines = &d.hunks[i].lines;
        let news: Vec<usize> = lines.iter().filter_map(|l| l.new).collect();
        let span = match (news.first(), news.last()) {
            (Some(a), Some(b)) if a == b => format!("line {a}"),
            (Some(a), Some(b)) => format!("lines {a}–{b}"),
            _ => {
                let olds: Vec<usize> = lines.iter().filter_map(|l| l.old).collect();
                match (olds.first(), olds.last()) {
                    (Some(a), Some(b)) if a == b => format!("line {a} removed"),
                    (Some(a), Some(b)) => format!("lines {a}–{b} removed"),
                    _ => String::new(),
                }
            }
        };
        let selected = i == self.hunk;
        let label = format!(
            "{} change {} of {} · {span} ",
            if selected { "▸" } else { "─" },
            i + 1,
            d.hunks.len()
        );
        let rule = format!(
            " {label}{}",
            "─".repeat(w.saturating_sub(wrap::width(&label) + 2))
        );
        let style = if selected {
            theme.on_panel(theme.accent).add_modifier(Modifier::BOLD)
        } else {
            theme.panel_muted()
        };
        Line::from(Span::styled(rule, style))
    }

    /// The diff pane's rows, and the row each change starts on.
    fn diff_rows(&self, w: usize, theme: Theme) -> (Vec<Line<'static>>, Vec<usize>) {
        let mut rows = Vec::new();
        let mut starts = Vec::new();
        match &self.diff {
            Some(d) => {
                let hunks = crate::chat::diff::hunk_rows(d, w, theme, theme.panel_bg);
                for (i, body) in hunks.into_iter().enumerate() {
                    starts.push(rows.len());
                    rows.push(self.hunk_heading(i, w, theme));
                    rows.extend(body);
                }
            }
            None => rows.extend(self.text.iter().map(|l| Self::text_row(l, w, theme))),
        }
        (rows, starts)
    }

    /// The one question's words.
    fn question(&self, r: Revert) -> Option<String> {
        let f = self.current()?;
        let when = match self.scope {
            Scope::Uncommitted => "at the last commit",
            Scope::LastTurn => "before the last build turn",
        };
        Some(match r {
            Revert::Hunk(i) => format!(
                "drop change {} of {} in {}, back to {}?",
                i + 1,
                self.hunks(),
                f.path,
                when.trim_start_matches("at ")
            ),
            Revert::File if f.status == Status::Added => {
                format!("delete {} (it didn't exist {when})?", f.path)
            }
            Revert::File => format!("put {} back as it was {when}?", f.path),
        })
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
            Some(_) if self.hunks() > 0 => format!(
                "{} of {} · change {} of {}",
                self.selected + 1,
                self.files().len(),
                self.hunk + 1,
                self.hunks()
            ),
            Some(_) => format!("{} of {}", self.selected + 1, self.files().len()),
            None => String::new(),
        }
    }

    fn legend(&self, _view: &View) -> String {
        match self.confirm {
            Some(Revert::File) => "⏎ put it back · n keep · esc".into(),
            Some(Revert::Hunk(_)) => "⏎ drop it · n keep · esc".into(),
            None => {
                "↑↓ file · ←→ change · x drop change · X undo file · tab scope · c commit · esc"
                    .into()
            }
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
                lines.len() + 1 + usize::from(self.note.is_some() || self.confirm.is_some()),
            );
            let (rows, starts) = self.diff_rows(w, theme);
            let total = rows.len();
            self.rows.set(total);
            if self.follow.replace(false) {
                if let Some(&at) = starts.get(self.hunk) {
                    self.scroll.set(at);
                }
            }
            let scroll = self.scroll.get().min(total.saturating_sub(diff_h));
            self.scroll.set(scroll);
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
            lines.extend(rows.into_iter().skip(scroll).take(diff_h));
        }
        let footer = match self.confirm {
            Some(r) => self
                .question(r)
                .map(|q| widgets::colored(&format!("{q}  ⏎ yes · n no"), theme.warn, theme)),
            None => self
                .note
                .as_ref()
                .map(|n| widgets::colored(n, theme.warn, theme)),
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
        if let Some(r) = self.confirm {
            // The one question's keys: Enter yes, n (or Esc) no.
            match key.code {
                KeyCode::Enter | KeyCode::Char('y' | 'Y') => {
                    self.confirm = None;
                    if let (Ok(base), Some(f)) = (self.base(view), self.current()) {
                        let path = f.path.clone();
                        return Outcome::Act(match r {
                            Revert::File => Action::Revert { base, path },
                            Revert::Hunk(hunk) => Action::RevertHunk { base, path, hunk },
                        });
                    }
                }
                KeyCode::Char('n' | 'N') | KeyCode::Esc => self.confirm = None,
                _ => {}
            }
            return Outcome::Stay;
        }
        match key.code {
            KeyCode::Esc => return Outcome::Close,
            KeyCode::Up | KeyCode::Char('k') => self.select(-1),
            KeyCode::Down | KeyCode::Char('j') => self.select(1),
            KeyCode::Left | KeyCode::Char('[') => self.select_hunk(-1),
            KeyCode::Right | KeyCode::Char(']') => self.select_hunk(1),
            KeyCode::PageDown | KeyCode::Char(' ') => {
                self.scroll
                    .set((self.scroll.get() + PAGE).min(self.rows.get().saturating_sub(1)));
            }
            KeyCode::PageUp => self.scroll.set(self.scroll.get().saturating_sub(PAGE)),
            KeyCode::Home => self.scroll.set(0),
            KeyCode::End => self.scroll.set(self.rows.get().saturating_sub(PAGE)),
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
                    self.note =
                        Some("wait for the turn to finish before putting a change back".into());
                } else if self.can_drop_hunk() {
                    self.confirm = Some(Revert::Hunk(self.hunk));
                } else {
                    // Nothing to take apart (binary, or a file that is gone):
                    // the file is the change.
                    self.confirm = Some(Revert::File);
                }
            }
            KeyCode::Char('X') if self.current().is_some() => {
                if view.busy {
                    self.note = Some("wait for the turn to finish before undoing a file".into());
                } else {
                    self.confirm = Some(Revert::File);
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
