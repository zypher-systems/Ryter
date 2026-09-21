//! In-tree fuzzy scorer (`R-PAL-08..12`). No dependency.

/// Score tiers (`R-PAL-09`). Higher wins.
pub const EXACT: u32 = 1000;
/// Name prefix.
pub const PREFIX: u32 = 800;
/// Name subsequence (plus consecutive-run bonus).
pub const SUBSEQ: u32 = 600;
/// Alias match.
pub const ALIAS: u32 = 400;
/// Description match.
pub const DESC: u32 = 200;
/// Recently used boost (`R-PAL-12`).
pub const RECENT: u32 = 50;

/// A scored match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Score {
    /// Tier plus bonuses.
    pub score: u32,
    /// Char indices in `name` that matched (for highlighting, `R-PAL-10`).
    pub name_hits: Vec<usize>,
}

/// Subsequence match of `q` in `hay` (both lowercase); returns matched indices.
fn subsequence(q: &str, hay: &str) -> Option<Vec<usize>> {
    let mut hits = Vec::with_capacity(q.len());
    let mut it = hay.char_indices().enumerate();
    for qc in q.chars() {
        let found = it.by_ref().find(|(_, (_, c))| *c == qc)?;
        hits.push(found.0);
    }
    Some(hits)
}

fn run_bonus(hits: &[usize]) -> u32 {
    let mut bonus = 0u32;
    for w in hits.windows(2) {
        if w[1] == w[0] + 1 {
            bonus += 8;
        }
    }
    if hits.first() == Some(&0) {
        bonus += 10;
    }
    bonus
}

/// Score `query` against a command. `/` separators are matchable (`R-PAL-11`).
pub fn score(
    query: &str,
    name: &str,
    aliases: &[&str],
    description: &str,
    recent: bool,
) -> Option<Score> {
    let q = query.trim().trim_start_matches('/').to_ascii_lowercase();
    let name_l = name.to_ascii_lowercase();
    let boost = if recent { RECENT } else { 0 };
    if q.is_empty() {
        return Some(Score {
            score: boost,
            name_hits: Vec::new(),
        });
    }
    if name_l == q {
        return Some(Score {
            score: EXACT + boost,
            name_hits: (0..name.chars().count()).collect(),
        });
    }
    if name_l.starts_with(&q) {
        return Some(Score {
            score: PREFIX + boost + (q.len() as u32).min(50),
            name_hits: (0..q.chars().count()).collect(),
        });
    }
    if let Some(hits) = subsequence(&q, &name_l) {
        // Penalize spread so tighter matches rank first.
        let spread = hits.last().copied().unwrap_or(0) - hits.first().copied().unwrap_or(0);
        let penalty = (spread as u32).min(40);
        return Some(Score {
            score: SUBSEQ + boost + run_bonus(&hits) - penalty,
            name_hits: hits,
        });
    }
    for a in aliases {
        let al = a.to_ascii_lowercase();
        if al == q || al.starts_with(&q) {
            return Some(Score {
                score: ALIAS + boost + if al == q { 20 } else { 0 },
                name_hits: Vec::new(),
            });
        }
        if subsequence(&q, &al).is_some() {
            return Some(Score {
                score: ALIAS + boost - 20,
                name_hits: Vec::new(),
            });
        }
    }
    let d = description.to_ascii_lowercase();
    if d.contains(&q) {
        return Some(Score {
            score: DESC + boost + 20,
            name_hits: Vec::new(),
        });
    }
    if q.split_whitespace().all(|w| d.contains(w)) {
        return Some(Score {
            score: DESC + boost,
            name_hits: Vec::new(),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_order_as_specified() {
        let exact = score("models", "models", &["model"], "switch", false).unwrap();
        let prefix = score("mod", "models", &["model"], "switch", false).unwrap();
        let subseq = score("mdl", "models", &["model"], "switch", false).unwrap();
        let alias = score("conn", "provider", &["connections"], "switch", false).unwrap();
        let desc = score("switch", "provider", &[], "switch connection", false).unwrap();
        assert!(exact.score > prefix.score);
        assert!(prefix.score > subseq.score);
        assert!(subseq.score > alias.score);
        assert!(alias.score > desc.score);
        assert_eq!(subseq.name_hits, vec![0, 2, 4]);
        assert!(score("zzz", "models", &[], "switch", false).is_none());
    }

    #[test]
    fn recent_boost_breaks_ties_and_slash_is_matchable() {
        let a = score("re", "review", &[], "", false).unwrap();
        let b = score("re", "review", &[], "", true).unwrap();
        assert_eq!(b.score, a.score + RECENT);
        assert!(score("pr/st", "pr/status", &[], "", false).is_some());
    }
}
