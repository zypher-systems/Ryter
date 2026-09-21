//! `/sessions` — unified session browser (`R-POP-38..42`).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::text::Line;
use ryter_core::{Session, format_usd};

use super::{Body, Notice, Outcome, Panel, PanelEnv, widgets};
use crate::action::{Action, SessionsMode};
use crate::chat::wrap;
use crate::theme::Theme;
use crate::view::View;

/// One saved session.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Full id.
    pub id: String,
    /// Short id.
    pub short: String,
    /// Title / preview.
    pub title: String,
    /// Last write, unix ms.
    pub updated_ms: Option<u64>,
    /// Transcript rows.
    pub messages: usize,
    /// Known spend.
    pub spend: Option<f64>,
    /// Phase.
    pub phase: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    Browse,
    Rename,
    ConfirmDelete,
}

/// Session browser.
#[derive(Debug, Clone)]
pub struct Sessions {
    rows: Vec<Row>,
    selected: usize,
    mode: Mode,
    home: std::path::PathBuf,
    cwd: std::path::PathBuf,
}

/// `2h ago` style relative time.
pub fn relative(now_ms: u64, then_ms: Option<u64>) -> String {
    let Some(t) = then_ms else {
        return String::new();
    };
    let d = now_ms.saturating_sub(t) / 1000;
    if d < 60 {
        "just now".into()
    } else if d < 3600 {
        format!("{}m ago", d / 60)
    } else if d < 86_400 {
        format!("{}h ago", d / 3600)
    } else {
        format!("{}d ago", d / 86_400)
    }
}

fn wall_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Sessions {
    /// Load the list for `env.cwd`.
    pub fn new(view: &mut View, env: &PanelEnv, mode: SessionsMode) -> Self {
        let mut s = Self {
            rows: Vec::new(),
            selected: 0,
            mode: match mode {
                SessionsMode::Browse => Mode::Browse,
                SessionsMode::Rename => Mode::Rename,
                SessionsMode::Delete => Mode::ConfirmDelete,
            },
            home: env.home.clone(),
            cwd: env.cwd.clone(),
        };
        s.reload(view);
        if s.mode == Mode::Rename {
            view.composer.set_text(&view.session_title);
        } else {
            view.composer.clear();
        }
        if s.mode == Mode::ConfirmDelete && s.current_is_selected(view) {
            s.mode = Mode::Browse;
        }
        s
    }

    fn reload(&mut self, view: &View) {
        let mut rows: Vec<Row> = Session::list(&self.home, &self.cwd)
            .unwrap_or_default()
            .into_iter()
            .map(|s| Row {
                id: s.meta.id.to_string(),
                short: s.short_id(),
                title: if s.meta.title.trim().is_empty() {
                    s.preview.clone()
                } else {
                    s.meta.title.clone()
                },
                updated_ms: s.updated_millis(),
                messages: s.messages,
                spend: s.meta.spend_usd_total,
                phase: s.meta.phase.to_string(),
            })
            .collect();
        rows.sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms));
        self.rows = rows;
        self.selected = self
            .rows
            .iter()
            .position(|r| r.id == view.session_id)
            .unwrap_or(0);
    }

    fn filtered(&self, view: &View) -> Vec<&Row> {
        if self.mode != Mode::Browse {
            return self.rows.iter().collect();
        }
        let f = view.composer.text().trim().to_ascii_lowercase();
        self.rows
            .iter()
            .filter(|r| {
                f.is_empty() || r.title.to_ascii_lowercase().contains(&f) || r.short.contains(&f)
            })
            .collect()
    }

    fn current_is_selected(&self, view: &View) -> bool {
        self.filtered(view)
            .get(self.selected)
            .is_some_and(|r| r.id == view.session_id)
    }
}

