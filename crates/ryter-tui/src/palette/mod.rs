//! Command palette: categorized, fuzzy-matched, floating above the composer (`R-PAL-*`).

pub mod matcher;
pub mod registry;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::action::Action;
use crate::chat::wrap;
use crate::panel::chrome::{Chrome, draw_frame};
use crate::theme::Theme;
use crate::view::View;
use registry::{COMMANDS, Category, CommandSpec};

/// Where an entry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `COMMANDS`.
    Builtin,
    /// `~/.ryter/skills` (`⌁`).
    Skill,
    /// `~/.ryter/commands/*.md` (`⌘`).
    UserCommand,
}

/// One palette row candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Name without slash.
    pub name: String,
    /// Aliases (built-ins only).
    pub aliases: Vec<&'static str>,
    /// One line.
    pub description: String,
    /// Group.
    pub category: Category,
    /// Origin.
    pub source: Source,
    /// Shows `▸`.
    pub opens_panel: bool,
    /// Keybinding column.
    pub keybinding: Option<&'static str>,
    /// User entry hidden behind a built-in (`R-PAL-18`).
    pub shadowed: bool,
}

/// Scored row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// Row.
    pub entry: Entry,
    /// Score.
    pub score: u32,
    /// Matched char indices in the name.
    pub hits: Vec<usize>,
}

/// Palette state. The filter lives in the composer (`R-PAL-02`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Palette {
    /// Highlighted match index.
    pub selected: usize,
}

/// Built-ins plus skills and user commands (`R-PAL-17`).
pub fn entries(view: &View) -> Vec<Entry> {
    let mut out: Vec<Entry> = COMMANDS
        .iter()
        .filter(|c| !c.hidden)
        .map(builtin_entry)
        .collect();
    let builtin_names: Vec<&str> = COMMANDS
        .iter()
        .flat_map(|c| std::iter::once(c.name).chain(c.aliases.iter().copied()))
        .collect();
    for s in view.catalog.skills.iter().filter(|s| s.user_invocable) {
        out.push(Entry {
            name: s.name.clone(),
            aliases: Vec::new(),
            description: s.description.clone(),
            category: Category::User,
            source: Source::Skill,
            opens_panel: false,
            keybinding: None,
            shadowed: builtin_names.contains(&s.name.as_str()),
        });
    }
    for c in &view.catalog.commands {
        out.push(Entry {
            name: c.name.clone(),
            aliases: Vec::new(),
            description: c.description.clone(),
            category: Category::User,
            source: Source::UserCommand,
            opens_panel: false,
            keybinding: None,
            shadowed: builtin_names.contains(&c.name.as_str()),
        });
    }
    out
}

fn builtin_entry(c: &'static CommandSpec) -> Entry {
    Entry {
        name: c.name.to_string(),
        aliases: c.aliases.to_vec(),
        description: c.description.to_string(),
        category: c.category,
        source: Source::Builtin,
        opens_panel: c.opens_panel,
        keybinding: c.keybinding,
        shadowed: false,
    }
}

/// Current filter text (composer text after `/`).
pub fn filter(view: &View) -> String {
    view.composer
        .text()
        .trim_start_matches('/')
        .trim()
        .to_string()
}

