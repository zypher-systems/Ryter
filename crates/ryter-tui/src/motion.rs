//! Motion on the workspace, drawn after the frame: a card fades in from
//! the hat's colour, a hat switch sweeps the chip and the hat's row in the
//! new colour, the welcome gathers itself on a fresh session.
//!
//! The effects run on the buffer the frame already holds, so nothing that
//! draws the screen knows about them, and a golden frame is a frame with
//! none. The run loop owns the state, observes each frame's marks for the
//! transitions, and keeps drawing while an effect runs. Colour effects
//! need RGB cells and run in truecolor alone; the welcome's gathering is
//! glyphs and runs wherever there is colour.

use ratatui::Frame;
use ratatui::layout::Rect;
use ryter_core::Role;
use tachyonfx::{Effect, EffectRenderer, Interpolation, fx};

use crate::draw::Hit;
use crate::theme::{ColorMode, Theme};
use crate::view::View;

/// Milliseconds a card takes to fade in.
const CARD_MS: u32 = 240;
/// Milliseconds a hat switch takes to sweep.
const SWEEP_MS: u32 = 320;
/// Milliseconds the welcome takes to gather.
const WELCOME_MS: u32 = 600;

/// The effects in flight and what the last frame showed.
#[derive(Default)]
pub struct Motion {
    running: Vec<Effect>,
    last_ms: Option<u64>,
    seen_mode: Option<Role>,
    seen_covers: Vec<Rect>,
    welcomed: bool,
}

impl Motion {
    /// Something is still moving: the loop keeps drawing at frame rate.
    pub fn active(&self) -> bool {
        !self.running.is_empty()
    }

