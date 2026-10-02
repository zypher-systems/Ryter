//! The watermark (`docs/hat-rack-design.md` §7): a fedora behind the
//! conversation, in the color of the hat that is on. It is a tint on the
//! cells' backgrounds, so the text over it is untouched; on a row with no
//! text, half-blocks draw the shape to half a row.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ryter_core::Role;

use crate::theme::Theme;

/// Columns the hat takes.
pub const WIDTH: u16 = 64;
/// Rows it takes: thirty half-rows.
pub const HEIGHT: u16 = 15;
/// The least room it is drawn in: itself and a cell of margin all round.
pub const MIN_AREA: (u16, u16) = (WIDTH + 2, HEIGHT + 2);

/// The half-rows the band is on.
const BAND: std::ops::Range<usize> = 16..20;

/// The hat, a half-row a line: each run is `(first column, columns)`.
/// Two runs at the top are the pinch in the crown.
const SHAPE: [&[(u8, u8)]; 30] = [
    &[(22, 8), (34, 8)],
    &[(20, 11), (33, 11)],
    &[(19, 26)],
    &[(18, 28)],
    &[(17, 30)],
    &[(17, 30)],
    &[(16, 32)],
    &[(16, 32)],
    &[(16, 32)],
    &[(15, 34)],
    &[(15, 34)],
    &[(15, 34)],
    &[(14, 36)],
    &[(14, 36)],
    &[(14, 36)],
    &[(14, 36)],
    &[(13, 38)],
    &[(13, 38)],
    &[(13, 38)],
    &[(13, 38)],
    &[(13, 38)],
    &[(6, 52)],
    &[(2, 60)],
    &[(0, 64)],
    &[(0, 64)],
    &[(1, 62)],
    &[(4, 56)],
    &[(10, 44)],
    &[(18, 28)],
    &[],
];

/// Which part of the hat a half-cell is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Hat,
    Band,
}

/// What is at column `x` of half-row `y`, if the hat is there at all.
fn part(x: u16, y: usize) -> Option<Part> {
    let inside = SHAPE.get(y)?.iter().any(|&(from, len)| {
        let from = u16::from(from);
        x >= from && x < from + u16::from(len)
    });
    inside.then_some(if BAND.contains(&y) {
        Part::Band
    } else {
        Part::Hat
    })
}

/// Where the hat goes in `area`: centred across it, its top a third of
/// the way down, moved up where that would run it off the foot. `None`
/// when `area` is too small: it is never scaled or cut.
pub fn place(area: Rect) -> Option<Rect> {
    if area.width < MIN_AREA.0 || area.height < MIN_AREA.1 {
        return None;
    }
    let down = (area.height / 3).min(area.height - HEIGHT - 1);
    Some(Rect {
        x: area.x + (area.width - WIDTH) / 2,
        y: area.y + down,
        width: WIDTH,
        height: HEIGHT,
    })
}

/// Tint `area` with the hat, in `hat`'s color. Call it after the
/// conversation is drawn and before anything is laid over it. Only cells
/// on the screen's own background are touched: an edit's row, a code
/// block, a pinned header and a selection keep theirs.
pub fn draw(frame: &mut Frame, area: Rect, hat: Role, theme: Theme) {
    let (Some((hat_tint, band_tint)), Some(at)) = (theme.watermark(hat), place(area)) else {
        return;
    };
    let tint = |p: Part| -> Color {
        match p {
            Part::Hat => hat_tint,
            Part::Band => band_tint,
        }
    };
    let buf = frame.buffer_mut();
    for row in 0..HEIGHT {
        // A row with words on it is tinted a whole cell at a time: a
        // half-block in the gap between two words reads as a stray mark.
        let worded = (0..WIDTH).any(|col| {
            let (x, y) = (at.x + col, at.y + row);
            x < buf.area.width && y < buf.area.height && buf[(x, y)].symbol() != " "
        });
        for col in 0..WIDTH {
            let (x, y) = (at.x + col, at.y + row);
            if x >= buf.area.width || y >= buf.area.height {
                continue;
            }
            let top = part(col, usize::from(row) * 2);
            let bottom = part(col, usize::from(row) * 2 + 1);
            if top.is_none() && bottom.is_none() {
                continue;
            }
            let cell = &mut buf[(x, y)];
            if cell.bg != theme.bg {
                continue;
            }
            if worded {
                // Text keeps its letters and its color; the cell takes the
                // tint its upper half has.
                if let Some(p) = top {
                    cell.set_bg(tint(p));
                }
                continue;
            }
            match (top, bottom) {
                (Some(t), Some(b)) if t == b => {
                    cell.set_bg(tint(t));
                }
                (Some(t), Some(b)) => {
                    cell.set_symbol("▀");
                    cell.set_style(Style::default().fg(tint(t)).bg(tint(b)));
                }
                (Some(t), None) => {
                    cell.set_symbol("▀");
                    cell.set_style(Style::default().fg(tint(t)).bg(theme.bg));
                }
                (None, Some(b)) => {
                    cell.set_symbol("▄");
                    cell.set_style(Style::default().fg(tint(b)).bg(theme.bg));
                }
                (None, None) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shape_is_the_contracts_bitmap() {
        // The crown's pinch, the band, the brim at its widest, and nothing
        // on the last half-row.
        assert_eq!(part(21, 0), None);
        assert_eq!(part(22, 0), Some(Part::Hat));
        assert_eq!(part(31, 0), None, "the pinch");
        assert_eq!(part(13, 16), Some(Part::Band));
        assert_eq!(part(50, 19), Some(Part::Band));
        assert_eq!(part(13, 20), Some(Part::Hat));
        assert_eq!(part(0, 23), Some(Part::Hat));
        assert_eq!(part(63, 24), Some(Part::Hat));
        assert_eq!(part(0, 25), None);
        assert!((0..WIDTH).all(|x| part(x, 29).is_none()));
        // Every run stays inside the hat's width.
        for runs in SHAPE {
            for &(from, len) in runs {
                assert!(u16::from(from) + u16::from(len) <= WIDTH);
            }
        }
    }

    #[test]
    fn it_is_placed_whole_or_not_at_all() {
        let at = |w, h| {
            place(Rect {
                x: 10,
                y: 2,
                width: w,
                height: h,
            })
        };
        assert_eq!(at(65, 40), None);
        assert_eq!(at(80, 16), None);
        let r = at(66, 17).unwrap();
        assert_eq!((r.x, r.y, r.width, r.height), (11, 3, 64, 15));
        // A third of the way down a tall area, centred in a wide one.
        let r = at(100, 60).unwrap();
        assert_eq!((r.x, r.y), (28, 22));
        // Never off the foot.
        let r = at(70, 20).unwrap();
        assert!(r.y + r.height < 2 + 20);
    }
}
