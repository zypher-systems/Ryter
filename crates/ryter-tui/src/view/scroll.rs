//! Chat scroll state, anchoring, and sticky-header math (`R-SCROLL-*`).
//!
//! The draw path reports `doc_rows` / `viewport` back through `Cell`s so key
//! handlers can clamp without re-rendering; `View` stays terminal-free.

use std::cell::{Cell, RefCell};

/// Scroll state for the chat pane.
#[derive(Debug, Clone, Default)]
pub struct ChatScroll {
    /// Rows from the top of the fully rendered document (used when `!follow`).
    pub offset: usize,
    /// True = stick to the bottom / anchor.
    pub follow: bool,
    /// Turn whose user message is anchored (`R-SCROLL-02`).
    pub anchor_turn: Option<u64>,
    /// Document height when follow was released; drives the `↓ N new` pill.
    pub detach_rows: Option<usize>,
    /// Document height from the last draw.
    pub doc_rows: Cell<usize>,
    /// Viewport height from the last draw.
    pub viewport: Cell<usize>,
    /// Effective offset from the last draw.
    pub effective: Cell<usize>,
    /// `(turn, top row)` of each turn's first message, from the last draw.
    pub turn_tops: RefCell<Vec<(u64, usize)>>,
}

/// Sticky header to paint over the top of the viewport (`R-SCROLL-04`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sticky {
    /// Anchored turn.
    pub turn: u64,
    /// Rows the real message sits above the viewport (`↑ N`).
    pub rows_above: usize,
}

/// Result of resolving the scroll state against a rendered document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolved {
    /// First visible document row.
    pub offset: usize,
    /// Sticky header, if engaged.
    pub sticky: Option<Sticky>,
    /// Rows appended since follow was released (`↓ N new`).
    pub new_rows: usize,
    /// True when the document does not fit the viewport.
    pub overflow: bool,
}

impl ChatScroll {
    /// Follow-bottom, no anchor.
    pub fn new() -> Self {
        Self {
            follow: true,
            ..Self::default()
        }
    }

    /// New turn submitted (`R-SCROLL-02`).
    pub fn on_submit(&mut self, turn: u64) {
        self.follow = true;
        self.anchor_turn = Some(turn);
        self.detach_rows = None;
    }

    /// Resolve against the current document.
    pub fn resolve(
        &self,
        doc_rows: usize,
        viewport: usize,
        anchor_top: Option<usize>,
        busy: bool,
    ) -> Resolved {
        let max_off = doc_rows.saturating_sub(viewport);
        let overflow = doc_rows > viewport;
        let (offset, sticky) = if self.follow {
            match anchor_top {
                Some(top) if doc_rows.saturating_sub(top) <= viewport => (top, None),
                Some(top) => {
                    let s = (top < max_off).then_some(Sticky {
                        turn: self.anchor_turn.unwrap_or(0),
                        rows_above: max_off - top,
                    });
                    (max_off, s)
                }
                None => (max_off, None),
            }
        } else {
            let off = self.offset.min(max_off);
            let s = match anchor_top {
                Some(top) if busy && top < off => Some(Sticky {
                    turn: self.anchor_turn.unwrap_or(0),
                    rows_above: off - top,
                }),
                _ => None,
            };
            (off, s)
        };
        let new_rows = if self.follow {
            0
        } else {
            doc_rows.saturating_sub(self.detach_rows.unwrap_or(doc_rows))
        };
        self.doc_rows.set(doc_rows);
        self.viewport.set(viewport);
        self.effective.set(offset);
        Resolved {
            offset,
            sticky,
            new_rows,
            overflow,
        }
    }

    fn max_off(&self) -> usize {
        self.doc_rows.get().saturating_sub(self.viewport.get())
    }

    /// Move by `delta` rows (negative = up). Upward input releases follow
    /// (`R-SCROLL-07`); reaching the bottom re-engages it (`R-SCROLL-09`).
    pub fn scroll_by(&mut self, delta: isize, busy: bool) {
        let cur = self.effective.get();
        let max = self.max_off();
        let next = if delta < 0 {
            cur.saturating_sub(delta.unsigned_abs())
        } else {
            cur.saturating_add(delta as usize).min(max)
        };
        if next >= max {
            self.follow = true;
            self.detach_rows = None;
            self.offset = max;
        } else {
            if self.follow {
                self.detach_rows = Some(self.doc_rows.get());
            }
            self.follow = false;
            self.offset = next;
            if !busy {
                self.anchor_turn = None;
            }
        }
        self.effective.set(self.offset);
    }

