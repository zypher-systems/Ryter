//! Pages the model shows the user (`show_page`, the canvas skill): one
//! self-contained HTML file per title, saved in the session and opened in the
//! user's browser.

use std::path::Path;

/// Put first in every page's head. The page may load nothing from the
/// network: no CDN scripts, web fonts, images or trackers, and no form posts.
/// Everything it needs is inline.
const CSP: &str = "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; \
     img-src data: blob:; media-src data: blob:; font-src data:; style-src 'unsafe-inline'; \
     script-src 'unsafe-inline'; form-action 'none'; base-uri 'none'\">";

/// Where a session's pages live: `~/.ryter/pages/<session id>/`. Short, so
/// its link fits on one line of the chat and a terminal can open it; the
/// session's own folder is named after the whole project path.
pub fn dir(home: &Path, session_id: &str) -> std::path::PathBuf {
    home.join("pages").join(session_id)
}

/// The file name for a page titled `title`: `crew-cost-by-task`.
pub fn slug(title: &str) -> String {
    let mut s = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('-') {
            s.push('-');
        }
    }
    let s: String = s.trim_matches('-').chars().take(60).collect();
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() { "page".into() } else { s }
}

/// Where `<tag` opens in `lower` (lowercased HTML): `<head>` or
/// `<head lang=…>`, not `<header>`.
fn find_tag(lower: &str, tag: &str) -> Option<usize> {
    let open = format!("<{tag}");
    let mut from = 0;
    while let Some(i) = lower[from..].find(&open).map(|i| i + from) {
        let next = lower[i + open.len()..].chars().next();
        if matches!(
            next,
            Some('>') | Some(' ') | Some('\t') | Some('\n') | Some('\r')
        ) {
            return Some(i);
        }
        from = i + open.len();
    }
    None
}

/// `html`, with the network policy first in its head.
pub fn sealed(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let after = |i: usize| lower[i..].find('>').map(|j| i + j + 1);
    if let Some(at) = find_tag(&lower, "head").and_then(after) {
        return format!("{}{CSP}{}", &html[..at], &html[at..]);
    }
    if let Some(at) = find_tag(&lower, "html").and_then(after) {
        return format!("{}<head>{CSP}</head>{}", &html[..at], &html[at..]);
    }
    if lower.trim_start().starts_with("<!doctype") {
        if let Some(at) = lower.find('>').map(|j| j + 1) {
            return format!("{}<head>{CSP}</head>{}", &html[..at], &html[at..]);
        }
    }
    format!("{CSP}{html}")
}

/// A `file://` link to `path` that a terminal and a browser both read right.
/// Session folders are named with `%2F`, which a browser would decode.
pub fn file_url(path: &Path) -> String {
    let mut url = String::from("file://");
    for c in path.to_string_lossy().chars() {
        match c {
            '%' => url.push_str("%25"),
            ' ' => url.push_str("%20"),
            '#' => url.push_str("%23"),
            '?' => url.push_str("%3F"),
            c => url.push(c),
        }
    }
    url
}

/// Open `path` in the user's browser. False when there is no desktop to
/// open it on (an SSH session, a server) or the opener could not start.
pub fn open(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(not(target_os = "macos"))]
    let opener = {
        if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return false;
        }
        "xdg-open"
    };
    match std::process::Command::new(opener)
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            // Reap it when it exits; the browser outlives it.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_become_file_names() {
        assert_eq!(slug("Crew cost by task"), "crew-cost-by-task");
        assert_eq!(
            slug("  v0.7 → v0.8: what changed?  "),
            "v0-7-v0-8-what-changed"
        );
        assert_eq!(slug("…"), "page");
        assert!(slug(&"x".repeat(200)).len() <= 60);
    }

    /// Whatever the page's shape, the network policy is first in its head.
    #[test]
    fn every_page_is_sealed_from_the_network() {
        let full = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>t</title></head><body><header>h</header></body></html>";
        let out = sealed(full);
        assert!(
            out.contains("<head><meta http-equiv=\"Content-Security-Policy\""),
            "{out}"
        );
        assert_eq!(out.matches("Content-Security-Policy").count(), 1);
        // `<header>` is not the head.
        let no_head = "<!doctype html><html><body><header>h</header></body></html>";
        let out = sealed(no_head);
        assert!(
            out.starts_with("<!doctype html><html><head><meta http-equiv"),
            "{out}"
        );
        let bare_doctype = "<!DOCTYPE html><p>hi</p>";
        assert!(sealed(bare_doctype).starts_with("<!DOCTYPE html><head><meta http-equiv"));
        let fragment = "<p>hi</p>";
        assert!(sealed(fragment).starts_with("<meta http-equiv"));
        assert!(CSP.contains("default-src 'none'") && CSP.contains("form-action 'none'"));
    }

    /// Session folders are named with `%2F`; a browser would decode them.
    #[test]
    fn file_links_survive_the_session_folder_names() {
        let p = Path::new("/home/me/.ryter/sessions/%2Fhome%2Fme%2Fapp/01/pages/my page.html");
        assert_eq!(
            file_url(p),
            "file:///home/me/.ryter/sessions/%252Fhome%252Fme%252Fapp/01/pages/my%20page.html"
        );
    }
}
