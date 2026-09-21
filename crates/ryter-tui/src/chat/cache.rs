//! LRU render cache (`R-PERF-01..04`).
//!
//! Keyed by `(message.id, message.rev, pane_width, theme_generation, flags)`.
//! A hit never re-parses markdown or re-runs syntect.

use std::collections::HashMap;
use std::rc::Rc;

use ratatui::text::Line;

/// Entry cap (`R-PERF-03`).
pub const MAX_ENTRIES: usize = 2000;
/// Byte cap, estimated (`R-PERF-03`).
pub const MAX_BYTES: usize = 20 * 1024 * 1024;

/// Cache key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    /// `Message::id`.
    pub id: u64,
    /// `Message::rev`.
    pub rev: u64,
    /// Pane width.
    pub width: u16,
    /// `Theme::generation`.
    pub generation: u32,
    /// Render-option bits (timestamps, line numbers, continuation, hint hash).
    pub flags: u64,
}

/// Rendered rows plus their count.
#[derive(Debug)]
pub struct Entry {
    /// Rows.
    pub lines: Vec<Line<'static>>,
    /// Estimated heap bytes.
    pub bytes: usize,
}

impl Entry {
    /// Row count (`R-PERF-04`).
    pub fn rows(&self) -> usize {
        self.lines.len()
    }
}

/// Bounded LRU of rendered messages.
pub struct RenderCache {
    map: HashMap<Key, (Rc<Entry>, u64)>,
    tick: u64,
    bytes: usize,
    /// Hits since creation (tests).
    pub hits: u64,
    /// Misses since creation (tests).
    pub misses: u64,
}

impl std::fmt::Debug for RenderCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderCache")
            .field("entries", &self.map.len())
            .field("bytes", &self.bytes)
            .field("hits", &self.hits)
            .field("misses", &self.misses)
            .finish()
    }
}

impl Clone for RenderCache {
    /// Clones start empty: the cache is derived state and rebuilds on demand.
    fn clone(&self) -> Self {
        Self::new()
    }
}

impl Default for RenderCache {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderCache {
    /// Empty cache.
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            tick: 0,
            bytes: 0,
            hits: 0,
            misses: 0,
        }
    }

    /// Entries held.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True when nothing is cached.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Estimated bytes held.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Drop everything (theme change, `/new`).
    pub fn clear(&mut self) {
        self.map.clear();
        self.bytes = 0;
    }

    /// Fetch or render.
    pub fn get_or_insert(
        &mut self,
        key: Key,
        render: impl FnOnce() -> Vec<Line<'static>>,
    ) -> Rc<Entry> {
        self.tick += 1;
        if let Some((e, used)) = self.map.get_mut(&key) {
            *used = self.tick;
            self.hits += 1;
            return e.clone();
        }
        self.misses += 1;
        let lines = render();
        let bytes = estimate(&lines);
        let entry = Rc::new(Entry { lines, bytes });
        self.bytes += bytes;
        self.map.insert(key, (entry.clone(), self.tick));
        self.evict();
        entry
    }

    fn evict(&mut self) {
        while self.map.len() > MAX_ENTRIES || self.bytes > MAX_BYTES {
            let Some((&k, _)) = self.map.iter().min_by_key(|(_, (_, used))| *used) else {
                break;
            };
            if let Some((e, _)) = self.map.remove(&k) {
                self.bytes = self.bytes.saturating_sub(e.bytes);
            }
        }
    }
}

fn estimate(lines: &[Line<'static>]) -> usize {
    lines
        .iter()
        .map(|l| 32 + l.spans.iter().map(|s| 40 + s.content.len()).sum::<usize>())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Span;

    fn key(id: u64, rev: u64) -> Key {
        Key {
            id,
            rev,
            width: 80,
            generation: 0,
            flags: 0,
        }
    }

    #[test]
    fn hit_does_not_rerender() {
        let mut c = RenderCache::new();
        let mut calls = 0;
        for _ in 0..3 {
            c.get_or_insert(key(1, 0), || {
                calls += 1;
                vec![Line::from(Span::raw("x"))]
            });
        }
        assert_eq!(calls, 1);
        assert_eq!(c.hits, 2);
        // A rev bump misses.
        c.get_or_insert(key(1, 1), || {
            calls += 1;
            vec![]
        });
        assert_eq!(calls, 2);
    }

    #[test]
    fn evicts_least_recently_used_past_cap() {
        let mut c = RenderCache::new();
        for i in 0..(MAX_ENTRIES as u64 + 5) {
            c.get_or_insert(key(i, 0), || vec![Line::from(Span::raw("y"))]);
        }
        assert_eq!(c.len(), MAX_ENTRIES);
        // The oldest keys are gone; the newest remain.
        assert!(!c.map.contains_key(&key(0, 0)));
        assert!(c.map.contains_key(&key(MAX_ENTRIES as u64 + 4, 0)));
    }
}
