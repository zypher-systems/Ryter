//! Append-only session log (`~/.ryter/logs/ryter.log`). Never write secrets.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Append one line. Failures are ignored (logging must not break the harness).
pub fn log(home: &Path, msg: &str) {
    let dir = home.join("logs");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!("{ts} {msg}\n");
    let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("ryter.log"))
    else {
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
