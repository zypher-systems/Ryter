//! `/rules` — the user's standing rules: for every project, and for this one.
//!
//! Both files go into every role's instructions on every message. This panel
//! is where the user reads and changes them by hand, with no model involved:
//! add a rule, remove a line, or open the file in an editor. The model's own
//! way in is the `update_rules` tool, which asks first.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::{Body, Outcome, Panel, PanelEnv, widgets};
use crate::action::Action;
use crate::chat::wrap;
use crate::theme::Theme;
use crate::view::View;

/// Which file is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    /// `~/.ryter/RYTER.md`.
    Every,
    /// `RYTER.md` at the top of the project.
    Project,
}

impl Tab {
    fn index(self) -> usize {
        match self {
            Self::Every => 0,
            Self::Project => 1,
        }
    }

    fn other(self) -> Self {
        match self {
            Self::Every => Self::Project,
            Self::Project => Self::Every,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Every => "every project",
            Self::Project => "this project",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Pane {
    Home,
    /// Typing a new rule.
    Add,
    /// Asked whether to remove line `.0`.
    ConfirmRemove(usize),
}

/// One rules file as the panel holds it.
#[derive(Debug, Clone)]
struct RulesFile {
    /// Where it is, or would be.
    path: PathBuf,
    /// Its name as the user knows it.
    shown: String,
    /// Its lines, without line endings. Empty when there is no file.
    lines: Vec<String>,
    /// There is a file.
    exists: bool,
}

impl RulesFile {
    fn load(path: PathBuf, shown: String, text: Option<String>) -> Self {
        let exists = text.is_some();
        let text = text.unwrap_or_default();
        let lines = text.trim_end().lines().map(str::to_string).collect();
        Self {
            path,
            shown,
            lines,
            exists,
        }
    }

    /// Lines that are rules: the bullets.
    fn rules(&self) -> usize {
        self.lines
            .iter()
            .filter(|l| {
                let l = l.trim_start();
                l.starts_with("- ") || l.starts_with("* ")
            })
            .count()
    }

    fn text(&self) -> String {
        let mut s = self.lines.join("\n");
        if !s.is_empty() {
            s.push('\n');
        }
        s
    }
}

/// The `/rules` panel.
#[derive(Debug, Clone)]
pub struct Rules {
    /// Ryter's home folder.
    home: PathBuf,
    /// The top of the project.
    project: PathBuf,
    tab: Tab,
    /// The selected line of each file.
    selected: [usize; 2],
    pane: Pane,
    files: [RulesFile; 2],
    error: Option<String>,
}

/// The project's rules file: `RYTER.md`, or `AGENTS.md` when that is the
/// one there. Ryter reads them in that order.
fn project_file(root: &Path) -> (PathBuf, &'static str) {
    if !root.join("RYTER.md").is_file() && root.join("AGENTS.md").is_file() {
        (root.join("AGENTS.md"), "./AGENTS.md")
    } else {
        (root.join("RYTER.md"), "./RYTER.md")
    }
}

impl Rules {
    /// Build, reading both files.
    pub fn new(view: &View, env: &PanelEnv) -> Self {
        let project = view
            .project_root
            .as_ref()
            .map_or_else(|| env.cwd.clone(), PathBuf::from);
        let mut panel = Self {
            home: env.home.clone(),
            project,
            tab: Tab::Every,
            selected: [0, 0],
            pane: Pane::Home,
            files: [
                RulesFile::load(PathBuf::new(), String::new(), None),
                RulesFile::load(PathBuf::new(), String::new(), None),
            ],
            error: None,
        };
        panel.reload();
        panel
    }

    /// Read both files again: after a change, and when the panel opens.
    fn reload(&mut self) {
        self.files[0] = RulesFile::load(
            ryter_core::rules::path(&self.home),
            ryter_core::rules::shown(&self.home),
            ryter_core::rules::read(&self.home),
        );
        let (path, shown) = project_file(&self.project);
        let text = std::fs::read_to_string(&path).ok();
        self.files[1] = RulesFile::load(path, shown.to_string(), text);
        for tab in [Tab::Every, Tab::Project] {
            let i = tab.index();
            let last = self.files[i].lines.len().saturating_sub(1);
            self.selected[i] = self.selected[i].min(last);
        }
    }

    fn file(&self) -> &RulesFile {
        &self.files[self.tab.index()]
    }

    fn sel(&self) -> usize {
        self.selected[self.tab.index()]
    }

    /// Move the selection by `delta`, over the blank lines.
    fn step(&mut self, delta: i32) {
        let lines = &self.file().lines;
        let n = lines.len();
        if n == 0 {
            return;
        }
        let mut i = self.sel();
        for _ in 0..n {
            i = (i as i32 + delta).rem_euclid(n as i32) as usize;
            if !lines[i].trim().is_empty() {
                break;
            }
        }
        self.selected[self.tab.index()] = i;
    }

    /// Write the file on screen as `lines`.
    fn save(&mut self, lines: Vec<String>) -> Result<(), String> {
        let i = self.tab.index();
        let mut next = self.files[i].clone();
        next.lines = lines;
        let text = next.text();
        match self.tab {
            Tab::Every => {
                if text.len() > ryter_core::rules::MAX_BYTES {
                    return Err(format!(
                        "that would pass the {} KB Ryter loads: shorten a rule first",
                        ryter_core::rules::MAX_BYTES / 1024
                    ));
                }
                ryter_core::rules::save(&self.home, &text).map_err(|e| e.to_string())?;
            }
            Tab::Project => {
                std::fs::write(&next.path, text)
                    .map_err(|e| format!("{}: {e}", next.path.display()))?;
            }
        }
        self.reload();
        Ok(())
    }

    /// Add `typed` as a rule under the selected line. A line that is
    /// already a bullet or a heading is kept as typed.
    fn add(&mut self, typed: &str) -> Result<String, String> {
        let typed = typed.trim();
        if typed.is_empty() {
            return Err("type the rule, then enter".into());
        }
        let line = if typed.starts_with("- ") || typed.starts_with("* ") || typed.starts_with('#') {
            typed.to_string()
        } else {
            format!("- {typed}")
        };
        let mut lines = self.file().lines.clone();
        let at = if lines.is_empty() { 0 } else { self.sel() + 1 };
        lines.insert(at.min(lines.len()), line);
        self.save(lines)?;
        self.selected[self.tab.index()] = at;
        Ok(format!("rules · added to {}", self.file().shown))
    }

    fn remove(&mut self, index: usize) -> Result<String, String> {
        let mut lines = self.file().lines.clone();
        if index >= lines.len() {
            return Err("that line is gone".into());
        }
        lines.remove(index);
        self.save(lines)?;
        Ok(format!("rules · removed from {}", self.file().shown))
    }

    fn back(&mut self, view: &mut View) {
        view.composer.clear();
        self.pane = Pane::Home;
    }
}

impl Panel for Rules {
    fn kind(&self) -> &'static str {
        "rules"
    }

    fn title(&self, _view: &View) -> String {
        "rules".into()
    }

    fn legend(&self, _view: &View) -> String {
        match self.pane {
            Pane::Home => "tab file · a add · d remove · e editor · esc close".into(),
            Pane::Add => "type the rule · enter add · esc back".into(),
            Pane::ConfirmRemove(_) => "y remove · n keep".into(),
        }
    }

    fn input(&self, _view: &View) -> Option<String> {
        matches!(self.pane, Pane::Add).then(|| "new rule".into())
    }

    fn size(&self, _view: &View) -> (u16, u16) {
        let longest = self.files.iter().map(|f| f.lines.len()).max().unwrap_or(0);
        (78, (longest + 6).clamp(10, 24) as u16)
    }

    fn render(&self, _view: &View, width: u16, height: u16, theme: Theme) -> Body {
        let w = usize::from(width);
        let h = usize::from(height).max(1);
        let file = self.file();
        let dim = theme.panel_muted();
        let mut lines: Vec<Line<'static>> = Vec::new();

        // The two files, the one on screen in capitals.
        let mut tabs = vec![Span::styled("  ", theme.panel())];
        for tab in [Tab::Every, Tab::Project] {
            if tab == self.tab {
                tabs.push(Span::styled(
                    tab.name().to_ascii_uppercase(),
                    Style::default()
                        .fg(theme.accent)
                        .bg(theme.panel_bg)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                tabs.push(Span::styled(tab.name().to_string(), dim));
            }
            tabs.push(Span::styled("    ", theme.panel()));
        }
        lines.push(Line::from(tabs));
        let count = file.rules();
        let state = if file.exists {
            format!("{count} rule{}", if count == 1 { "" } else { "s" })
        } else {
            "not created yet".to_string()
        };
        // A long path loses its start, not the count after it.
        let room = w.saturating_sub(2 + 3 + wrap::width(&state));
        let shown: String = if wrap::width(&file.shown) > room {
            let keep = room.saturating_sub(1);
            let tail: String = file
                .shown
                .chars()
                .rev()
                .take(keep)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!("…{tail}")
        } else {
            file.shown.clone()
        };
        lines.push(Line::from(Span::styled(
            format!("  {shown} · {state}"),
            dim,
        )));
        lines.push(widgets::blank(theme));

        // What sits under the list.
        let mut foot: Vec<Line<'static>> = Vec::new();
        if let Pane::ConfirmRemove(i) = self.pane {
            if let Some(line) = file.lines.get(i) {
                foot.push(widgets::colored(
                    &wrap::truncate(&format!("remove \"{}\"?", line.trim()), w.saturating_sub(2)),
                    theme.warn,
                    theme,
                ));
            }
        }
        if let Some(e) = &self.error {
            foot.push(widgets::colored(&format!("✕ {e}"), theme.error, theme));
        }

        if file.lines.is_empty() {
            let hint = match self.tab {
                Tab::Every => {
                    "No rules yet. These go to every role, in every project: how you like \
                     work reported, what to ask before doing, spelling, tone."
                }
                Tab::Project => {
                    "No rules for this project yet. Its commands, its conventions, what \
                     not to touch."
                }
            };
            for row in wrap::wrap_plain(hint, w.saturating_sub(4)) {
                lines.push(Line::from(Span::styled(format!("  {row}"), dim)));
            }
            lines.push(widgets::blank(theme));
            lines.push(Line::from(Span::styled("  a adds the first one.", dim)));
            lines.extend(foot);
            return Body {
                lines,
                scroll: None,
            };
        }

        // Every line, wrapped under its own text, and a window on them that
        // keeps the selected one in view.
        let text_w = w.saturating_sub(4).max(8);
        let mut rows: Vec<Line<'static>> = Vec::new();
        let mut first_row_of = Vec::with_capacity(file.lines.len());
        for (i, line) in file.lines.iter().enumerate() {
            first_row_of.push(rows.len());
            let sel = i == self.sel();
            let bg = if sel {
                theme.selection_bg
            } else {
                theme.panel_bg
            };
            let heading = line.trim_start().starts_with('#');
            let style = if heading {
                Style::default()
                    .fg(theme.fg)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg).bg(bg)
            };
            // A bullet's later rows sit under its text, not its dash.
            let hang = if line.trim_start().starts_with("- ") || line.trim_start().starts_with("* ")
            {
                2
            } else {
                0
            };
            let pieces = if line.trim().is_empty() {
                vec![String::new()]
            } else {
                wrap::wrap_plain(line, text_w.saturating_sub(hang))
            };
            for (pi, piece) in pieces.iter().enumerate() {
                let mark = if sel && pi == 0 { "›" } else { " " };
                let indent = if pi == 0 { 0 } else { hang };
                let body = format!(" {}{piece}", " ".repeat(indent));
                let used = 1 + wrap::width(&body);
                rows.push(Line::from(vec![
                    Span::styled(mark, Style::default().fg(theme.accent).bg(bg)),
                    Span::styled(body, style),
                    Span::styled(" ".repeat(w.saturating_sub(used)), style),
                ]));
            }
        }
        let room = h.saturating_sub(lines.len() + foot.len()).max(1);
        let total = rows.len();
        let at = first_row_of.get(self.sel()).copied().unwrap_or(0);
        let first = super::window(at, total, room);
        lines.extend(rows.into_iter().skip(first).take(room));
        lines.extend(foot);
        Body {
            lines,
            scroll: (total > room).then_some((first, total)),
        }
    }

    fn key(&mut self, key: KeyEvent, view: &mut View) -> Outcome {
        match self.pane.clone() {
            Pane::Home => match key.code {
                KeyCode::Esc => Outcome::Close,
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                    self.tab = self.tab.other();
                    self.error = None;
                    Outcome::Stay
                }
                KeyCode::Up => {
                    self.step(-1);
                    Outcome::Stay
                }
                KeyCode::Down => {
                    self.step(1);
                    Outcome::Stay
                }
                KeyCode::Char('a' | 'A') => {
                    self.error = None;
                    view.composer.clear();
                    self.pane = Pane::Add;
                    Outcome::Stay
                }
                KeyCode::Char('d' | 'D') => {
                    self.error = None;
                    let i = self.sel();
                    if self
                        .file()
                        .lines
                        .get(i)
                        .is_some_and(|l| !l.trim().is_empty())
                    {
                        self.pane = Pane::ConfirmRemove(i);
                    }
                    Outcome::Stay
                }
                // The file in the user's own editor, with the terminal
                // handed over. The panel closes: it would show the old text.
                KeyCode::Char('e' | 'E') => {
                    Outcome::CloseAct(Action::EditCatalog(self.file().path.clone()))
                }
                _ => Outcome::Stay,
            },
            Pane::Add => match key.code {
                KeyCode::Esc => {
                    self.back(view);
                    Outcome::Stay
                }
                KeyCode::Enter => {
                    let typed = view.composer.text().to_string();
                    match self.add(&typed) {
                        Ok(said) => {
                            self.error = None;
                            self.back(view);
                            view.system(said);
                        }
                        Err(e) => self.error = Some(e),
                    }
                    Outcome::Stay
                }
                // Typing goes to the input row.
                _ => {
                    super::edit_field(&mut view.composer, key);
                    self.error = None;
                    Outcome::Stay
                }
            },
            Pane::ConfirmRemove(i) => match key.code {
                KeyCode::Char('y' | 'Y') => {
                    match self.remove(i) {
                        Ok(said) => {
                            self.error = None;
                            view.system(said);
                        }
                        Err(e) => self.error = Some(e),
                    }
                    self.pane = Pane::Home;
                    Outcome::Stay
                }
                KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                    self.pane = Pane::Home;
                    Outcome::Stay
                }
                _ => Outcome::Stay,
            },
        }
    }

    fn box_clone(&self) -> Box<dyn Panel> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use ryter_core::sandbox::SandboxProfile;
    use tempfile::TempDir;

    struct Fixture {
        home: TempDir,
        project: TempDir,
        view: View,
    }

    fn fixture() -> Fixture {
        let home = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        let mut view = View::new("c".into(), "m".into(), project.path().display().to_string());
        view.project_root = Some(project.path().display().to_string());
        Fixture {
            home,
            project,
            view,
        }
    }

    fn open(f: &Fixture) -> Rules {
        Rules::new(
            &f.view,
            &PanelEnv {
                home: f.home.path().to_path_buf(),
                cwd: f.project.path().to_path_buf(),
                trusted: false,
                sandbox: SandboxProfile::Off,
            },
        )
    }

    fn rows(p: &Rules, f: &Fixture) -> Vec<String> {
        p.render(&f.view, 60, 14, Theme::truecolor_dark())
            .lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    fn press(p: &mut Rules, f: &mut Fixture, code: KeyCode) -> Outcome {
        p.key(KeyEvent::new(code, KeyModifiers::NONE), &mut f.view)
    }

    /// Type a rule into the panel's input and press Enter.
    fn add(p: &mut Rules, f: &mut Fixture, text: &str) {
        press(p, f, KeyCode::Char('a'));
        assert_eq!(p.input(&f.view).as_deref(), Some("new rule"));
        // Typed, key by key, as a person types it.
        for c in text.chars() {
            press(p, f, KeyCode::Char(c));
        }
        press(p, f, KeyCode::Enter);
    }

    const EVERY: &str = "- Answer in British spelling.\n- Ask before installing packages.\n\n## Reviews\n- Post a verdict comment on every PR, with the commit reviewed and the checks run.\n";

    /// The panel as it was drawn for the user: two tabs with the open one in
    /// capitals, the file and how many rules it holds, then its lines, with
    /// a long rule wrapped under its own text.
    #[test]
    fn both_files_are_read_one_at_a_time() {
        let mut f = fixture();
        ryter_core::rules::save(f.home.path(), EVERY).unwrap();
        std::fs::write(
            f.project.path().join("RYTER.md"),
            "- Run cargo test before saying a change is done.\n",
        )
        .unwrap();
        let mut p = open(&f);
        let shown = rows(&p, &f);
        assert_eq!(shown[0], "  EVERY PROJECT    this project");
        assert!(
            shown[1].ends_with("RYTER.md · 3 rules") && shown[1].starts_with("  "),
            "{shown:?}"
        );
        assert_eq!(shown[2], "");
        assert_eq!(shown[3], "› - Answer in British spelling.");
        assert_eq!(shown[4], "  - Ask before installing packages.");
        assert_eq!(shown[6], "  ## Reviews");
        assert_eq!(
            &shown[7..9],
            [
                "  - Post a verdict comment on every PR, with the commit",
                "    reviewed and the checks run."
            ]
        );
        assert_eq!(
            p.legend(&f.view),
            "tab file · a add · d remove · e editor · esc close"
        );
        // Tab: the project's.
        press(&mut p, &mut f, KeyCode::Tab);
        let shown = rows(&p, &f);
        assert_eq!(shown[0], "  every project    THIS PROJECT");
        assert_eq!(shown[1], "  ./RYTER.md · 1 rule");
        assert_eq!(
            shown[3],
            "› - Run cargo test before saying a change is done."
        );
        // Down moves over the blank line; the selection is per file.
        press(&mut p, &mut f, KeyCode::Tab);
        press(&mut p, &mut f, KeyCode::Down);
        press(&mut p, &mut f, KeyCode::Down);
        assert_eq!(rows(&p, &f)[6], "› ## Reviews");
    }

    /// `a` adds a rule under the selected line, as a bullet, and saves the
    /// file. `d` removes the selected line once the user has said yes.
    #[test]
    fn a_rule_is_added_and_removed_by_hand() {
        let mut f = fixture();
        ryter_core::rules::save(f.home.path(), EVERY).unwrap();
        let mut p = open(&f);
        add(&mut p, &mut f, "Keep replies short.");
        assert_eq!(p.pane, Pane::Home);
        assert_eq!(
            ryter_core::rules::read(f.home.path()).unwrap(),
            EVERY.replacen("- Ask before", "- Keep replies short.\n- Ask before", 1)
        );
        assert_eq!(rows(&p, &f)[4], "› - Keep replies short.");
        // A heading is kept as typed, and nothing typed adds nothing.
        add(&mut p, &mut f, "## Tone");
        assert!(
            ryter_core::rules::read(f.home.path())
                .unwrap()
                .contains("\n## Tone\n")
        );
        add(&mut p, &mut f, "   ");
        assert_eq!(p.pane, Pane::Add, "still asking");
        assert!(rows(&p, &f).iter().any(|r| r.contains("type the rule")));
        press(&mut p, &mut f, KeyCode::Esc);
        // Remove asks first; `n` keeps it, `y` removes it.
        let before = ryter_core::rules::read(f.home.path()).unwrap();
        press(&mut p, &mut f, KeyCode::Char('d'));
        assert!(
            rows(&p, &f)
                .iter()
                .any(|r| r.contains("remove \"## Tone\"?"))
        );
        assert_eq!(p.legend(&f.view), "y remove · n keep");
        press(&mut p, &mut f, KeyCode::Char('n'));
        assert_eq!(ryter_core::rules::read(f.home.path()).unwrap(), before);
        press(&mut p, &mut f, KeyCode::Char('d'));
        press(&mut p, &mut f, KeyCode::Char('y'));
        assert!(
            !ryter_core::rules::read(f.home.path())
                .unwrap()
                .contains("## Tone")
        );
    }

    /// With no file yet the panel says so, and the first rule creates it:
    /// the every-project file in Ryter's folder, the project's as
    /// `RYTER.md`. A project that keeps `AGENTS.md` has its rules read from
    /// and added to that.
    #[test]
    fn the_first_rule_creates_the_file() {
        let mut f = fixture();
        let mut p = open(&f);
        let shown = rows(&p, &f);
        assert!(
            shown[1].ends_with("RYTER.md · not created yet"),
            "{shown:?}"
        );
        assert!(
            shown.iter().any(|r| r.contains("No rules yet")),
            "{shown:?}"
        );
        add(&mut p, &mut f, "Be brief.");
        assert_eq!(
            ryter_core::rules::read(f.home.path()).as_deref(),
            Some("- Be brief.\n")
        );
        press(&mut p, &mut f, KeyCode::Tab);
        assert_eq!(rows(&p, &f)[1], "  ./RYTER.md · not created yet");
        add(&mut p, &mut f, "Never edit generated/.");
        assert_eq!(
            std::fs::read_to_string(f.project.path().join("RYTER.md")).unwrap(),
            "- Never edit generated/.\n"
        );
        // AGENTS.md, when that is the file the project has.
        let mut f = fixture();
        std::fs::write(f.project.path().join("AGENTS.md"), "- Use tabs.\n").unwrap();
        let mut p = open(&f);
        press(&mut p, &mut f, KeyCode::Tab);
        assert_eq!(rows(&p, &f)[1], "  ./AGENTS.md · 1 rule");
        add(&mut p, &mut f, "Wrap at 100.");
        assert_eq!(
            std::fs::read_to_string(f.project.path().join("AGENTS.md")).unwrap(),
            "- Use tabs.\n- Wrap at 100.\n"
        );
        assert!(!f.project.path().join("RYTER.md").exists());
    }

    /// `e` hands the file on screen to the user's editor.
    #[test]
    fn e_opens_the_file_on_screen_in_the_editor() {
        let mut f = fixture();
        let mut p = open(&f);
        let Outcome::CloseAct(Action::EditCatalog(path)) =
            press(&mut p, &mut f, KeyCode::Char('e'))
        else {
            panic!("e did not open the editor");
        };
        assert_eq!(path, ryter_core::rules::path(f.home.path()));
        let mut p = open(&f);
        press(&mut p, &mut f, KeyCode::Tab);
        let Outcome::CloseAct(Action::EditCatalog(path)) =
            press(&mut p, &mut f, KeyCode::Char('e'))
        else {
            panic!("e did not open the editor");
        };
        assert_eq!(path, f.project.path().join("RYTER.md"));
    }

    /// A file past what Ryter loads isn't grown from here.
    #[test]
    fn the_every_project_file_stays_within_what_ryter_loads() {
        let mut f = fixture();
        let line = "- a rule that is fairly long, to fill the file up quickly\n";
        let nearly = line.repeat(ryter_core::rules::MAX_BYTES / line.len());
        ryter_core::rules::save(f.home.path(), &nearly).unwrap();
        let mut p = open(&f);
        add(&mut p, &mut f, &"x".repeat(200));
        assert!(
            p.error.as_deref().is_some_and(|e| e.contains("32 KB")),
            "{:?}",
            p.error
        );
        assert_eq!(ryter_core::rules::read(f.home.path()).unwrap(), nearly);
    }
}
