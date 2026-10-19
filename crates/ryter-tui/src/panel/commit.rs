//! `/commit` — choose the files, read (or edit) a drafted message, see the
//! receipt, commit.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::Line;
use ryter_core::AgentEvent;
use ryter_core::review::{self, Receipt, Status};

use super::{Body, Outcome, Panel, PanelEnv, widgets};
use crate::action::Action;
use crate::chat::wrap;
use crate::theme::Theme;
use crate::view::View;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Stage {
    /// Waiting for the model's draft.
    Drafting,
    /// Message shown; ready to commit.
    Ready,
    /// The composer holds the message.
    Editing,
    /// Commit sent to git.
    Committing,
}

/// Commit panel.
#[derive(Debug, Clone)]
pub struct Commit {
    files: Vec<(review::FileChange, bool)>,
    selected: usize,
    message: String,
    stage: Stage,
    receipt: Receipt,
    error: Option<String>,
}

impl Commit {
    /// Open on everything uncommitted, all chosen, and ask for a draft.
    pub fn new(view: &View, env: &PanelEnv) -> (Self, Action) {
        let base = review::head_base(&env.cwd);
        let mut now = None;
        let (files, error) = match review::changes(&env.cwd, &base) {
            Ok(c) => {
                now = review::tree_of(&env.cwd, &c.now);
                (c.files.into_iter().map(|f| (f, true)).collect(), None)
            }
            Err(e) => (Vec::new(), Some(super::changes::plain(&e))),
        };
        let since = review::head_time_ms(&env.cwd).unwrap_or(0);
        let spend = ryter_core::project::spend_since(&env.home, &env.cwd, since);
        let receipt = Receipt {
            models: spend.models.into_iter().map(|(m, _)| m).collect(),
            usd: spend.usd,
            partial: spend.unpriced > 0,
            tests: view.last_tests.clone(),
            tests_stale: view.tests_stale,
            review: review::Reviewed::of(view.last_review.as_ref(), now.as_deref()),
        };
        let mut c = Self {
            files,
            selected: 0,
            message: String::new(),
            stage: Stage::Ready,
            receipt,
            error,
        };
        let act = if c.files.is_empty() {
            if c.error.is_none() {
                c.error = Some("nothing to commit: the files match the last commit".into());
            }
            Action::None
        } else {
            c.stage = Stage::Drafting;
            Action::DraftCommit(c.chosen())
        };
        (c, act)
    }

    /// The receipt's line, as the commit message would carry it.
    #[cfg(test)]
    pub(crate) fn receipt_line(&self) -> String {
        self.receipt.line()
    }

    fn chosen(&self) -> Vec<String> {
        self.files
            .iter()
            .filter(|(_, on)| *on)
            .map(|(f, _)| f.path.clone())
            .collect()
    }

    /// The message as it will be committed.
    fn full_message(&self, view: &View) -> String {
        if view.ui.receipts {
            review::with_receipt(&self.message, &self.receipt)
        } else {
            format!("{}\n", self.message.trim_end())
        }
    }
}

