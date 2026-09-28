//! The workbench (`^T`, or `/changes` on the ledger): what changed, beside
//! the chat. Files on the left with the turn's commands and the turns; the
//! chat in the middle; the selected file's changes on the right, one hunk at
//! a time, each one undoable on its own.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ryter_core::diff::{FileDiff, LineKind};
use ryter_core::review::{self, FileChange};

use crate::action::Action;
use crate::chat::{MessageKind, SystemLevel, wrap};
use crate::theme::Theme;
use crate::view::View;

/// What the files are compared against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Since the checkpoint before the latest build turn.
    LastTurn,
    /// Since the last commit.
    Uncommitted,
}

/// The workbench's state.
#[derive(Debug, Clone)]
pub struct Workbench {
    root: PathBuf,
    scope: Scope,
    base: Result<String, String>,
    files: Vec<FileChange>,
    selected: usize,
    hunk: usize,
    diff: Option<FileDiff>,
    /// `X` pressed: the next `y` undoes the whole file.
    confirm_file: bool,
    note: Option<String>,
}

impl Workbench {
    /// Open on the last turn's changes (or uncommitted ones when no turn has
    /// changed anything yet).
    pub fn open(view: &View, root: PathBuf) -> Self {
        let scope = if view.last_checkpoint.is_some() {
            Scope::LastTurn
        } else {
            Scope::Uncommitted
        };
        let mut w = Self {
            root,
            scope,
            base: Err(String::new()),
            files: Vec::new(),
            selected: 0,
            hunk: 0,
            diff: None,
            confirm_file: false,
            note: None,
        };
        w.reload(view);
        w
    }

