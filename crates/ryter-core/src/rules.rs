//! The user's own rules for every project: `~/.ryter/RYTER.md`.
//!
//! Ryter puts the file's text into every role's instructions, ahead of the
//! project's own `RYTER.md`. The model changes it only through
//! `update_rules`, which shows the user the change and asks every time: the
//! file steers every later session, so nothing writes it unseen.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// The file's name in Ryter's home folder.
pub const FILE: &str = "RYTER.md";

/// The most Ryter loads or saves: rules are read on every model call.
pub const MAX_BYTES: usize = 32 * 1024;

/// `~/.ryter/RYTER.md`.
pub fn path(home: &Path) -> PathBuf {
    home.join(FILE)
}

/// The file as it is on disk, if there is one.
pub fn read(home: &Path) -> Option<String> {
    std::fs::read_to_string(path(home)).ok()
}

/// The rules to put in a prompt: `None` when there are none. A file past
/// [`MAX_BYTES`] is cut there, on a line, and says so.
pub fn load(home: &Path) -> Option<String> {
    let text = read(home)?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.len() <= MAX_BYTES {
        return Some(text.to_string());
    }
    let mut end = MAX_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let kept = match text[..end].rfind('\n') {
        Some(line) => &text[..line],
        None => &text[..end],
    };
    Some(format!(
        "{kept}\n\n(The rest of this file is not loaded: it is larger than {} KB.)",
        MAX_BYTES / 1024
    ))
}

/// Replace the file with `text` (ending in one newline). Written beside it
/// and renamed into place, so a crash can't leave half a file.
pub fn save(home: &Path, text: &str) -> Result<()> {
    let io = |e: std::io::Error| Error::Io(format!("{}: {e}", path(home).display()));
    std::fs::create_dir_all(home).map_err(io)?;
    let tmp = home.join(format!(".{FILE}.{}", std::process::id()));
    std::fs::write(&tmp, format!("{}\n", text.trim_end())).map_err(io)?;
    std::fs::rename(&tmp, path(home)).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io(e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn rules_are_saved_and_loaded() {
        let home = TempDir::new().unwrap();
        assert_eq!(load(home.path()), None, "no file, no rules");
        std::fs::write(path(home.path()), "  \n\n").unwrap();
        assert_eq!(load(home.path()), None, "an empty file is no rules");
        save(home.path(), "- Use British spelling.\n\n\n").unwrap();
        assert_eq!(
            read(home.path()).as_deref(),
            Some("- Use British spelling.\n")
        );
        assert_eq!(
            load(home.path()).as_deref(),
            Some("- Use British spelling.")
        );
        let left: Vec<_> = std::fs::read_dir(home.path()).unwrap().flatten().collect();
        assert_eq!(left.len(), 1, "no temporary file left");
    }

    /// A file larger than Ryter loads is cut on a line, and says so.
    #[test]
    fn a_long_file_is_cut_and_says_so() {
        let home = TempDir::new().unwrap();
        let line = "- a rule that is fairly long, to fill the file up quickly\n";
        std::fs::write(path(home.path()), line.repeat(MAX_BYTES / line.len() + 50)).unwrap();
        let loaded = load(home.path()).unwrap();
        assert!(loaded.len() < MAX_BYTES + 200);
        assert!(
            loaded.ends_with("larger than 32 KB.)"),
            "{}",
            &loaded[loaded.len() - 80..]
        );
        let body = loaded.split("\n\n(The rest").next().unwrap();
        assert!(body.lines().all(|l| l == line.trim_end()), "cut on a line");
    }
}
