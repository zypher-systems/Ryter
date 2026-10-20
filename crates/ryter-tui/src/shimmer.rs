//! A band of light sliding across a line of text: the sign that a turn is
//! alive, in place of a spinner.
//!
//! The band is a raised cosine five cells either side of its centre,
//! crossing the text and the padding beyond it once every two seconds, so
//! the text is plain for part of each pass. The band is drawn by blending
//! the text's colour toward `band` where both are RGB; indexed colours
//! take the band colour and bold at the centre; without colour the text is
//! plain. The phase comes from the clock the caller already keeps, so a
//! golden frame at a fixed time is the same frame every run.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use crate::theme::ColorMode;

/// Milliseconds for one pass of the band.
const PERIOD_MS: u64 = 2000;
/// Cells of silence before the text and after it, so the band leaves.
const PADDING: usize = 10;
/// Cells from the band's centre to its edge.
const HALF: usize = 5;

/// `text` with the band at the phase `now_ms` gives, on `base`.
pub fn spans(
    text: &str,
    base: Style,
    band: Color,
    now_ms: u64,
    mode: ColorMode,
) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    if mode == ColorMode::Mono {
        return vec![Span::styled(text.to_string(), base)];
    }
    let phase = (now_ms % PERIOD_MS) as f64 / PERIOD_MS as f64;
    let period = chars.len() + PADDING * 2;
    let pos = (phase * period as f64) as isize;
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut run_style: Option<Style> = None;
    for (i, ch) in chars.iter().enumerate() {
        let dist = ((i + PADDING) as isize - pos).unsigned_abs();
        let intensity = if dist <= HALF {
            0.5 * (1.0 + (std::f64::consts::PI * dist as f64 / HALF as f64).cos())
        } else {
            0.0
        };
        let style = lit(base, band, intensity);
        if run_style != Some(style) {
            if let Some(s) = run_style.take() {
                out.push(Span::styled(std::mem::take(&mut run), s));
            }
            run_style = Some(style);
        }
        run.push(*ch);
    }
    if let Some(s) = run_style {
        out.push(Span::styled(run, s));
    }
    out
}

/// The style of one cell under the band at `intensity` (0 outside it, 1 at
/// its centre).
fn lit(base: Style, band: Color, intensity: f64) -> Style {
    if intensity <= 0.0 {
        return base;
    }
    match (base.fg, band) {
        (Some(Color::Rgb(r, g, b)), Color::Rgb(br, bg, bb)) => {
            let mix = |a: u8, b: u8| -> u8 {
                (f64::from(a) + (f64::from(b) - f64::from(a)) * intensity * 0.9).round() as u8
            };
            base.fg(Color::Rgb(mix(r, br), mix(g, bg), mix(b, bb)))
                .add_modifier(Modifier::BOLD)
        }
        _ if intensity >= 0.6 => base.fg(band).add_modifier(Modifier::BOLD),
        _ if intensity >= 0.2 => base.add_modifier(Modifier::BOLD),
        _ => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(spans: &[Span<'_>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn the_text_is_whole_at_every_phase() {
        let base = Style::default().fg(Color::Rgb(100, 100, 100));
        for ms in (0..PERIOD_MS).step_by(37) {
            let s = spans(
                "running · 0:04",
                base,
                Color::Rgb(230, 230, 230),
                ms,
                ColorMode::TrueColor,
            );
            assert_eq!(text_of(&s), "running · 0:04", "at {ms} ms");
        }
    }

    #[test]
    fn the_band_crosses_the_text_and_leaves_it() {
        let base = Style::default().fg(Color::Rgb(100, 100, 100));
        let band = Color::Rgb(230, 230, 230);
        let lit_at = |ms: u64| {
            spans("thinking", base, band, ms, ColorMode::TrueColor)
                .iter()
                .any(|s| s.style.fg != base.fg)
        };
        // Phase 0: the band sits in the left padding, ten cells away.
        assert!(!lit_at(0));
        // A third of the way through, the band is over the word.
        assert!(lit_at(PERIOD_MS / 3));
        assert!(!lit_at(PERIOD_MS - 1));
    }

    #[test]
    fn indexed_colours_bold_the_centre_and_mono_is_plain() {
        let base = Style::default().fg(Color::Gray);
        let s = spans(
            "thinking",
            base,
            Color::White,
            PERIOD_MS / 3,
            ColorMode::Ansi16,
        );
        assert!(
            s.iter()
                .any(|x| x.style.add_modifier.contains(Modifier::BOLD))
        );
        assert!(
            s.iter()
                .all(|x| !matches!(x.style.fg, Some(Color::Rgb(..))))
        );
        let m = spans(
            "thinking",
            base,
            Color::White,
            PERIOD_MS / 3,
            ColorMode::Mono,
        );
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].style, base);
    }
}
