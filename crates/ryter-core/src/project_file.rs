//! Reads anchored to a project directory. Project-controlled links are never followed.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Component, Path};

pub(crate) const TRUNCATED: &str = "\n…(truncated)\n";

/// Open a regular file through plain relative components. The root itself
/// may be a link (as `/var` is on macOS); links below it are refused.
pub(crate) fn open(root: &Path, rel: &Path) -> io::Result<File> {
    let parts: Vec<_> = rel.components().collect();
    if parts.is_empty() || parts.iter().any(|p| !matches!(p, Component::Normal(_))) {
        return Err(io::Error::other("not a project-relative file"));
    }
    #[cfg(unix)]
    let file = {
        use rustix::fs::{CWD, Mode, OFlags, openat};
        let mut dir = openat(
            CWD,
            root,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        for part in &parts[..parts.len() - 1] {
            dir = openat(
                &dir,
                part.as_os_str(),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )?;
        }
        File::from(openat(
            &dir,
            parts.last().unwrap().as_os_str(),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )?)
    };
    // Fail closed on platforms without a descriptor-relative, no-follow
    // implementation. Those platforms are not yet supported by Ryter.
    #[cfg(not(unix))]
    let file: File = {
        let _ = root;
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "safe project reads are not supported on this platform",
        ));
    };
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("not a regular project file"));
    }
    Ok(file)
}

/// A bounded UTF-8 prefix, with an explicit marker when more bytes exist.
pub(crate) fn read(root: &Path, rel: &Path, cap: usize) -> io::Result<String> {
    let mut bytes = Vec::new();
    open(root, rel)?
        .take(cap.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    let truncated = bytes.len() > cap;
    if truncated {
        bytes.truncate(cap.saturating_sub(TRUNCATED.len()));
        // Only an incomplete final character may be discarded.
        if let Err(e) = std::str::from_utf8(&bytes) {
            if e.error_len().is_none() {
                bytes.truncate(e.valid_up_to());
            }
        }
    }
    let mut text =
        String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if truncated && cap >= TRUNCATED.len() {
        text.push_str(TRUNCATED);
    }
    Ok(text)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    #[test]
    fn refuses_links_and_traversal_but_accepts_a_linked_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project");
        fs::create_dir(&root).unwrap();
        fs::write(tmp.path().join("secret"), "private").unwrap();
        fs::write(root.join("own"), "public").unwrap();
        symlink("../secret", root.join("alias")).unwrap();
        symlink("..", root.join("folder")).unwrap();
        symlink(&root, tmp.path().join("linked-project")).unwrap();
        for rel in [
            "alias",
            "folder/secret",
            "../secret",
            "/etc/passwd",
            "folder",
        ] {
            assert!(open(&root, Path::new(rel)).is_err(), "{rel}");
        }
        assert_eq!(
            read(&tmp.path().join("linked-project"), Path::new("own"), 100).unwrap(),
            "public"
        );
    }

    #[test]
    fn caps_bytes_without_splitting_utf8() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("text"), "é".repeat(100)).unwrap();
        let text = read(tmp.path(), Path::new("text"), 31).unwrap();
        assert!(text.len() <= 31);
        assert!(text.ends_with(TRUNCATED));
    }
}