    /// `PgUp`: one viewport minus two rows.
    pub fn page_up(&mut self, busy: bool) {
        let n = self.viewport.get().saturating_sub(2).max(1);
        self.scroll_by(-(n as isize), busy);
    }

    /// `PgDn`.
    pub fn page_down(&mut self, busy: bool) {
        let n = self.viewport.get().saturating_sub(2).max(1);
        self.scroll_by(n as isize, busy);
    }

    /// `Ctrl+Home`.
    #[allow(clippy::wrong_self_convention)]
    pub fn to_top(&mut self, busy: bool) {
        let cur = self.effective.get();
        self.scroll_by(-(cur as isize) - 1, busy);
        self.offset = 0;
        self.effective.set(0);
        if self.max_off() == 0 {
            self.follow = true;
        }
    }

    /// `Ctrl+End` / `End`: bottom and re-follow (`R-SCROLL-08`).
    #[allow(clippy::wrong_self_convention)]
    pub fn to_bottom(&mut self) {
        self.follow = true;
        self.detach_rows = None;
        self.offset = self.max_off();
        self.effective.set(self.offset);
    }

    /// `Ctrl+↑`: previous turn top (or the anchor when the sticky is shown).
    pub fn prev_turn(&mut self, busy: bool) {
        let cur = self.effective.get();
        let tops = self.turn_tops.borrow();
        let target = tops.iter().rev().map(|(_, t)| *t).find(|t| *t < cur);
        drop(tops);
        if let Some(t) = target {
            let delta = t as isize - cur as isize;
            self.scroll_by(delta, busy);
            self.offset = t;
            self.effective.set(t);
        }
    }

    /// `Ctrl+↓`: next turn top.
    pub fn next_turn(&mut self, busy: bool) {
        let cur = self.effective.get();
        let tops = self.turn_tops.borrow();
        let target = tops.iter().map(|(_, t)| *t).find(|t| *t > cur);
        drop(tops);
        match target {
            Some(t) => {
                let delta = t as isize - cur as isize;
                self.scroll_by(delta, busy);
            }
            None => self.to_bottom(),
        }
    }

    /// Mouse wheel: three rows.
    pub fn wheel(&mut self, up: bool, busy: bool) {
        self.scroll_by(if up { -3 } else { 3 }, busy);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_holds_then_sticky_engages() {
        let mut s = ChatScroll::new();
        s.on_submit(3);
        // Turn 3 starts at row 40 in a 100-row doc; viewport 20; content 60 rows → overflow.
        let r = s.resolve(100, 20, Some(40), true);
        assert_eq!(r.offset, 80);
        assert_eq!(
            r.sticky,
            Some(Sticky {
                turn: 3,
                rows_above: 40
            })
        );
        // Content fits: anchored at the message top, no sticky.
        let r = s.resolve(55, 20, Some(40), true);
        assert_eq!(r.offset, 40);
        assert_eq!(r.sticky, None);
    }

    #[test]
    fn scrolling_up_detaches_and_bottom_reattaches() {
        let mut s = ChatScroll::new();
        s.resolve(100, 20, None, false);
        s.page_up(false);
        assert!(!s.follow);
        assert_eq!(s.offset, 80 - 18);
        // New content while detached does not move the viewport and counts as new.
        let r = s.resolve(130, 20, None, false);
        assert_eq!(r.offset, 62);
        assert_eq!(r.new_rows, 30);
        s.to_bottom();
        assert!(s.follow);
        let r = s.resolve(130, 20, None, false);
        assert_eq!(r.offset, 110);
        assert_eq!(r.new_rows, 0);
        // Scrolling down to the exact bottom re-engages follow.
        s.page_up(false);
        s.scroll_by(100, false);
        assert!(s.follow);
    }

    #[test]
    fn turn_navigation_uses_tops() {
        let mut s = ChatScroll::new();
        s.resolve(100, 20, None, false);
        *s.turn_tops.borrow_mut() = vec![(1, 0), (2, 30), (3, 70)];
        s.prev_turn(false);
        assert_eq!(s.effective.get(), 70);
        s.prev_turn(false);
        assert_eq!(s.effective.get(), 30);
        s.next_turn(false);
        assert_eq!(s.effective.get(), 70);
        s.next_turn(false);
        assert!(s.follow);
        s.to_top(false);
        assert_eq!(s.effective.get(), 0);
        assert!(!s.follow);
    }
}
