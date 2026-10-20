//! A ring of braille dots for a fraction: the context window as a dial.
//!
//! A cell holds two dots across and four down, each dot as tall as it is
//! wide on a screen whose cells are twice as tall as wide, so a circle in
//! dot space is a circle on the screen. The ring is ten cells wide and
//! five tall: twenty dots each way, radius nine. The part from twelve
//! o'clock clockwise to the fraction is in `fill`; the rest in `track`. A
//! cell with a dot of each takes `fill`.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// Cells across.
pub const WIDTH: usize = 10;
/// Cells down.
pub const HEIGHT: usize = 5;

/// The ring's rows, `HEIGHT` of them, each `WIDTH` cells, on `bg`.
pub fn rows(frac: f64, fill: Color, track: Color, bg: Color) -> Vec<Line<'static>> {
    let frac = if frac.is_finite() {
        frac.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (dw, dh) = (WIDTH * 2, HEIGHT * 4);
    // 0: no dot, 1: track, 2: fill.
    let mut dots = vec![0u8; dw * dh];
    let (cx, cy, r) = (dw as f64 / 2.0 - 0.5, dh as f64 / 2.0 - 0.5, 9.0f64);
    let steps = 720;
    for i in 0..steps {
        let t = i as f64 / steps as f64;
        let a = -std::f64::consts::FRAC_PI_2 + t * std::f64::consts::TAU;
        let x = (cx + r * a.cos()).round();
        let y = (cy + r * a.sin()).round();
        if x < 0.0 || y < 0.0 || x >= dw as f64 || y >= dh as f64 {
            continue;
        }
        let kind = if frac > 0.0 && t <= frac { 2 } else { 1 };
        let cell = &mut dots[y as usize * dw + x as usize];
        *cell = (*cell).max(kind);
    }
    let mut lines = Vec::with_capacity(HEIGHT);
    for row in 0..HEIGHT {
        let mut spans: Vec<Span<'static>> = Vec::new();
        for col in 0..WIDTH {
            let mut bits = 0u32;
            let mut kind = 0u8;
            for (dx, dy, bit) in [
                (0, 0, 0x01),
                (0, 1, 0x02),
                (0, 2, 0x04),
                (1, 0, 0x08),
                (1, 1, 0x10),
                (1, 2, 0x20),
                (0, 3, 0x40),
                (1, 3, 0x80),
            ] {
                let d = dots[(row * 4 + dy) * dw + col * 2 + dx];
                if d > 0 {
                    bits |= bit;
                    kind = kind.max(d);
                }
            }
            let ch = if bits == 0 {
                ' '
            } else {
                char::from_u32(0x2800 + bits).unwrap_or(' ')
            };
            let color = if kind == 2 { fill } else { track };
            spans.push(Span::styled(
                ch.to_string(),
                Style::default().fg(color).bg(bg),
            ));
        }
        lines.push(Line::from(spans));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn the_ring_is_five_rows_of_ten_with_a_hollow_middle() {
        let rows = rows(0.0, Color::White, Color::Gray, Color::Black);
        let t = text(&rows);
        assert_eq!(t.len(), HEIGHT);
        assert!(t.iter().all(|r| r.chars().count() == WIDTH), "{t:?}");
        // The middle row has dots at both ends and room for a number between.
        let mid = &t[HEIGHT / 2];
        assert!(!mid.starts_with(' ') && !mid.ends_with(' '), "{mid:?}");
        let inside: String = mid.chars().skip(2).take(WIDTH - 4).collect();
        assert!(inside.trim().is_empty(), "{mid:?}");
    }

    #[test]
    fn the_fill_runs_clockwise_from_the_top() {
        let fill = Color::Rgb(1, 2, 3);
        let quarter = rows(0.25, fill, Color::Gray, Color::Black);
        let filled = |l: &Line<'_>| l.spans.iter().filter(|s| s.style.fg == Some(fill)).count();
        // A quarter lights the top-right: the first row's right half.
        assert!(filled(&quarter[0]) > 0);
        assert_eq!(filled(&quarter[HEIGHT - 1]), 0, "nothing at the bottom yet");
        let full = rows(1.0, fill, Color::Gray, Color::Black);
        assert!(full.iter().all(|l| filled(l) > 0));
        let empty = rows(0.0, fill, Color::Gray, Color::Black);
        assert!(empty.iter().all(|l| filled(l) == 0));
    }
}