    /// Start the effects this frame's transitions call for, then run every
    /// effect for the time since the last frame.
    pub fn render(&mut self, frame: &mut Frame, hit: &Hit, view: &View, theme: Theme, now_ms: u64) {
        let colour = theme.mode == ColorMode::TrueColor;
        let any = theme.mode != ColorMode::Mono;
        let accent = theme.mode(view.mode);
        // The welcome gathers when it appears: on the first frame of a
        // fresh session, and again after `/new` in the same run, once the
        // transcript between has taken it off the screen.
        match hit.welcome {
            Some(area) if any && !self.welcomed => {
                self.welcomed = true;
                self.running
                    .push(fx::coalesce((WELCOME_MS, Interpolation::QuadOut)).with_area(area));
            }
            Some(_) => {}
            None => self.welcomed = false,
        }
        // A cover at a place the last frame had none: a card has opened.
        // The same card redrawn, or one that moved a row as the chat
        // grew, is not a new one.
        if colour {
            for area in hit
                .covers
                .iter()
                .filter(|a| !self.seen_covers.iter().any(|s| s.y == a.y || s.x == a.x))
            {
                self.running.push(
                    fx::fade_from_fg(accent, (CARD_MS, Interpolation::QuadOut)).with_area(*area),
                );
            }
        }
        self.seen_covers.clone_from(&hit.covers);
        if colour && self.seen_mode.is_some_and(|m| m != view.mode) {
            for area in [hit.chip, hit.hat_row].into_iter().flatten() {
                self.running.push(
                    fx::sweep_in(
                        tachyonfx::Motion::LeftToRight,
                        6,
                        0,
                        accent,
                        (SWEEP_MS, Interpolation::QuadOut),
                    )
                    .with_area(area),
                );
            }
        }
        self.seen_mode = Some(view.mode);
        let tick = tachyonfx::Duration::from_millis(
            u32::try_from(now_ms.saturating_sub(self.last_ms.unwrap_or(now_ms)))
                .unwrap_or(u32::MAX),
        );
        self.last_ms = Some(now_ms);
        // An effect whose place has left the screen (the terminal shrank
        // under a card) is dropped, or it would never finish and the loop
        // would redraw at frame rate for good.
        let full = frame.area();
        self.running
            .retain(|e| !e.area().unwrap_or(full).intersection(full).is_empty());
        for effect in &mut self.running {
            let area = effect.area().unwrap_or(full).intersection(full);
            frame.render_effect(effect, area, tick);
        }
        self.running.retain(|e| !e.done());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    fn hit(cover: Option<Rect>, welcome: Option<Rect>) -> Hit {
        Hit {
            covers: cover.into_iter().collect(),
            welcome,
            ..Hit::default()
        }
    }

    fn view() -> View {
        View::new("x".into(), "m".into(), "p".into())
    }

    #[test]
    fn a_new_card_fades_in_and_the_fade_ends() {
        let theme = Theme::truecolor_dark();
        let mut m = Motion::default();
        let v = view();
        let backend = TestBackend::new(40, 10);
        let mut term = Terminal::new(backend).unwrap();
        let card = Rect::new(2, 2, 20, 4);
        // Frame one: no card. Frame two: a card appears.
        term.draw(|f| m.render(f, &hit(None, None), &v, theme, 0))
            .unwrap();
        assert!(!m.active());
        term.draw(|f| {
            f.render_widget(
                ratatui::widgets::Paragraph::new("allow?")
                    .style(ratatui::style::Style::default().fg(Color::Rgb(200, 200, 200))),
                card,
            );
            m.render(f, &hit(Some(card), None), &v, theme, 16);
        })
        .unwrap();
        assert!(m.active(), "the fade runs");
        let early = term.backend().buffer()[(2, 2)].fg;
        assert_ne!(
            early,
            Color::Rgb(200, 200, 200),
            "the first frame is tinted"
        );
        term.draw(|f| m.render(f, &hit(Some(card), None), &v, theme, 16 + 1000))
            .unwrap();
        assert!(!m.active(), "a second later it is done");
    }

    #[test]
    fn nothing_moves_without_colour() {
        let mut theme = Theme::truecolor_dark();
        theme.mode = ColorMode::Mono;
        let mut m = Motion::default();
        let v = view();
        let backend = TestBackend::new(40, 10);
        let mut term = Terminal::new(backend).unwrap();
        let card = Rect::new(2, 2, 20, 4);
        term.draw(|f| m.render(f, &hit(None, Some(card)), &v, theme, 0))
            .unwrap();
        term.draw(|f| m.render(f, &hit(Some(card), Some(card)), &v, theme, 16))
            .unwrap();
        assert!(!m.active());
    }

    #[test]
    fn an_effect_off_the_screen_is_dropped() {
        let theme = Theme::truecolor_dark();
        let mut m = Motion::default();
        let v = view();
        let mut term = Terminal::new(TestBackend::new(40, 10)).unwrap();
        let card = Rect::new(2, 8, 20, 2);
        term.draw(|f| m.render(f, &hit(None, None), &v, theme, 0))
            .unwrap();
        term.draw(|f| m.render(f, &hit(Some(card), None), &v, theme, 16))
            .unwrap();
        assert!(m.active());
        // The terminal shrinks under the card: the fade must not run on.
        let mut small = Terminal::new(TestBackend::new(40, 6)).unwrap();
        small
            .draw(|f| m.render(f, &hit(Some(card), None), &v, theme, 32))
            .unwrap();
        assert!(!m.active(), "an effect with no place on the screen is gone");
    }

    #[test]
    fn the_welcome_gathers_again_after_it_has_been_away() {
        let theme = Theme::truecolor_dark();
        let mut m = Motion::default();
        let v = view();
        let mut term = Terminal::new(TestBackend::new(40, 10)).unwrap();
        let text = Rect::new(2, 2, 20, 3);
        // A first frame without a welcome (a resumed session) does not
        // spend the gathering.
        term.draw(|f| m.render(f, &hit(None, None), &v, theme, 0))
            .unwrap();
        term.draw(|f| m.render(f, &hit(None, Some(text)), &v, theme, 16))
            .unwrap();
        assert!(m.active(), "the welcome gathers when it appears");
        term.draw(|f| m.render(f, &hit(None, Some(text)), &v, theme, 2000))
            .unwrap();
        assert!(!m.active());
        term.draw(|f| m.render(f, &hit(None, Some(text)), &v, theme, 2016))
            .unwrap();
        assert!(!m.active(), "the same welcome does not gather twice");
        // Away (a turn), then `/new`: it gathers again.
        term.draw(|f| m.render(f, &hit(None, None), &v, theme, 2032))
            .unwrap();
        term.draw(|f| m.render(f, &hit(None, Some(text)), &v, theme, 2048))
            .unwrap();
        assert!(m.active());
    }

    #[test]
    fn a_hat_switch_sweeps_the_chip() {
        let theme = Theme::truecolor_dark();
        let mut m = Motion::default();
        let mut v = view();
        let backend = TestBackend::new(40, 10);
        let mut term = Terminal::new(backend).unwrap();
        let chip = Rect::new(1, 8, 8, 1);
        let marks = Hit {
            chip: Some(chip),
            ..Hit::default()
        };
        term.draw(|f| m.render(f, &marks, &v, theme, 0)).unwrap();
        v.mode = Role::Plan;
        term.draw(|f| m.render(f, &marks, &v, theme, 16)).unwrap();
        assert!(m.active(), "the sweep runs after a switch");
        term.draw(|f| m.render(f, &marks, &v, theme, 2000)).unwrap();
        assert!(!m.active());
    }
}
