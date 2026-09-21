//! Append-only session log (`~/.ryter/logs/ryter.log`). Never write secrets.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Roll the log over at this size, keeping one previous generation.
const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;

/// Append one line. Failures are ignored (logging must not break the harness).
pub fn log(home: &Path, msg: &str) {
    let dir = home.join("logs");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    // Append-only with no ceiling grows without bound for the life of the
    // install. One rotation keeps recent history without unbounded disk.
    let path = dir.join("ryter.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
        let _ = std::fs::rename(&path, dir.join("ryter.log.1"));
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!("{ts} {msg}\n");
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let _ = f.write_all(line.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn appends_a_line() {
        let dir = TempDir::new().unwrap();
        log(dir.path(), "session start");
        let text = std::fs::read_to_string(dir.path().join("logs/ryter.log")).unwrap();
        assert!(text.contains("session start"));
        assert!(!text.contains("api_key"));
    }
}
