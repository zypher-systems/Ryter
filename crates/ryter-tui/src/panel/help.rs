//! `/help` — keymap and command reference (`R-POP-67..70`).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::{Body, Outcome, Panel, widgets};
use crate::chat::wrap;
use crate::keymap::{self, Ctx};
use crate::palette::registry::{COMMANDS, Category};
use crate::theme::Theme;
use crate::view::View;

/// Help panel.
#[derive(Debug, Clone, Default)]
pub struct Help {
    /// 0 = keys, 1 = commands.
    tab: usize,
    scroll: usize,
}

enum Row {
    Header(String),
    Pair(String, String),
    Blank,
}

impl Help {
    fn rows(&self, filter: &str) -> Vec<Row> {
        let f = filter.trim().to_ascii_lowercase();
        let hit = |a: &str, b: &str| {
            f.is_empty()
                || a.to_ascii_lowercase().contains(&f)
                || b.to_ascii_lowercase().contains(&f)
        };
        let mut out = Vec::new();
        if self.tab == 0 {
            for ctx in Ctx::ALL {
                let bs: Vec<_> = keymap::bindings(ctx)
                    .into_iter()
                    .filter(|b| hit(b.key, b.help))
                    .collect();
                if bs.is_empty() {
                    continue;
                }
                out.push(Row::Header(ctx.title().into()));
                for b in bs {
                    out.push(Row::Pair(keymap::display(b.key), b.help.into()));
                }
                out.push(Row::Blank);
            }
        } else {
            for cat in Category::ALL {
                let cs: Vec<_> = COMMANDS
                    .iter()
                    .filter(|c| c.category == cat && !c.hidden && hit(c.name, c.description))
                    .collect();
                if cs.is_empty() {
                    continue;
                }
                out.push(Row::Header(cat.title().into()));
                for c in cs {
                    let name = match c.usage {
                        Some(u) => format!("/{} {u}", c.name),
                        None => format!("/{}", c.name),
                    };
                    let mut desc = c.description.to_string();
                    if let Some(k) = c.keybinding {
                        desc.push_str(&format!("  ({})", keymap::display(k)));
                    }
                    out.push(Row::Pair(name, desc));
                }
                out.push(Row::Blank);
            }
        }
        while matches!(out.last(), Some(Row::Blank)) {
            out.pop();
        }
        out
    }
}

impl Panel for Help {
    fn kind(&self) -> &'static str {
        "help"
    }

    fn title(&self, _view: &View) -> String {
        "help".into()
    }

    fn status(&self, _view: &View) -> String {
        if self.tab == 0 {
            "[keys] commands".into()
        } else {
            "keys [commands]".into()
        }
    }

    fn legend(&self, _view: &View) -> String {
        "←→ tab · ↑↓ pgup pgdn scroll · type to filter · esc".into()
    }

    fn input(&self, _view: &View) -> Option<String> {
        Some("filter".into())
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        (84, 28)
    }

    fn render(&self, view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let h = usize::from(height).max(1);
        let rows = self.rows(view.composer.text());
        let total = rows.len();
        let first = self.scroll.min(total.saturating_sub(h));
        let key_w = rows
            .iter()
            .filter_map(|r| match r {
                Row::Pair(k, _) => Some(wrap::width(k)),
                _ => None,
            })
            .max()
            .unwrap_or(10)
            .min(w / 2);
        let mut lines: Vec<Line<'static>> = Vec::new();
        for r in rows.iter().skip(first).take(h) {
            match r {
                Row::Header(t) => lines.push(Line::from(Span::styled(
                    format!(" {}", t.to_ascii_uppercase()),
                    theme.panel_muted().add_modifier(Modifier::BOLD),
                ))),
                Row::Pair(k, d) => {
                    let d = wrap::truncate(d, w.saturating_sub(key_w + 5));
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("  {}", wrap::pad_right(k, key_w)),
                            Style::default().fg(theme.accent).bg(theme.panel_bg),
                        ),
                        Span::styled(format!("  {d}"), theme.panel()),
                    ]));
                }
                Row::Blank => lines.push(widgets::blank(theme)),
            }
        }
        if total == 0 {
            lines.push(widgets::note("no matches", theme));
        }
        Body {
            lines,
            scroll: (total > h).then_some((first, total)),
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        match key.code {
            KeyCode::Esc => Outcome::Close,
            KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                self.tab = 1 - self.tab;
                self.scroll = 0;
                Outcome::Stay
            }
            KeyCode::Up => {
                self.scroll = self.scroll.saturating_sub(1);
                Outcome::Stay
            }
            KeyCode::Down => {
                self.scroll += 1;
                Outcome::Stay
            }
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_sub(10);
                Outcome::Stay
            }
            KeyCode::PageDown => {
                self.scroll += 10;
                Outcome::Stay
            }
            KeyCode::Enter => Outcome::Stay,
            _ => {
                super::edit_field(&mut view.composer, key);
                self.scroll = 0;
                Outcome::Stay
            }
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}
