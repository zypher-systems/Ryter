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

/// `html` with the network policy ahead of everything in it: a doctype,
/// the policy, then the page. The parser puts the policy in the head before
/// it reads anything of the page's, so nothing can load first. Inserting it
/// at the first `<head>` in the text was defeated by a `<head>` inside a
/// comment, and by a script or image placed before the real head.
pub fn sealed(html: &str) -> String {
    format!("<!doctype html>{CSP}{}", without_leading_doctype(html))
}

/// `html` less a doctype it opens with (after whitespace or a BOM): the
/// sealed page supplies its own.
fn without_leading_doctype(html: &str) -> &str {
    let t = html.trim_start_matches(['\u{feff}', ' ', '\t', '\n', '\r']);
    let opens = t
        .get(..9)
        .is_some_and(|p| p.eq_ignore_ascii_case("<!doctype"));
    match t.find('>') {
        Some(end) if opens => &t[end + 1..],
        _ => html,
    }
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
/// open it on (an SSH session, a server), or the opener could not start or
/// said it failed. An opener still running after two seconds has handed the
/// page over (some wait on the browser); it is reaped when it exits.
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
    let Ok(mut child) = std::process::Command::new(opener)
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return false;
    };
    let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < until {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(_) => return false,
        }
    }
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    true
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

    /// The policy comes before anything of the page's, whatever its shape.
    /// Both of a reviewer's pages defeated the first version: a `<head>` in
    /// a comment, and a script ahead of the real head.
    #[test]
    fn every_page_is_sealed_from_the_network() {
        let first = format!("<!doctype html>{CSP}");
        for page in [
            "<!doctype html><!-- template: <head> --><html><head></head><body><img src=\"http://127.0.0.1:1/c\"></body></html>",
            "<!doctype html><html><script src=\"http://127.0.0.1:1/e\"></script><head></head><body>r</body></html>",
            "<!DOCTYPE html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"></head><body><header>h</header></body></html>",
            "\u{feff}  <!doctype html><p>hi</p>",
            "<p>no doctype at all</p>",
            "<!-- a comment first --><!doctype html><p>x</p>",
        ] {
            let out = sealed(page);
            assert!(out.starts_with(&first), "{out}");
            assert_eq!(out.matches("Content-Security-Policy").count(), 1, "{out}");
            // The page follows whole, less the doctype it opened with.
            let body = &out[first.len()..];
            assert!(
                page.contains(body.trim_start()) || page.ends_with(body),
                "{body}"
            );
        }
        assert!(CSP.contains("default-src 'none'") && CSP.contains("form-action 'none'"));
    }

    /// In a real browser, nothing on a sealed page reaches the network: the
    /// reviewer's two pages (a `<head>` in a comment, a script ahead of the
    /// head), and a fetch from a script. The same page unsealed does reach
    /// it, so the check can see a load. Opt in: `RYTER_BROWSER=1`, with
    /// Chromium on `PATH` as `chromium-browser` or named by `CHROMIUM`.
    #[test]
    #[ignore = "set RYTER_BROWSER=1; needs headless Chromium"]
    fn sealed_pages_load_nothing_in_a_real_browser() {
        use std::io::{Read, Write};
        use std::sync::{Arc, Mutex};
        if std::env::var("RYTER_BROWSER").as_deref() != Ok("1") {
            return;
        }
        let chromium = std::env::var("CHROMIUM").unwrap_or_else(|_| "chromium-browser".into());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut buf = [0u8; 1024];
                let n = stream.read(&mut buf).unwrap_or(0);
                let line = String::from_utf8_lossy(&buf[..n])
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string();
                seen.lock().unwrap().push(line);
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n");
            }
        });
        let url = |path: &str| format!("http://127.0.0.1:{port}/{path}");
        let pages = [
            (
                "comment",
                format!(
                    "<!doctype html><!-- template: <head> --><html><head></head><body><img src=\"{}\"></body></html>",
                    url("comment")
                ),
            ),
            (
                "early",
                format!(
                    "<!doctype html><html><script src=\"{}\"></script><head></head><body>report</body></html>",
                    url("early")
                ),
            ),
            (
                "fetch",
                format!(
                    "<!doctype html><html><head></head><body><script>fetch('{}')</script></body></html>",
                    url("fetch")
                ),
            ),
        ];
        let dir = tempfile::TempDir::new().unwrap();
        let run = |name: &str, html: &str| {
            let file = dir.path().join(format!("{name}.html"));
            std::fs::write(&file, html).unwrap();
            let status = std::process::Command::new(&chromium)
                .args([
                    "--headless=new",
                    "--disable-gpu",
                    "--no-sandbox",
                    "--virtual-time-budget=3000",
                ])
                .arg(format!(
                    "--user-data-dir={}",
                    dir.path().join("profile").display()
                ))
                .arg("--dump-dom")
                .arg(format!("file://{}", file.display()))
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .expect("run chromium");
            assert!(status.success(), "chromium failed on {name}");
        };
        // The control: unsealed, the image loads.
        run("control", &pages[0].1.replace("/comment", "/control"));
        for (name, html) in &pages {
            run(name, &sealed(html));
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        let hits = hits.lock().unwrap().clone();
        assert!(
            hits.iter().any(|h| h.contains("/control")),
            "the check saw no load: {hits:?}"
        );
        for (name, _) in &pages {
            assert!(
                !hits.iter().any(|h| h.contains(&format!("/{name}"))),
                "{name} loaded: {hits:?}"
            );
        }
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
