//! Prompt history: 100 entries, per session, in memory only (`R-COMP-13`).

/// Ring of submitted prompts with a browse cursor.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct History {
    entries: Vec<String>,
    /// Browse position; `None` = not browsing.
    pos: Option<usize>,
    /// Text that was in the composer when browsing began.
    draft: String,
}

/// Capacity.
pub const CAP: usize = 100;

impl History {
    /// Record a submitted prompt (deduplicates an immediate repeat).
    pub fn push(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        if self.entries.last().is_some_and(|l| l == text) {
            self.reset();
            return;
        }
        self.entries.push(text.to_string());
        if self.entries.len() > CAP {
            let over = self.entries.len() - CAP;
            self.entries.drain(0..over);
        }
        self.reset();
    }

    /// Stop browsing.
    pub fn reset(&mut self) {
        self.pos = None;
        self.draft.clear();
    }

    /// True while browsing.
    pub fn browsing(&self) -> bool {
        self.pos.is_some()
    }

    /// Entries held.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing is stored.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `↑`: older entry. Returns the text to place in the composer.
    pub fn back(&mut self, current: &str) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }
        let next = match self.pos {
            None => {
                self.draft = current.to_string();
                self.entries.len() - 1
            }
            Some(0) => 0,
            Some(p) => p - 1,
        };
        self.pos = Some(next);
        self.entries.get(next).map(String::as_str)
    }

    /// `↓`: newer entry, or the saved draft past the end.
    pub fn forward(&mut self) -> Option<String> {
        let p = self.pos?;
        if p + 1 < self.entries.len() {
            self.pos = Some(p + 1);
            self.entries.get(p + 1).cloned()
        } else {
            self.pos = None;
            Some(std::mem::take(&mut self.draft))
        }
    }

    /// Forget everything (`/new`, `/resume`).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browse_back_and_forward_restores_draft() {
        let mut h = History::default();
        h.push("one");
        h.push("two");
        h.push("two");
        assert_eq!(h.len(), 2);
        assert_eq!(h.back("draft"), Some("two"));
        assert_eq!(h.back(""), Some("one"));
        assert_eq!(h.back(""), Some("one"));
        assert_eq!(h.forward().as_deref(), Some("two"));
        assert_eq!(h.forward().as_deref(), Some("draft"));
        assert!(!h.browsing());
        assert_eq!(h.forward(), None);
    }

    #[test]
    fn capped_at_100() {
        let mut h = History::default();
        for i in 0..150 {
            h.push(&format!("p{i}"));
        }
        assert_eq!(h.len(), CAP);
        assert_eq!(h.back(""), Some("p149"));
    }
}
