//! `/storage` — what Ryter keeps for the project, and a cleanup shown in
//! full before it is done.
//!
//! The figures come first: the project's sessions, recovery copies and
//! checkpoints, with their sizes and ages. Under them, three cleanups, each
//! with what it would remove. Enter opens one's preview: what goes, what is
//! lost with it, what stays, and the question. Nothing is removed before
//! the yes, and the yes reads everything again before it removes anything.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::text::Line;
use ryter_core::storage::{self, Cleanup, Plan, Report};

use super::{Body, Notice, Outcome, Panel, PanelEnv, widgets};
use crate::action::Action;
use crate::chat::wrap;
use crate::theme::Theme;
use crate::view::View;

/// Rows of the preview's list before `and N more`.
const LISTED: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Browse,
    Preview,
}

/// The storage panel.
#[derive(Debug, Clone)]
pub struct Storage {
    home: std::path::PathBuf,
    cwd: std::path::PathBuf,
    current: String,
    report: Result<Report, String>,
    plans: Vec<Plan>,
    selected: usize,
    /// Index into [`storage::AGES`].
    age: usize,
    mode: Mode,
    /// What the last cleanup did, shown above the figures.
    last: Option<String>,
}

fn wall_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Storage {
    /// Read the figures for `env.cwd`.
    pub fn new(view: &View, env: &PanelEnv) -> Self {
        let mut s = Self {
            home: env.home.clone(),
            cwd: env.cwd.clone(),
            current: view.session_id.clone(),
            report: Err(String::new()),
            plans: Vec::new(),
            selected: 0,
            age: storage::AGES
                .iter()
                .position(|d| *d == storage::DEFAULT_AGE)
                .unwrap_or(0),
            mode: Mode::Browse,
            last: None,
        };
        s.reload();
        s
    }

    fn reload(&mut self) {
        self.report = storage::report(&self.home, &self.cwd, wall_ms()).map_err(|e| e.to_string());
        self.replan();
    }

    fn replan(&mut self) {
        let Ok(report) = &self.report else {
            self.plans.clear();
            return;
        };
        let current = (!self.current.is_empty()).then_some(self.current.as_str());
        self.plans = self
            .cleanups()
            .into_iter()
            .map(|c| storage::plan(report, c, current))
            .collect();
    }

    fn cleanups(&self) -> [Cleanup; 3] {
        [
            Cleanup::Unreachable,
            Cleanup::OlderThan(storage::AGES[self.age]),
            Cleanup::Copies,
        ]
    }

    fn chosen(&self) -> Option<&Plan> {
        self.plans.get(self.selected)
    }

    fn browse(&self, w: usize, theme: Theme) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let report = match &self.report {
            Ok(r) => r,
            Err(e) => {
                lines.push(widgets::colored(
                    &format!("could not read the storage: {e}"),
                    theme.error,
                    theme,
                ));
                return lines;
            }
        };
        if let Some(last) = &self.last {
            for row in wrap::wrap_plain(last, w.saturating_sub(2).max(20)) {
                lines.push(widgets::colored(&row, theme.success, theme));
            }
        }
        lines.push(widgets::note(
            &wrap::truncate(
                &format!("project {}", report.root.display()),
                w.saturating_sub(2),
            ),
            theme,
        ));
        let figures: Vec<Vec<String>> = report
            .rows()
            .into_iter()
            .map(|(what, value)| vec![what, value])
            .collect();
        lines.extend(
            widgets::table(
                &["kept", ""],
                &figures,
                &[widgets::Al::L, widgets::Al::L],
                None,
                w,
                theme,
            )
            .into_iter()
            .skip(1),
        );
        lines.push(widgets::blank(theme));
        let rows: Vec<Vec<String>> = self
            .plans
            .iter()
            .map(|p| {
                let label = match p.cleanup {
                    Cleanup::OlderThan(days) => {
                        format!("sessions unused for ‹{days}› days")
                    }
                    _ => p.label(),
                };
                vec![label, p.amount()]
            })
            .collect();
        lines.extend(widgets::table(
            &["clean up", "removes"],
            &rows,
            &[widgets::Al::L, widgets::Al::L],
            Some(self.selected),
            w,
            theme,
        ));
        lines
    }

    fn preview(&self, w: usize, h: usize, theme: Theme) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let Some(p) = self.chosen() else {
            return lines;
        };
        let room = w.saturating_sub(2).max(20);
        let now = wall_ms();
        let mut items: Vec<String> = Vec::new();
        for s in &p.sessions {
            items.push(format!(
                "{}  {}  {}  {}",
                s.short(),
                s.updated_ms
                    .map(|t| storage::ago(now, t))
                    .unwrap_or_default(),
                storage::format_bytes(s.bytes),
                s.title
            ));
        }
        for c in &p.copies {
            items.push(format!(
                "{}  {}",
                storage::format_bytes(c.bytes),
                c.path.strip_prefix(&self.home).unwrap_or(&c.path).display()
            ));
        }
        if p.sessions.is_empty() {
            for c in &p.checkpoints {
                items.push(format!(
                    "{}  {}",
                    c.made_ms.map(|t| storage::ago(now, t)).unwrap_or_default(),
                    c.name.trim_start_matches("refs/ryter/undo/")
                ));
            }
        }
        // The sentences and the question are whole whatever the height; the
        // list gives up rows to them.
        let mut tail: Vec<Line<'static>> = Vec::new();
        for sentence in p.explain() {
            for row in wrap::wrap_plain(&sentence, room) {
                tail.push(widgets::text(&row, theme));
            }
        }
        if !p.is_empty() {
            tail.push(widgets::blank(theme));
            tail.push(widgets::colored(
                &wrap::truncate(&format!("clean up: {}?", p.amount()), room),
                theme.warn,
                theme,
            ));
        }
        // Rows left for the list, and the blank under it.
        let avail = h.saturating_sub(tail.len()).min(LISTED + 2);
        if !items.is_empty() && avail == 1 {
            let only = match items.len() {
                1 => items[0].clone(),
                n => format!("{} and {} more", items[0], n - 1),
            };
            lines.push(widgets::text(&wrap::truncate(&only, room), theme));
        } else if !items.is_empty() && avail > 1 {
            let room_rows = avail - 1;
            let shown = if items.len() <= room_rows {
                items.len()
            } else {
                room_rows - 1
            };
            for line in items.iter().take(shown) {
                lines.push(widgets::text(&wrap::truncate(line, room), theme));
            }
            if items.len() > shown {
                lines.push(widgets::note(
                    &format!("and {} more", items.len() - shown),
                    theme,
                ));
            }
            lines.push(widgets::blank(theme));
        }
        lines.extend(tail);
        lines
    }
}