impl Panel for Commit {
    fn kind(&self) -> &'static str {
        "commit"
    }

    fn title(&self, _view: &View) -> String {
        "commit".into()
    }

    fn status(&self, _view: &View) -> String {
        let n = self.files.iter().filter(|(_, on)| *on).count();
        format!("{n} of {} files", self.files.len())
    }

    fn legend(&self, _view: &View) -> String {
        if self.files.is_empty() {
            return "t receipt on/off · esc".into();
        }
        match self.stage {
            Stage::Editing => "type · shift/alt+enter new line · enter done · esc cancel".into(),
            Stage::Drafting => "drafting the message… · space choose files · esc".into(),
            Stage::Committing => "committing…".into(),
            Stage::Ready => {
                "enter commit · e edit · d redraft · space choose file · t receipt · esc".into()
            }
        }
    }

    fn input(&self, _view: &View) -> Option<String> {
        (self.stage == Stage::Editing).then(|| "commit message".into())
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        if self.files.is_empty() {
            (90, 3)
        } else {
            (100, 40)
        }
    }

    fn render(&self, view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let h = usize::from(height).max(4);
        let mut lines: Vec<Line<'static>> = Vec::new();
        if self.files.is_empty() {
            let why = self.error.as_deref().unwrap_or("nothing to commit");
            for row in wrap::wrap_plain(why, w.saturating_sub(2)) {
                lines.push(widgets::text(&row, theme));
            }
            return Body {
                lines,
                scroll: None,
            };
        }
        let list_h = self.files.len().min((h / 3).max(3));
        let first = super::window(self.selected, self.files.len(), list_h);
        let hidden = self.files.len() - list_h;
        for (i, (f, on)) in self.files.iter().enumerate().skip(first).take(list_h) {
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
            lines.push(widgets::list_row(
                if *on { "[x]" } else { "[ ]" },
                &format!("{} {}", f.status.letter(), f.path),
                "",
                &stat,
                i == self.selected,
                w,
                theme,
                Some(color),
            ));
        }
        // A list cut short says so: seven files must never read as five.
        if hidden > 0 {
            let above = first;
            let below = hidden - above;
            let where_ = match (above, below) {
                (0, n) => format!("{n} more below · ↑↓"),
                (n, 0) => format!("{n} more above · ↑↓"),
                (a, b) => format!("{a} more above · {b} below · ↑↓"),
            };
            lines.push(widgets::note(&where_, theme));
        }
        lines.push(widgets::blank(theme));
        match self.stage {
            Stage::Drafting => lines.push(widgets::note(
                "drafting a message from the diff and this conversation…",
                theme,
            )),
            Stage::Editing => lines.push(widgets::note("editing the message below", theme)),
            _ => {
                let full = self.full_message(view);
                for (i, l) in full.lines().enumerate() {
                    for row in wrap::wrap_plain(l, w.saturating_sub(2)) {
                        lines.push(if i == 0 {
                            widgets::colored(&row, theme.accent, theme)
                        } else if l.starts_with("Ryter: ") {
                            widgets::colored(&row, theme.dim, theme)
                        } else {
                            widgets::text(&row, theme)
                        });
                    }
                }
            }
        }
        if let Some(e) = &self.error {
            lines.push(widgets::blank(theme));
            for row in wrap::wrap_plain(e, w.saturating_sub(2)) {
                lines.push(widgets::colored(&row, theme.error, theme));
            }
        }
        let receipt_note = if view.ui.receipts {
            "receipt on · t turns it off"
        } else {
            "receipt off · t adds model, cost, tests and review to the message"
        };
        if lines.len() + 2 < h {
            while lines.len() + 1 < h {
                lines.push(widgets::blank(theme));
            }
            lines.push(widgets::note(receipt_note, theme));
        }
        lines.truncate(h);
        Body {
            lines,
            scroll: None,
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        if self.stage == Stage::Editing {
            let shift = key
                .modifiers
                .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT);
            return match key.code {
                KeyCode::Esc => {
                    self.stage = Stage::Ready;
                    view.composer.clear();
                    Outcome::Stay
                }
                KeyCode::Enter if shift => {
                    view.composer.newline();
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    let text = view.composer.text().trim().to_string();
                    if !text.is_empty() {
                        self.message = text;
                    }
                    self.stage = Stage::Ready;
                    view.composer.clear();
                    Outcome::Stay
                }
                KeyCode::Up => {
                    view.composer.up();
                    Outcome::Stay
                }
                KeyCode::Down => {
                    view.composer.down();
                    Outcome::Stay
                }
                _ => {
                    super::edit_field(&mut view.composer, key);
                    Outcome::Stay
                }
            };
        }
        match key.code {
            KeyCode::Esc => Outcome::Close,
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                Outcome::Stay
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(self.files.len().saturating_sub(1));
                Outcome::Stay
            }
            KeyCode::Char(' ') => {
                if let Some((_, on)) = self.files.get_mut(self.selected) {
                    *on = !*on;
                }
                Outcome::Stay
            }
            KeyCode::Char('t') => {
                let on = !view.ui.receipts;
                Outcome::Act(Action::SetReceipts(on))
            }
            _ if self.stage != Stage::Ready => Outcome::Stay,
            KeyCode::Char('e') => {
                // Enter field mode here, with the text: the sync after this
                // key would start an empty field.
                view.composer.begin_field("commit message", &self.message);
                self.stage = Stage::Editing;
                Outcome::Stay
            }
            KeyCode::Char('d') if !self.chosen().is_empty() => {
                self.stage = Stage::Drafting;
                self.error = None;
                Outcome::Act(Action::DraftCommit(self.chosen()))
            }
            KeyCode::Enter => {
                let paths = self.chosen();
                if view.busy {
                    self.error = Some("wait for the turn to finish before committing".into());
                } else if paths.is_empty() {
                    self.error = Some("choose at least one file (space)".into());
                } else if self.message.trim().is_empty() {
                    self.error = Some("the message is empty: e to write one".into());
                } else {
                    self.error = None;
                    self.stage = Stage::Committing;
                    return Outcome::Act(Action::Commit {
                        paths,
                        message: self.full_message(view),
                    });
                }
                Outcome::Stay
            }
            _ => Outcome::Stay,
        }
    }

    fn on_event(&mut self, ev: &AgentEvent, _view: &mut View) {
        match ev {
            AgentEvent::CommitDraft { message, error } if self.stage == Stage::Drafting => {
                self.stage = Stage::Ready;
                if let Some(m) = message {
                    self.message = m.clone();
                }
                self.error = error
                    .as_ref()
                    .map(|e| format!("couldn't draft a message: {e} · e to write one"));
            }
            AgentEvent::Committed { error: Some(e), .. } => {
                self.stage = Stage::Ready;
                self.error = Some(format!("git refused the commit: {}", e.trim()));
            }
            _ => {}
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ryter_core::review::{FileChange, Status};

    fn text(body: Body) -> String {
        body.lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
                    + "\n"
            })
            .collect()
    }

    /// On a short screen the file list says how many files are out of
    /// sight, so a commit of seven files never reads as five.
    #[test]
    fn a_short_list_says_how_many_more() {
        let files = (1..=7)
            .map(|i| {
                (
                    FileChange {
                        path: format!("src/f{i}.rs"),
                        status: Status::Added,
                        added: i,
                        removed: 0,
                        binary: false,
                    },
                    true,
                )
            })
            .collect();
        let c = Commit {
            files,
            selected: 0,
            message: "Add seven".into(),
            stage: Stage::Ready,
            receipt: Receipt::default(),
            error: None,
        };
        let v = View::new("c".into(), "m".into(), "/tmp".into());
        let short = text(c.render(&v, 90, 12, Theme::truecolor_dark()));
        assert!(short.contains("src/f4.rs"), "{short}");
        assert!(!short.contains("src/f5.rs"), "{short}");
        assert!(short.contains("3 more"), "{short}");
        let tall = text(c.render(&v, 90, 40, Theme::truecolor_dark()));
        assert!(
            tall.contains("src/f7.rs") && !tall.contains("more"),
            "{tall}"
        );
    }
}