impl Panel for Sessions {
    fn kind(&self) -> &'static str {
        "sessions"
    }

    fn title(&self, _view: &View) -> String {
        match self.mode {
            Mode::Browse => "sessions".into(),
            Mode::Rename => "rename session".into(),
            Mode::ConfirmDelete => "delete session".into(),
        }
    }

    fn status(&self, view: &View) -> String {
        format!("{}", self.filtered(view).len())
    }

    fn legend(&self, view: &View) -> String {
        match self.mode {
            Mode::Browse => {
                "enter resume · r rename · d delete · n new · type to filter · esc".into()
            }
            Mode::Rename => "type the new title · enter save · esc cancel".into(),
            Mode::ConfirmDelete => {
                let short = self
                    .filtered(view)
                    .get(self.selected)
                    .map(|r| r.short.clone())
                    .unwrap_or_default();
                format!("type `{short}` to delete · enter · esc cancel")
            }
        }
    }

    fn input(&self, _view: &View) -> Option<String> {
        Some(match self.mode {
            Mode::Browse => "filter".into(),
            Mode::Rename => "title".into(),
            Mode::ConfirmDelete => "confirm".into(),
        })
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        (86, (self.rows.len() + 3).clamp(5, 20) as u16)
    }

    fn render(&self, view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let h = usize::from(height).max(1);
        let list = self.filtered(view);
        let n = list.len();
        let sel = self.selected.min(n.saturating_sub(1));
        let rows_h = h.saturating_sub(1).max(1);
        let first = super::window(sel, n, rows_h);
        let now = wall_ms();
        let mut lines: Vec<Line<'static>> = Vec::new();
        if n == 0 {
            lines.push(widgets::note("no saved sessions in this directory", theme));
        }
        let rows: Vec<Vec<String>> = list
            .iter()
            .skip(first)
            .take(rows_h)
            .map(|r| {
                let mut title = r.title.clone();
                if r.id == view.session_id {
                    title.push_str("  ●");
                }
                vec![
                    wrap::truncate(&title, w.saturating_sub(44).max(12)),
                    relative(now, r.updated_ms),
                    r.messages.to_string(),
                    format_usd(r.spend),
                    r.phase.clone(),
                    r.short.clone(),
                ]
            })
            .collect();
        if n > 0 {
            lines.extend(widgets::table(
                &["title", "updated", "msgs", "spend", "phase", "id"],
                &rows,
                &[
                    widgets::Al::L,
                    widgets::Al::R,
                    widgets::Al::R,
                    widgets::Al::R,
                    widgets::Al::L,
                    widgets::Al::L,
                ],
                Some(sel.saturating_sub(first)),
                w,
                theme,
            ));
        }
        if self.mode == Mode::ConfirmDelete {
            lines.push(widgets::colored(
                "this removes the session directory and its transcript",
                theme.warn,
                theme,
            ));
        }
        Body {
            lines,
            scroll: (n > rows_h).then_some((first, n)),
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        let n = self.filtered(view).len();
        let sel = self.selected.min(n.saturating_sub(1));
        match self.mode {
            Mode::Rename => {
                return match key.code {
                    KeyCode::Esc => {
                        self.mode = Mode::Browse;
                        view.composer.clear();
                        Outcome::Stay
                    }
                    KeyCode::Enter => {
                        let title = view.composer.text().trim().to_string();
                        if title.is_empty() {
                            return Outcome::Stay;
                        }
                        view.composer.clear();
                        self.mode = Mode::Browse;
                        Outcome::Act(Action::RenameSession(title))
                    }
                    _ => {
                        super::edit_field(&mut view.composer, key);
                        Outcome::Stay
                    }
                };
            }
            Mode::ConfirmDelete => {
                return match key.code {
                    KeyCode::Esc => {
                        self.mode = Mode::Browse;
                        view.composer.clear();
                        Outcome::Stay
                    }
                    KeyCode::Enter => {
                        let Some(r) = self.filtered(view).get(sel).cloned().cloned() else {
                            return Outcome::Stay;
                        };
                        if view.composer.text().trim() == r.short {
                            view.composer.clear();
                            self.mode = Mode::Browse;
                            Outcome::Act(Action::DeleteSession(r.id))
                        } else {
                            Outcome::Stay
                        }
                    }
                    _ => {
                        super::edit_field(&mut view.composer, key);
                        Outcome::Stay
                    }
                };
            }
            Mode::Browse => {}
        }
        match key.code {
            KeyCode::Esc => {
                view.composer.clear();
                Outcome::Close
            }
            KeyCode::Up => {
                self.selected = super::step(sel, -1, n);
                Outcome::Stay
            }
            KeyCode::Down => {
                self.selected = super::step(sel, 1, n);
                Outcome::Stay
            }
            KeyCode::Enter => match self.filtered(view).get(sel) {
                Some(r) if r.id == view.session_id => Outcome::Close,
                Some(r) => {
                    let id = r.id.clone();
                    view.composer.clear();
                    Outcome::CloseAct(Action::Resume(id))
                }
                None => Outcome::Stay,
            },
            KeyCode::Char('r') if view.composer.is_empty() => {
                if self.current_is_selected(view) {
                    self.mode = Mode::Rename;
                    view.composer.set_text(&view.session_title);
                }
                Outcome::Stay
            }
            KeyCode::Char('d') if view.composer.is_empty() => {
                if n > 0 && !self.current_is_selected(view) {
                    self.mode = Mode::ConfirmDelete;
                    view.composer.clear();
                }
                Outcome::Stay
            }
            KeyCode::Char('n') if view.composer.is_empty() => {
                view.composer.clear();
                Outcome::CloseAct(Action::New)
            }
            _ => {
                if super::edit_field(&mut view.composer, key) {
                    self.selected = 0;
                }
                Outcome::Stay
            }
        }
    }

    fn on_notice(&mut self, n: &Notice, view: &mut View) {
        if matches!(n, Notice::SessionsChanged) {
            self.reload(view);
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_times() {
        let now = 10_000_000;
        assert_eq!(relative(now, Some(now - 5_000)), "just now");
        assert_eq!(relative(now, Some(now - 120_000)), "2m ago");
        assert_eq!(relative(now, Some(now - 7_200_000)), "2h ago");
        assert_eq!(relative(now, None), "");
    }
}
