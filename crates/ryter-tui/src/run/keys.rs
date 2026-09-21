//! Keyboard dispatch: `KEYMAP` contexts → view mutations / `Action`s (§12).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::{Action, PanelId};
use crate::composer::Mode as ComposerMode;
use crate::keymap::{self, Ctx, KeyAction};
use crate::palette;
use crate::panel;
use crate::view::View;

/// Window for the second `Ctrl+C` (`R-COMP-14`).
pub const QUIT_ARM_MS: u64 = 2000;

/// Route one key press.
pub fn handle(view: &mut View, key: KeyEvent) -> Action {
    // Global bindings win everywhere, except `Esc` which panels interpret
    // themselves (wizard step back, confirm cancel), and `Ctrl+D`/`Ctrl+P`
    // which are composer-only conveniences.
    if let Some(a) = keymap::lookup(Ctx::Global, key) {
        match a {
            KeyAction::CtrlC => return ctrl_c(view),
            KeyAction::Redraw => return Action::Redraw,
            KeyAction::ToggleMouse => return Action::ToggleMouse,
            KeyAction::TogglePanel => {
                view.panel_visible = !view.panel_visible;
                return Action::None;
            }
            KeyAction::ToggleReasoning => {
                view.activity.toggle();
                return Action::None;
            }
            KeyAction::Help if !view.panels.has_modal() => {
                if view.panels.top().is_some_and(|p| p.kind() == "help") {
                    view.panels.pop();
                    return Action::None;
                }
                return Action::OpenPanel(PanelId::Help);
            }
            KeyAction::QuitIfEmpty if view.panels.is_empty() && view.composer.is_empty() => {
                return Action::Quit;
            }
            KeyAction::OpenPalette if view.panels.is_empty() => {
                if matches!(view.composer.mode, ComposerMode::Normal) {
                    palette::open(view);
                }
                return Action::None;
            }
            KeyAction::Back if view.panels.is_empty() => return esc(view),
            _ => {}
        }
    }

    // A focused panel (or modal) owns everything else.
    if !view.panels.is_empty() {
        let a = panel::handle_key(view, key);
        panel::sync_composer(view);
        return a;
    }

    // Secret capture (`R-COMP-07`): only Enter / Esc / edit keys.
    if matches!(view.composer.mode, ComposerMode::Secret { .. }) {
        return secret_key(view, key);
    }

    // Palette navigation (`R-PAL-*`).
    if view.palette.is_some() {
        if let Some(a) = keymap::lookup(Ctx::Palette, key) {
            match a {
                KeyAction::Up => {
                    palette::step(view, -1);
                    return Action::None;
                }
                KeyAction::Down => {
                    palette::step(view, 1);
                    return Action::None;
                }
                KeyAction::Complete => {
                    palette::complete(view);
                    return Action::None;
                }
                KeyAction::Activate => return palette::run(view),
                KeyAction::OpenRow => return palette::open_row(view),
                KeyAction::Back => {
                    palette::close(view);
                    return Action::None;
                }
                _ => {}
            }
        }
    }

    // Chat scroll (`R-SCROLL-10`).
    if let Some(a) = keymap::lookup(Ctx::Scroll, key) {
        let busy = view.busy;
        match a {
            KeyAction::PageUp => view.scroll.page_up(busy),
            KeyAction::PageDown => view.scroll.page_down(busy),
            KeyAction::LineUp => view.scroll.scroll_by(-1, busy),
            KeyAction::LineDown => view.scroll.scroll_by(1, busy),
            KeyAction::Top => view.scroll.to_top(busy),
            KeyAction::Bottom => view.scroll.to_bottom(),
            KeyAction::PrevTurn => view.scroll.prev_turn(busy),
            KeyAction::NextTurn => view.scroll.next_turn(busy),
            KeyAction::ReasoningUp => reasoning_scroll(view, -1),
            KeyAction::ReasoningDown => reasoning_scroll(view, 1),
            _ => {}
        }
        return Action::None;
    }

    composer_key(view, key)
}

/// `Ctrl+C`: clear composer → close panels → cancel turn → quit (double-press).
fn ctrl_c(view: &mut View) -> Action {
    if !view.composer.is_empty() {
        view.composer.end_special();
        view.palette = None;
        view.quit_armed_until = None;
        return Action::None;
    }
    if !view.panels.is_empty() {
        view.panels.clear();
        panel::sync_composer(view);
        return Action::None;
    }
    if view.busy {
        return Action::Cancel;
    }
    if view.quit_armed_until.is_some_and(|t| view.now_ms <= t) {
        return Action::Quit;
    }
    view.quit_armed_until = Some(view.now_ms + QUIT_ARM_MS);
    Action::None
}

