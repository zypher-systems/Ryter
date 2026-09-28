//! The last model list each connection returned, kept on disk.
//!
//! OpenRouter's catalog is most of a megabyte, and on a bad day its endpoint
//! trickles it out over minutes. The model picker opens on the list it had
//! last time and refreshes behind it, instead of spinning until the download
//! ends.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::ModelInfo;

#[derive(Serialize, Deserialize)]
struct Cached {
    /// Unix milliseconds.
    fetched_ms: u64,
    models: Vec<ModelInfo>,
}

fn path(home: &Path, connection: &str) -> PathBuf {
    let safe: String = connection
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    home.join("cache")
        .join("models")
        .join(format!("{safe}.json"))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The list `connection` returned last, and how many seconds ago.
pub fn load(home: &Path, connection: &str) -> Option<(Vec<ModelInfo>, u64)> {
    let text = std::fs::read_to_string(path(home, connection)).ok()?;
    let c: Cached = serde_json::from_str(&text).ok()?;
    if c.models.is_empty() {
        return None;
    }
    Some((c.models, now_ms().saturating_sub(c.fetched_ms) / 1000))
}

/// Keep `models` as what `connection` returned just now.
pub fn save(home: &Path, connection: &str, models: &[ModelInfo]) {
    if models.is_empty() {
        return;
    }
    let p = path(home, connection);
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let body = Cached {
        fetched_ms: now_ms(),
        models: models.to_vec(),
    };
    if let Ok(json) = serde_json::to_vec(&body) {
        // Write then rename, so a reader never sees half a file.
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(tmp, p);
        }
    }
}

/// `3 min`, `2 h`, `5 days`: how old a cached list is.
pub fn age(secs: u64) -> String {
    match secs {
        s if s < 90 => "a moment".into(),
        s if s < 90 * 60 => format!("{} min", s / 60),
        s if s < 36 * 3600 => format!("{} h", s / 3600),
        s => format!("{} days", s / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_list_comes_back_with_its_age() {
        let home = tempfile::tempdir().unwrap();
        assert!(load(home.path(), "openrouter").is_none());
        save(
            home.path(),
            "openrouter",
            &[ModelInfo::named("a/b", Some(1000))],
        );
        let (models, secs) = load(home.path(), "openrouter").unwrap();
        assert_eq!(models[0].id, "a/b");
        assert!(secs < 5);
        assert_eq!(age(7200), "2 h");
        assert_eq!(age(30), "a moment");
    }
}
