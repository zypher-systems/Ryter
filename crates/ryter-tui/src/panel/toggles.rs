//! `/tools` and `/auditor` — small toggles with prose (`R-POP-54`, `R-POP-55`).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::{Body, Outcome, Panel, widgets};
use crate::action::Action;
use crate::chat::wrap;
use crate::theme::Theme;
use crate::view::View;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Which {
    Tools,
    Auditor,
}

struct Opt {
    value: &'static str,
    prose: &'static str,
    warn: bool,
}

const TOOLS: [Opt; 2] = [
    Opt {
        value: "ask",
        prose: "shell, write, and delete calls pause for a y/n before they run.",
        warn: false,
    },
    Opt {
        value: "always",
        prose: "tools run without asking, including destructive ones.",
        warn: true,
    },
];

const AUDITOR: [Opt; 2] = [
    Opt {
        value: "on",
        prose: "every builder result is reviewed by an auditor before it merges.",
        warn: false,
    },
    Opt {
        value: "off",
        prose: "builder results merge as soon as they finish. faster, unreviewed.",
        warn: true,
    },
];

/// Two-row toggle panel.
#[derive(Debug, Clone)]
pub struct Toggles {
    which: Which,
    selected: usize,
}

impl Toggles {
    /// `/tools`.
    pub fn tools(view: &View) -> Self {
        Self {
            which: Which::Tools,
            selected: usize::from(view.perm_mode == "always"),
        }
    }

    /// `/auditor`.
    pub fn auditor(view: &View) -> Self {
        Self {
            which: Which::Auditor,
            selected: usize::from(!view.auditor_on),
        }
    }

    fn opts(&self) -> &'static [Opt; 2] {
        match self.which {
            Which::Tools => &TOOLS,
            Which::Auditor => &AUDITOR,
        }
    }

    fn current(&self, view: &View) -> usize {
        match self.which {
            Which::Tools => usize::from(view.perm_mode == "always"),
            Which::Auditor => usize::from(!view.auditor_on),
        }
    }
}

impl Panel for Toggles {
    fn kind(&self) -> &'static str {
        match self.which {
            Which::Tools => "tools",
            Which::Auditor => "auditor",
        }
    }

    fn title(&self, _view: &View) -> String {
        match self.which {
            Which::Tools => "tool permissions".into(),
            Which::Auditor => "auditor gate".into(),
        }
    }

    fn legend(&self, _view: &View) -> String {
        "↑↓ move · enter apply · esc  · changes apply immediately".into()
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        (66, 7)
    }

    fn render(&self, view: &View, width: u16, _height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let cur = self.current(view);
        let mut lines: Vec<Line<'static>> = Vec::new();
        for (i, o) in self.opts().iter().enumerate() {
            let sel = i == self.selected;
            let bg = if sel {
                theme.selection_bg
            } else {
                theme.panel_bg
            };
            let color = if o.warn { theme.warn } else { theme.success };
            let mark = if i == cur { "●" } else { "○" };
            let mut spans = vec![
                Span::styled(
                    if sel { "›" } else { " " },
                    Style::default().fg(theme.accent).bg(bg),
                ),
                Span::styled(format!("{mark} "), Style::default().fg(color).bg(bg)),
                Span::styled(
                    wrap::pad_right(o.value, 8),
                    Style::default()
                        .fg(if o.warn { theme.warn } else { theme.fg })
                        .bg(bg)
                        .add_modifier(Modifier::BOLD),
                ),
            ];
            if i == cur {
                spans.push(Span::styled(
                    "current",
                    Style::default().fg(theme.dim).bg(bg),
                ));
            }
            lines.push(Line::from(spans));
            for row in wrap::wrap_plain(o.prose, w.saturating_sub(5)) {
                lines.push(widgets::note(&format!("   {row}"), theme));
            }
            lines.push(widgets::blank(theme));
        }
        Body {
            lines,
            scroll: None,
        }
    }

    fn key(&mut self, key: KeyEvent, _view: &mut View) -> Outcome {
        match key.code {
            KeyCode::Esc => Outcome::Close,
            KeyCode::Up
            | KeyCode::Down
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Tab
            | KeyCode::Char(' ') => {
                self.selected = 1 - self.selected;
                Outcome::Stay
            }
            KeyCode::Enter => {
                let a = match self.which {
                    Which::Tools => Action::SetTools {
                        always: self.selected == 1,
                    },
                    Which::Auditor => Action::SetAuditor(self.selected == 0),
                };
                Outcome::CloseAct(a)
            }
            _ => Outcome::Stay,
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}