/// `Esc` precedence (`R-COMP-15`): panel → palette → handoff → queued →
/// in-flight turn → clear composer.
fn esc(view: &mut View) -> Action {
    if view.palette.is_some() {
        palette::close(view);
        return Action::None;
    }
    if let ComposerMode::Secret { .. } = view.composer.mode {
        view.composer.end_special();
        view.system("cancelled");
        return Action::None;
    }
    if view.queued_prompt.take().is_some() {
        view.system("queued message dropped");
        return Action::None;
    }
    if view.busy && !view.cancelling {
        return Action::Cancel;
    }
    view.composer.clear();
    view.history.reset();
    Action::None
}

fn secret_key(view: &mut View, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Enter => match view.composer.take_secret() {
            Some((name, secret)) if !secret.trim().is_empty() => Action::SetKey {
                name,
                key: secret.trim().to_string(),
            },
            Some((name, _)) => {
                view.warn(format!("empty key for {name} — nothing saved"));
                Action::None
            }
            None => Action::None,
        },
        KeyCode::Backspace => {
            view.composer.backspace();
            Action::None
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            view.composer.insert_char(c);
            Action::None
        }
        _ => Action::None,
    }
}

fn reasoning_scroll(view: &mut View, dir: i32) {
    let cur = view.activity.scroll.unwrap_or(usize::MAX);
    let next = if dir < 0 {
        cur.saturating_sub(4)
    } else {
        cur.saturating_add(4)
    };
    // `None` means "follow the tail"; any explicit value detaches.
    view.activity.scroll = Some(next);
}

/// Composer editing and submit (`R-COMP-09..13`).
fn composer_key(view: &mut View, key: KeyEvent) -> Action {
    // Tab switches hats in solo mode. In crew mode there is one speaker,
    // the lead; say how to get back rather than doing nothing.
    if matches!(view.composer.mode, crate::composer::Mode::Normal)
        && matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
    {
        if view.crew_mode() {
            view.system("crew mode · /solo to go back to build, plan, and review");
            return Action::None;
        }
        return Action::SetMode(if key.code == KeyCode::BackTab {
            view.mode.prev_hat()
        } else {
            view.mode.next_hat()
        });
    }
    let action = keymap::lookup(Ctx::Composer, key);
    let mut edited = true;
    let result = match action {
        Some(KeyAction::Send) => return submit(view),
        Some(KeyAction::Newline) => {
            view.composer.newline();
            Action::None
        }
        Some(KeyAction::Left) => {
            view.composer.left();
            Action::None
        }
        Some(KeyAction::Right) => {
            view.composer.right();
            Action::None
        }
        Some(KeyAction::Home) => {
            view.composer.home();
            Action::None
        }
        Some(KeyAction::End) => {
            view.composer.end();
            Action::None
        }
        Some(KeyAction::WordLeft) => {
            view.composer.word_left();
            Action::None
        }
        Some(KeyAction::WordRight) => {
            view.composer.word_right();
            Action::None
        }
        Some(KeyAction::Up) => {
            if view.composer.is_multiline() && view.composer.up() {
                // moved within the buffer
            } else if view.composer.is_empty() || view.history.browsing() {
                let cur = view.composer.text().to_string();
                if let Some(prev) = view.history.back(&cur).map(str::to_string) {
                    view.composer.set_text(&prev);
                }
            }
            Action::None
        }
        Some(KeyAction::Down) => {
            if view.composer.is_multiline() && view.composer.down() {
                // moved within the buffer
            } else if view.history.browsing() {
                let next = view.history.forward().unwrap_or_default();
                view.composer.set_text(&next);
            }
            Action::None
        }
        Some(KeyAction::KillWord) => {
            view.composer.kill_word_back();
            Action::None
        }
        Some(KeyAction::KillToStart) => {
            view.composer.kill_to_line_start();
            Action::None
        }
        Some(KeyAction::KillToEnd) => {
            view.composer.kill_to_line_end();
            Action::None
        }
        Some(KeyAction::Backspace) => {
            view.composer.backspace();
            Action::None
        }
        Some(KeyAction::Delete) => {
            view.composer.delete();
            Action::None
        }
        _ => match key.code {
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                view.composer.insert_char(c);
                Action::None
            }
            KeyCode::Tab => {
                view.composer.insert_char('\t');
                Action::None
            }
            _ => {
                edited = false;
                Action::None
            }
        },
    };
    if edited {
        view.quit_armed_until = None;
        palette::refresh(view);
    }
    result
}