impl Panel for Storage {
    fn kind(&self) -> &'static str {
        "storage"
    }

    fn title(&self, _view: &View) -> String {
        match (self.mode, self.chosen()) {
            (Mode::Preview, Some(p)) => format!("clean up · {}", p.label()),
            _ => "storage".into(),
        }
    }

    fn legend(&self, _view: &View) -> String {
        match self.mode {
            Mode::Browse => "↑↓ choose · ←→ age · enter preview · esc".into(),
            Mode::Preview if self.chosen().is_some_and(|p| !p.is_empty()) => {
                "⏎ clean up · n keep · esc".into()
            }
            Mode::Preview => "esc back".into(),
        }
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        let rows = match (self.mode, self.chosen()) {
            // As tall as the preview needs at an 80-column panel, up to
            // the list's cap: no empty band above the legend.
            (Mode::Preview, Some(p)) => {
                let items = p.sessions.len()
                    + p.copies.len()
                    + if p.sessions.is_empty() {
                        p.checkpoints.len()
                    } else {
                        0
                    };
                let listed = if items == 0 {
                    0
                } else {
                    items.min(LISTED + 1) + 1
                };
                let said: usize = p
                    .explain()
                    .iter()
                    .map(|s| wrap::wrap_plain(s, 72).len())
                    .sum();
                listed + said + if p.is_empty() { 0 } else { 2 }
            }
            _ => {
                let figures = self.report.as_ref().map_or(1, |r| r.rows().len() + 1);
                figures + usize::from(self.last.is_some()) * 2 + 1 + 1 + self.plans.len()
            }
        };
        (90, rows.clamp(6, 24) as u16)
    }

    fn render(&self, _view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let h = usize::from(height).max(1);
        let all = match self.mode {
            Mode::Browse => self.browse(w, theme),
            Mode::Preview => self.preview(w, h, theme),
        };
        let n = all.len();
        // The question is the preview's last row: a long list gives up
        // rows above it, never the question.
        let lines = if n > h && self.mode == Mode::Preview {
            let mut kept: Vec<Line<'static>> = all[..h.saturating_sub(3)].to_vec();
            kept.extend_from_slice(&all[n - 3..]);
            kept
        } else {
            all.into_iter().take(h).collect()
        };
        Body {
            lines,
            scroll: None,
        }
    }

    fn key(&mut self, key: KeyEvent, _view: &mut View) -> Outcome {
        match self.mode {
            Mode::Preview => match key.code {
                KeyCode::Esc | KeyCode::Char('n' | 'N') => {
                    self.mode = Mode::Browse;
                    Outcome::Stay
                }
                KeyCode::Enter | KeyCode::Char('y' | 'Y') => {
                    let Some(p) = self.chosen().filter(|p| !p.is_empty()).cloned() else {
                        return Outcome::Stay;
                    };
                    self.mode = Mode::Browse;
                    Outcome::Act(Action::CleanStorage(Box::new(p)))
                }
                _ => Outcome::Stay,
            },
            Mode::Browse => match key.code {
                KeyCode::Esc => Outcome::Close,
                KeyCode::Up => {
                    self.selected = super::step(self.selected, -1, self.plans.len());
                    Outcome::Stay
                }
                KeyCode::Down => {
                    self.selected = super::step(self.selected, 1, self.plans.len());
                    Outcome::Stay
                }
                KeyCode::Left | KeyCode::Right
                    if matches!(
                        self.chosen().map(|p| p.cleanup),
                        Some(Cleanup::OlderThan(_))
                    ) =>
                {
                    let last = storage::AGES.len() - 1;
                    self.age = if key.code == KeyCode::Left {
                        self.age.saturating_sub(1)
                    } else {
                        (self.age + 1).min(last)
                    };
                    self.replan();
                    Outcome::Stay
                }
                KeyCode::Enter if self.chosen().is_some() => {
                    self.mode = Mode::Preview;
                    Outcome::Stay
                }
                _ => Outcome::Stay,
            },
        }
    }

    fn on_notice(&mut self, n: &Notice, _view: &mut View) {
        match n {
            Notice::SessionsChanged => self.reload(),
            Notice::StorageCleaned(said) => self.last = Some(said.clone()),
            _ => {}
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}
