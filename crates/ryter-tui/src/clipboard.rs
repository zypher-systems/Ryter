//! Putting text on the clipboard, two ways at once.
//!
//! The terminal is asked with OSC 52, which needs nothing installed and
//! reaches the clipboard of the machine the terminal is on, across `ssh`
//! too. Not every terminal answers it, so the desktop's own tool is run as
//! well when there is one: `wl-copy`, `xclip` or `xsel`, `pbcopy`.

use std::io::Write;
use std::process::{Command, Stdio};

use base64::Engine;

/// The longest text sent by OSC 52: terminals drop a longer sequence, some
/// without a word.
const MOST_OSC52: usize = 74_000;

/// The escape sequence that asks a terminal to put `text` on its clipboard.
pub fn osc52(text: &str) -> String {
    let data = base64::engine::general_purpose::STANDARD.encode(text);
    format!("\x1b]52;c;{data}\x07")
}

/// The desktop's own tool for this session, and its arguments. `has` says
/// whether a program is installed.
fn tool(
    macos: bool,
    wayland: bool,
    x11: bool,
    has: impl Fn(&str) -> bool,
) -> Option<(&'static str, &'static [&'static str])> {
    const TOOLS: &[(&str, &[&str], u8)] = &[
        ("pbcopy", &[], 0),
        ("wl-copy", &[], 1),
        ("xclip", &["-selection", "clipboard"], 2),
        ("xsel", &["--clipboard", "--input"], 2),
    ];
    TOOLS
        .iter()
        .find(|(name, _, kind)| {
            let fits = match kind {
                0 => macos,
                1 => wayland,
                _ => x11,
            };
            fits && has(name)
        })
        .map(|(name, args, _)| (*name, *args))
}

fn installed(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
}

/// Put `text` on the clipboard: through the terminal `out` is, and through
/// the desktop's tool when one is installed. The tool is fed on a thread of
/// its own, so a slow one never holds the screen.
pub fn copy(out: &mut impl Write, text: &str) -> std::io::Result<()> {
    let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
    if let Some((name, args)) = tool(
        cfg!(target_os = "macos"),
        set("WAYLAND_DISPLAY"),
        set("DISPLAY"),
        installed,
    ) {
        let text = text.to_string();
        let _ = std::thread::Builder::new()
            .name("ryter-clipboard".into())
            .spawn(move || {
                let child = Command::new(name)
                    .args(args)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
                if let Ok(mut child) = child {
                    if let Some(mut stdin) = child.stdin.take() {
                        let _ = stdin.write_all(text.as_bytes());
                    }
                    let _ = child.wait();
                }
            });
    }
    if text.len() <= MOST_OSC52 {
        out.write_all(osc52(text).as_bytes())?;
        out.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_terminal_is_asked_in_base64() {
        assert_eq!(osc52("hi"), "\x1b]52;c;aGk=\x07");
        assert_eq!(osc52("é\n"), "\x1b]52;c;w6kK\x07");
    }

    /// The tool is the session's own: Wayland's before X's when both are
    /// set, and none where it isn't installed.
    #[test]
    fn the_desktops_tool_follows_the_session() {
        let all = |_: &str| true;
        assert_eq!(tool(true, false, false, all).map(|t| t.0), Some("pbcopy"));
        assert_eq!(tool(false, true, true, all).map(|t| t.0), Some("wl-copy"));
        assert_eq!(tool(false, false, true, all).map(|t| t.0), Some("xclip"));
        let only_xsel = |n: &str| n == "xsel";
        assert_eq!(
            tool(false, true, true, only_xsel),
            Some(("xsel", &["--clipboard", "--input"][..]))
        );
        assert_eq!(tool(false, true, true, |_| false), None);
        assert_eq!(tool(false, false, false, all), None);
    }
}