/// Matches for the current filter, ranked (`R-PAL-09`).
pub fn matches(view: &View) -> Vec<Match> {
    let q = filter(view);
    let mut out: Vec<Match> = entries(view)
        .into_iter()
        .filter_map(|e| {
            let recent = view.recent_commands.iter().any(|r| r == &e.name);
            matcher::score(&q, &e.name, &e.aliases, &e.description, recent).map(|s| Match {
                entry: e,
                score: s.score,
                hits: s.name_hits,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(a.entry.category.cmp(&b.entry.category))
            .then(a.entry.name.cmp(&b.entry.name))
    });
    if q.is_empty() {
        // Flat category order when nothing is typed.
        out.sort_by(|a, b| {
            a.entry
                .category
                .cmp(&b.entry.category)
                .then(b.score.cmp(&a.score))
                .then(a.entry.name.cmp(&b.entry.name))
        });
    }
    out
}

/// Sync the palette with the composer after an edit (`R-PAL-16`).
pub fn refresh(view: &mut View) {
    let text = view.composer.text();
    let open = matches!(view.composer.mode, crate::composer::Mode::Normal)
        && text.starts_with('/')
        && !text.trim_start_matches('/').contains(char::is_whitespace)
        && view.panels.is_empty();
    if open {
        let n = matches(view).len();
        let p = view.palette.get_or_insert_with(Palette::default);
        p.selected = p.selected.min(n.saturating_sub(1));
    } else {
        view.palette = None;
    }
}

/// Open from an empty composer (`Ctrl+P`, `R-PAL-15`).
pub fn open(view: &mut View) {
    if !view.composer.text().starts_with('/') {
        view.composer.set_text("/");
    }
    view.palette = Some(Palette::default());
    refresh(view);
}

/// Move the highlight.
pub fn step(view: &mut View, delta: i32) {
    let n = matches(view).len();
    if n == 0 {
        return;
    }
    if let Some(p) = &mut view.palette {
        p.selected = (p.selected as i32 + delta).rem_euclid(n as i32) as usize;
    }
}

/// Highlighted match.
pub fn current(view: &View) -> Option<Match> {
    let p = view.palette.as_ref()?;
    matches(view).into_iter().nth(p.selected)
}

/// `Tab`: complete the name into the composer without running (`R-PAL-13`).
pub fn complete(view: &mut View) {
    if let Some(m) = current(view) {
        view.composer.set_text(&format!("/{} ", m.entry.name));
        view.palette = None;
    }
}

/// `Enter`: run the highlighted command; with zero matches, send as a message.
pub fn run(view: &mut View) -> Action {
    match current(view) {
        Some(m) => {
            view.palette = None;
            let text = view.composer.take();
            let rest = text
                .trim_start_matches('/')
                .split_once(char::is_whitespace)
                .map(|(_, r)| r.trim().to_string())
                .unwrap_or_default();
            let line = if rest.is_empty() {
                m.entry.name.clone()
            } else {
                format!("{} {rest}", m.entry.name)
            };
            registry::run_command(view, &line)
        }
        None => {
            view.palette = None;
            let text = view.composer.take();
            let trimmed = text.trim().to_string();
            if trimmed.is_empty() {
                return Action::None;
            }
            view.submit_user(trimmed.clone(), trimmed)
        }
    }
}

/// `→`: open the highlighted command's panel (`R-PAL-14`).
pub fn open_row(view: &mut View) -> Action {
    match current(view) {
        Some(m) if m.entry.opens_panel => {
            view.palette = None;
            view.composer.clear();
            registry::run_command(view, &m.entry.name)
        }
        _ => Action::None,
    }
}

/// `Esc`: close and keep the text; a second `Esc` clears it.
pub fn close(view: &mut View) {
    if view.palette.take().is_none() {
        view.composer.clear();
    }
}

/// Floating panel geometry above the composer (`R-PAL-01`).
pub fn rect(chat: Rect, composer_top: u16, rows: usize) -> Rect {
    let width = chat.width.clamp(24, 72);
    let body_h = composer_top.saturating_sub(chat.y);
    let max_h = body_h.saturating_sub(4).max(6);
    let height = u16::try_from(rows + 2).unwrap_or(max_h).clamp(6, max_h);
    let y = composer_top.saturating_sub(height);
    Rect {
        x: chat.x,
        y: y.max(chat.y),
        width,
        height,
    }
}

/// Rows the palette wants (headers + matches, plus the echoed filter).
fn body_rows(view: &View, ms: &[Match], q: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    rows.push(Row::Filter);
    if ms.is_empty() {
        rows.push(Row::Empty);
        return rows;
    }
    let with_headers = q.is_empty() || ms.len() >= 8;
    let mut last: Option<Category> = None;
    for (i, m) in ms.iter().enumerate() {
        if with_headers && last != Some(m.entry.category) {
            if last.is_some() {
                rows.push(Row::Blank);
            }
            rows.push(Row::Header(m.entry.category));
            last = Some(m.entry.category);
        }
        rows.push(Row::Match(i));
    }
    let _ = view;
    rows
}

enum Row {
    Filter,
    Blank,
    Header(Category),
    Match(usize),
    Empty,
}

/// Draw the palette over `chat`, above `composer_top`.
pub fn draw(frame: &mut Frame, chat: Rect, composer_top: u16, view: &View, theme: Theme) {
    let Some(p) = &view.palette else {
        return;
    };
    let ms = matches(view);
    let q = filter(view);
    let rows = body_rows(view, &ms, &q);
    let area = rect(chat, composer_top, rows.len());
    let total = entries(view).len();
    let chrome = Chrome {
        title: "commands".into(),
        status: format!("{} of {}", ms.len(), total),
        legend: "enter run · tab complete · esc close".into(),
        border: theme.panel_border,
        heavy: false,
    };
    let inner = draw_frame(frame, area, &chrome, theme);
    let width = inner.width as usize;
    // Keep the selection visible.
    let sel_row = rows
        .iter()
        .position(|r| matches!(r, Row::Match(i) if *i == p.selected))
        .unwrap_or(0);
    let vis = inner.height as usize;
    let start = (sel_row + 1).saturating_sub(vis);
    let mut lines: Vec<Line> = Vec::new();
    for row in rows.iter().skip(start).take(vis) {
        match row {
            Row::Filter => {
                lines.push(Line::from(vec![
                    Span::styled(" ", theme.panel()),
                    Span::styled(
                        if q.is_empty() {
                            "type to filter".to_string()
                        } else {
                            q.clone()
                        },
                        if q.is_empty() {
                            theme.panel_muted()
                        } else {
                            theme.panel()
                        },
                    ),
                ]));
            }
            Row::Blank => lines.push(Line::from(Span::styled("", theme.panel()))),
            Row::Header(c) => lines.push(Line::from(Span::styled(
                format!("  {}", c.title()),
                theme.panel_muted().add_modifier(Modifier::BOLD),
            ))),
            Row::Empty => {
                lines.push(Line::from(Span::styled(
                    format!(" no command matches \"{q}\""),
                    theme.panel_muted(),
                )));
                lines.push(Line::from(Span::styled(
                    " press enter to send it as a message instead",
                    theme.panel_muted(),
                )));
            }
            Row::Match(i) => lines.push(match_line(&ms[*i], *i == p.selected, width, theme)),
        }
    }
    frame.render_widget(Paragraph::new(lines).style(theme.panel()), inner);
}

fn match_line(m: &Match, selected: bool, width: usize, theme: Theme) -> Line<'static> {
    let bg = if selected {
        theme.selection_bg
    } else {
        theme.panel_bg
    };
    let base = Style::default().fg(theme.fg).bg(bg);
    let dim = Style::default().fg(theme.dim).bg(bg);
    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::styled(
        if selected { "›" } else { " " },
        Style::default().fg(theme.accent).bg(bg),
    ));
    // Name with matched chars highlighted (`R-PAL-10`).
    let name = format!("/{}", m.entry.name);
    let hit_style = Style::default()
        .fg(theme.accent)
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    let mut name_spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut run_hit = false;
    for (i, c) in name.chars().enumerate() {
        let hit = i > 0 && m.hits.contains(&(i - 1));
        if !run.is_empty() && hit != run_hit {
            name_spans.push(Span::styled(
                std::mem::take(&mut run),
                if run_hit { hit_style } else { base },
            ));
        }
        run_hit = hit;
        run.push(c);
    }
    if !run.is_empty() {
        name_spans.push(Span::styled(run, if run_hit { hit_style } else { base }));
    }
    let name_w = wrap::width(&name);
    let name_col = 16usize;
    spans.extend(name_spans);
    if name_w < name_col {
        spans.push(Span::styled(" ".repeat(name_col - name_w), base));
    } else {
        spans.push(Span::styled(" ", base));
    }
    let right = match (m.entry.source, m.entry.opens_panel, m.entry.keybinding) {
        (Source::Skill, _, _) => "⌁".to_string(),
        (Source::UserCommand, _, _) => "⌘".to_string(),
        (_, true, _) => "▸".to_string(),
        (_, false, Some(k)) => crate::keymap::display(k),
        _ => String::new(),
    };
    let used = 1 + name_w.max(name_col) + 1;
    let mut desc = m.entry.description.clone();
    if m.entry.shadowed {
        desc.push_str(" (shadowed)");
    }
    let room = width.saturating_sub(used + wrap::width(&right) + 2);
    let desc = wrap::truncate(&desc, room);
    spans.push(Span::styled(desc.clone(), dim));
    let pad = width.saturating_sub(used + wrap::width(&desc) + wrap::width(&right) + 1);
    spans.push(Span::styled(" ".repeat(pad), base));
    spans.push(Span::styled(right, dim));
    spans.push(Span::styled(" ", base));
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ryter_core::Phase;

    fn view() -> View {
        View::new(
            Phase::Build,
            "spacexai".into(),
            "grok-4.6".into(),
            "p".into(),
        )
    }

    #[test]
    fn mdl_ranks_models_first_and_every_spec_is_reachable() {
        let mut v = view();
        v.composer.set_text("/mdl");
        v.palette = Some(Palette::default());
        let ms = matches(&v);
        assert_eq!(
            ms[0].entry.name,
            "models",
            "{:?}",
            ms.iter().map(|m| &m.entry.name).collect::<Vec<_>>()
        );
        v.composer.set_text("/");
        let all = matches(&v);
        for c in COMMANDS.iter().filter(|c| !c.hidden) {
            assert!(
                all.iter().any(|m| m.entry.name == c.name),
                "/{} unreachable",
                c.name
            );
        }
        // Hidden aliases are runnable but not listed.
        assert!(!all.iter().any(|m| m.entry.name == "auto"));
        assert!(registry::find("auto").is_some());
    }

    #[test]
    fn tab_completes_and_space_closes() {
        let mut v = view();
        v.composer.set_text("/prov");
        refresh(&mut v);
        assert!(v.palette.is_some());
        complete(&mut v);
        assert_eq!(v.composer.text(), "/provider ");
        assert!(v.palette.is_none());
        v.composer.set_text("/rename web");
        refresh(&mut v);
        assert!(v.palette.is_none());
    }

    #[test]
    fn enter_runs_highlighted_and_zero_matches_sends_message() {
        let mut v = view();
        v.composer.set_text("/");
        open(&mut v);
        let idx = matches(&v)
            .iter()
            .position(|m| m.entry.name == "provider")
            .unwrap();
        v.palette.as_mut().unwrap().selected = idx;
        assert_eq!(
            run(&mut v),
            Action::OpenPanel(crate::action::PanelId::Providers)
        );
        assert!(v.composer.text().is_empty());
        v.composer.set_text("/zzzq");
        open(&mut v);
        assert!(matches(&v).is_empty());
        let act = run(&mut v);
        assert!(matches!(act, Action::Submit(s) if s == "/zzzq"));
    }

    #[test]
    fn user_skills_are_listed_with_shadow_note() {
        let mut v = view();
        v.catalog.skills.push(ryter_core::Skill {
            name: "review".into(),
            description: "Review the diff".into(),
            user_invocable: true,
            body: "look".into(),
            source: std::path::PathBuf::from("/tmp/SKILL.md"),
        });
        v.catalog.skills.push(ryter_core::Skill {
            name: "help".into(),
            description: "shadow".into(),
            user_invocable: true,
            body: "x".into(),
            source: std::path::PathBuf::from("/tmp/h.md"),
        });
        v.composer.set_text("/re");
        open(&mut v);
        assert!(
            matches(&v)
                .iter()
                .any(|m| m.entry.name == "review" && m.entry.source == Source::Skill)
        );
        let es = entries(&v);
        assert!(es.iter().any(|e| e.name == "help" && e.shadowed));
        let s = crate::draw::render_to_string(&v, 100, 30);
        assert!(s.contains("commands"), "{s}");
        assert!(s.contains("/review"), "{s}");
    }
}