    /// Read the changes again, keeping the selected file where it can.
    pub fn reload(&mut self, view: &View) {
        let keep = self.files.get(self.selected).map(|f| f.path.clone());
        self.base = match self.scope {
            Scope::Uncommitted => Ok(review::head_base(&self.root)),
            Scope::LastTurn => view
                .last_checkpoint
                .clone()
                .ok_or_else(|| "no build turn has changed files yet".to_string()),
        };
        self.files = match &self.base {
            Ok(b) => review::changes(&self.root, b)
                .map(|c| c.files)
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        self.selected = keep
            .and_then(|p| self.files.iter().position(|f| f.path == p))
            .unwrap_or(self.selected)
            .min(self.files.len().saturating_sub(1));
        self.load_diff();
    }

    fn load_diff(&mut self) {
        self.diff = match (&self.base, self.files.get(self.selected)) {
            (Ok(base), Some(f)) => {
                review::file_versions(&self.root, base, &f.path)
                    .ok()
                    .map(|(old, new)| {
                        FileDiff::new(f.path.clone(), old.as_deref(), new.as_deref().unwrap_or(""))
                    })
            }
            _ => None,
        };
        let n = self.diff.as_ref().map_or(0, |d| d.hunks.len());
        self.hunk = self.hunk.min(n.saturating_sub(1));
    }

    /// Keys while the workbench is open. The composer doesn't take text here:
    /// `x` undoes a change, it isn't a letter in a message.
    pub fn key(&mut self, key: KeyEvent, view: &View) -> (bool, Action) {
        let busy = |w: &mut Self| {
            w.note = Some("wait for the turn to finish before undoing".into());
            (true, Action::None)
        };
        if self.confirm_file {
            self.confirm_file = false;
            if matches!(key.code, KeyCode::Char('y' | 'Y')) {
                if let (Ok(base), Some(f)) = (&self.base, self.files.get(self.selected)) {
                    return (
                        true,
                        Action::Revert {
                            base: base.clone(),
                            path: f.path.clone(),
                        },
                    );
                }
            }
            self.note = None;
            return (true, Action::None);
        }
        self.note = None;
        match key.code {
            KeyCode::Esc => return (false, Action::None),
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                self.hunk = 0;
                self.load_diff();
            }
            KeyCode::Down => {
                if self.selected + 1 < self.files.len() {
                    self.selected += 1;
                    self.hunk = 0;
                    self.load_diff();
                }
            }
            KeyCode::Char('j') | KeyCode::PageDown => {
                let n = self.diff.as_ref().map_or(0, |d| d.hunks.len());
                if self.hunk + 1 < n {
                    self.hunk += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::PageUp => self.hunk = self.hunk.saturating_sub(1),
            KeyCode::Tab | KeyCode::BackTab => {
                self.scope = match self.scope {
                    Scope::LastTurn => Scope::Uncommitted,
                    Scope::Uncommitted => Scope::LastTurn,
                };
                self.selected = 0;
                self.hunk = 0;
                self.reload(view);
            }
            KeyCode::Char('r') => self.reload(view),
            KeyCode::Char('x') => {
                if view.busy {
                    return busy(self);
                }
                if let (Ok(base), Some(f), Some(d)) =
                    (&self.base, self.files.get(self.selected), &self.diff)
                {
                    if d.hunks.get(self.hunk).is_some() {
                        return (
                            true,
                            Action::RevertHunk {
                                base: base.clone(),
                                path: f.path.clone(),
                                hunk: self.hunk,
                            },
                        );
                    }
                }
            }
            KeyCode::Char('X') => {
                if view.busy {
                    return busy(self);
                }
                if self.files.get(self.selected).is_some() {
                    self.confirm_file = true;
                }
            }
            KeyCode::Char('u') => {
                if view.busy {
                    return busy(self);
                }
                return (true, Action::Undo { force: false });
            }
            _ => {}
        }
        (true, Action::None)
    }

    /// The bar's keys while the workbench is open.
    pub fn legend(&self) -> String {
        if self.confirm_file {
            return "y undo the whole file · any other key keeps it".into();
        }
        let scope = match self.scope {
            Scope::LastTurn => "since last commit",
            Scope::Uncommitted => "last turn",
        };
        format!(
            "↑↓ file · j k change · x undo change · X undo file · u undo turn · tab {scope} · esc chat"
        )
    }

    /// Paint the three panes into `area`, with the chat drawn by `chat`.
    pub fn draw(
        &self,
        frame: &mut Frame,
        area: Rect,
        view: &View,
        theme: Theme,
        chat: impl FnOnce(&mut Frame, Rect),
    ) {
        let left_w = 30.min(area.width / 4);
        let right_w = (area.width * 45 / 100)
            .max(40)
            .min(area.width.saturating_sub(left_w + 30));
        let mid_w = area.width.saturating_sub(left_w + right_w + 2);
        let left = Rect {
            width: left_w,
            ..area
        };
        let mid = Rect {
            x: area.x + left_w + 1,
            width: mid_w,
            ..area
        };
        let right = Rect {
            x: area.x + left_w + mid_w + 2,
            width: right_w,
            ..area
        };
        self.draw_left(frame, left, view, theme);
        chat(frame, mid);
        self.draw_inspector(frame, right, theme);
    }

    fn draw_left(&self, frame: &mut Frame, area: Rect, view: &View, theme: Theme) {
        let dim = Style::default().fg(theme.dim).bg(theme.bg);
        let head = |s: &str| {
            Line::from(Span::styled(
                s.to_string(),
                dim.add_modifier(Modifier::BOLD),
            ))
        };
        let w = usize::from(area.width.saturating_sub(1));
        let mut rows = vec![head(match self.scope {
            Scope::LastTurn => "CHANGES · LAST TURN",
            Scope::Uncommitted => "CHANGES · UNCOMMITTED",
        })];
        match &self.base {
            Err(e) if !e.is_empty() => {
                rows.push(Line::from(Span::styled(wrap::truncate(e, w), dim)))
            }
            _ if self.files.is_empty() => {
                rows.push(Line::from(Span::styled("nothing changed", dim)))
            }
            _ => {}
        }
        for (i, f) in self.files.iter().enumerate() {
            let sel = i == self.selected;
            let counts = format!(" +{} −{}", f.added, f.removed);
            let name = wrap::truncate(&f.path, w.saturating_sub(wrap::width(&counts) + 2));
            let bg = if sel { theme.selection_bg } else { theme.bg };
            let pad = w.saturating_sub(2 + wrap::width(&name) + wrap::width(&counts));
            rows.push(Line::from(vec![
                Span::styled(
                    if sel { "▸ " } else { "  " },
                    Style::default().fg(theme.accent).bg(bg),
                ),
                Span::styled(name, Style::default().fg(theme.fg).bg(bg)),
                Span::styled(" ".repeat(pad), Style::default().bg(bg)),
                Span::styled(
                    format!(" +{}", f.added),
                    Style::default().fg(theme.success).bg(bg),
                ),
                Span::styled(
                    format!(" −{}", f.removed),
                    Style::default().fg(theme.error).bg(bg),
                ),
            ]));
        }
        // The last turn's commands, with what came of each.
        let latest = view.messages.iter().map(|m| m.turn).max().unwrap_or(0);
        let runs: Vec<&crate::chat::Message> = view
            .messages
            .iter()
            .filter(|m| {
                m.turn == latest
                    && matches!(&m.kind, MessageKind::Tool { name, .. } if name == "run")
            })
            .collect();
        rows.push(Line::from(Span::styled(String::new(), dim)));
        rows.push(head("COMMANDS"));
        if runs.is_empty() {
            rows.push(Line::from(Span::styled("none this turn", dim)));
        }
        for m in runs {
            let what = m.meta.label.clone().unwrap_or_default();
            let what = what.strip_prefix("run ").unwrap_or(&what).to_string();
            let came = m.meta.detail.clone().unwrap_or_default();
            let ok = came.starts_with('✓');
            rows.push(Line::from(vec![
                Span::styled(
                    if ok { "✓ " } else { "· " },
                    Style::default()
                        .fg(if ok { theme.success } else { theme.dim })
                        .bg(theme.bg),
                ),
                Span::styled(
                    wrap::truncate(&what, w.saturating_sub(2)),
                    Style::default().fg(theme.fg).bg(theme.bg),
                ),
            ]));
            if !came.is_empty() {
                rows.push(Line::from(Span::styled(
                    format!("  {}", wrap::truncate(&came, w.saturating_sub(2))),
                    dim,
                )));
            }
        }
        // The turns, newest first, with what each came to.
        rows.push(Line::from(Span::styled(String::new(), dim)));
        rows.push(head("TURNS"));
        let mut turns: Vec<(u64, String, String)> = Vec::new();
        for m in &view.messages {
            if matches!(m.kind, MessageKind::User) {
                turns.push((m.turn, m.collapsed(w), String::new()));
            }
            if matches!(
                m.kind,
                MessageKind::System {
                    level: SystemLevel::Receipt
                }
            ) {
                if let Some(t) = turns.iter_mut().find(|t| t.0 == m.turn) {
                    t.2 = m.body.chars().next().map(String::from).unwrap_or_default();
                }
            }
        }
        for (turn, asked, mark) in turns.iter().rev().take(8) {
            let mark = if mark.is_empty() {
                "◌".to_string()
            } else {
                mark.clone()
            };
            let style = if *turn == latest {
                Style::default().fg(theme.fg).bg(theme.bg)
            } else {
                dim
            };
            rows.push(Line::from(vec![
                Span::styled(
                    format!("{mark} "),
                    Style::default()
                        .fg(if mark == "✓" {
                            theme.success
                        } else {
                            theme.dim
                        })
                        .bg(theme.bg),
                ),
                Span::styled(wrap::truncate(asked, w.saturating_sub(2)), style),
            ]));
        }
        rows.truncate(usize::from(area.height));
        frame.render_widget(
            Paragraph::new(rows).style(Style::default().bg(theme.bg)),
            area,
        );
        // The pane's edge.
        let buf = frame.buffer_mut();
        let x = area.x + area.width;
        for y in area.y..area.y + area.height {
            if x < buf.area.width && y < buf.area.height {
                buf[(x, y)].set_symbol("│").set_style(dim);
            }
        }
    }

    fn draw_inspector(&self, frame: &mut Frame, area: Rect, theme: Theme) {
        let dim = Style::default().fg(theme.dim).bg(theme.bg);
        let title = match (self.files.get(self.selected), &self.diff) {
            (Some(f), Some(d)) => format!(
                " {}  +{} −{} · change {} of {} ",
                f.path,
                d.added,
                d.removed,
                (self.hunk + 1).min(d.hunks.len()),
                d.hunks.len()
            ),
            (Some(f), None) => format!(" {} ", f.path),
            _ => " no file ".into(),
        };
        let b = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(dim)
            .title(Span::styled(
                title,
                Style::default()
                    .fg(theme.fg)
                    .bg(theme.bg)
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(theme.bg));
        let inner = b.inner(area);
        frame.render_widget(b, area);
        let w = usize::from(inner.width);
        let mut rows: Vec<Line<'static>> = Vec::new();
        let mut focus_row = 0usize;
        if let Some(d) = &self.diff {
            for (i, h) in d.hunks.iter().enumerate() {
                let sel = i == self.hunk;
                if sel {
                    focus_row = rows.len();
                }
                let first = h.lines.first().and_then(|l| l.new.or(l.old)).unwrap_or(1);
                rows.push(Line::from(vec![
                    Span::styled(
                        if sel { "▌" } else { " " },
                        Style::default().fg(theme.accent).bg(theme.bg),
                    ),
                    Span::styled(
                        format!("change {} · line {first}", i + 1),
                        if sel {
                            Style::default().fg(theme.accent).bg(theme.bg)
                        } else {
                            dim
                        },
                    ),
                ]));
                for l in &h.lines {
                    let (sign, style) = match l.kind {
                        LineKind::Added => {
                            ("+", Style::default().fg(theme.fg).bg(theme.diff_add_bg))
                        }
                        LineKind::Removed => {
                            ("-", Style::default().fg(theme.fg).bg(theme.diff_del_bg))
                        }
                        LineKind::Context => (" ", Style::default().fg(theme.dim).bg(theme.bg)),
                    };
                    let old = l
                        .old
                        .map(|n| format!("{n:>4}"))
                        .unwrap_or_else(|| "    ".into());
                    let new = l
                        .new
                        .map(|n| format!("{n:>4}"))
                        .unwrap_or_else(|| "    ".into());
                    let text = format!("{old} {new} {sign} {}", wrap::expand_tabs(&l.text));
                    let text = wrap::truncate(&text, w.saturating_sub(1));
                    let pad = w.saturating_sub(1 + wrap::width(&text));
                    rows.push(Line::from(vec![
                        Span::styled(
                            if sel { "▌" } else { " " },
                            Style::default().fg(theme.accent).bg(theme.bg),
                        ),
                        Span::styled(text, style),
                        Span::styled(" ".repeat(pad), style),
                    ]));
                }
                rows.push(Line::from(Span::styled(String::new(), dim)));
            }
            if d.hunks.is_empty() {
                rows.push(Line::from(Span::styled(
                    " no line changes to show (binary, or too large)",
                    dim,
                )));
            }
        }
        if let Some(n) = &self.note {
            rows.insert(
                0,
                Line::from(Span::styled(
                    format!(" {n}"),
                    Style::default().fg(theme.warn).bg(theme.bg),
                )),
            );
        }
        if self.confirm_file {
            rows.insert(
                0,
                Line::from(Span::styled(
                    " undo every change to this file? y to undo, any other key keeps it",
                    Style::default()
                        .fg(theme.warn)
                        .bg(theme.bg)
                        .add_modifier(Modifier::BOLD),
                )),
            );
        }
        // Keep the selected change in view.
        let h = usize::from(inner.height);
        let skip = focus_row
            .saturating_sub(h / 4)
            .min(rows.len().saturating_sub(h));
        let rows: Vec<Line<'static>> = rows.into_iter().skip(skip).take(h).collect();
        frame.render_widget(
            Paragraph::new(rows).style(Style::default().bg(theme.bg)),
            inner,
        );
    }
}