/// `Enter` in the composer.
fn submit(view: &mut View) -> Action {
    if view.palette.is_some() {
        return palette::run(view);
    }
    let text = view.composer.take();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        view.composer.rejected = true;
        return Action::None;
    }
    view.history.reset();
    if trimmed.starts_with('/') {
        let shown = trimmed.to_string();
        let a = palette::registry::run_command(view, &shown);
        return a;
    }
    let shown = trimmed.to_string();
    view.submit_user(shown, text.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ryter_core::Phase;

    fn v() -> View {
        let mut v = View::new(Phase::Build, "c".into(), "m".into(), "p".into());
        v.now_ms = 10_000;
        v
    }

    fn press(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn ctrl_c_clears_then_cancels_then_arms_quit() {
        let mut view = v();
        view.composer.insert_char('x');
        assert_eq!(
            handle(&mut view, press(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::None
        );
        assert!(view.composer.is_empty());
        let _ = view.submit_user("go".into(), "go".into());
        assert_eq!(
            handle(&mut view, press(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Cancel
        );
        view.busy = false;
        assert_eq!(
            handle(&mut view, press(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::None
        );
        assert!(view.quit_armed_until.is_some());
        assert_eq!(
            handle(&mut view, press(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit
        );
        // Expired arm does not quit.
        let mut view = v();
        view.quit_armed_until = Some(1);
        assert_eq!(
            handle(&mut view, press(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::None
        );
    }

    #[test]
    fn enter_submits_and_shift_enter_inserts_newline() {
        let mut view = v();
        for c in "hi".chars() {
            handle(&mut view, press(KeyCode::Char(c), KeyModifiers::NONE));
        }
        handle(&mut view, press(KeyCode::Enter, KeyModifiers::SHIFT));
        assert_eq!(view.composer.text(), "hi\n");
        handle(&mut view, press(KeyCode::Char('!'), KeyModifiers::NONE));
        let a = handle(&mut view, press(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(a, Action::Submit("hi\n!".into()));
        assert!(view.composer.is_empty());
    }

    #[test]
    fn slash_opens_palette_and_esc_precedence() {
        let mut view = v();
        handle(&mut view, press(KeyCode::Char('/'), KeyModifiers::NONE));
        assert!(view.palette.is_some());
        handle(&mut view, press(KeyCode::Esc, KeyModifiers::NONE));
        assert!(view.palette.is_none());
        assert_eq!(view.composer.text(), "/");
        handle(&mut view, press(KeyCode::Esc, KeyModifiers::NONE));
        assert!(view.composer.is_empty());
        // Esc during a turn cancels.
        let _ = view.submit_user("go".into(), "go".into());
        assert_eq!(
            handle(&mut view, press(KeyCode::Esc, KeyModifiers::NONE)),
            Action::Cancel
        );
    }

    #[test]
    fn history_recall_when_empty_only() {
        let mut view = v();
        let _ = view.submit_user("first".into(), "first".into());
        view.busy = false;
        handle(&mut view, press(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(view.composer.text(), "first");
        handle(&mut view, press(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(view.composer.text(), "");
        // Non-empty single-line composer: Up does nothing to the text.
        view.composer.set_text("draft");
        view.history.reset();
        handle(&mut view, press(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(view.composer.text(), "draft");
    }

    #[test]
    fn scroll_keys_detach_and_end_refollows() {
        let mut view = v();
        view.scroll.doc_rows.set(200);
        view.scroll.viewport.set(20);
        view.scroll.effective.set(180);
        handle(&mut view, press(KeyCode::PageUp, KeyModifiers::NONE));
        assert!(!view.scroll.follow);
        handle(&mut view, press(KeyCode::End, KeyModifiers::CONTROL));
        assert!(view.scroll.follow);
    }

    #[test]
    fn secret_mode_enter_yields_set_key() {
        let mut view = v();
        view.composer.begin_secret("spacexai".into());
        for c in "sk-1".chars() {
            handle(&mut view, press(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert_eq!(view.composer.text(), "");
        let a = handle(&mut view, press(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            a,
            Action::SetKey {
                name: "spacexai".into(),
                key: "sk-1".into()
            }
        );
    }
}
