//! Root, one approved command at a time: `sudo -A` with Ryter's own askpass.
//!
//! A command Ryter runs has nobody at its keyboard, and the terminal it
//! could prompt on is the TUI's. So a `sudo` wrapper at the front of an
//! approved command's `PATH` adds `-A`, and sudo runs `ryter-askpass` (a
//! link to this binary) for the password. The helper asks the running Ryter
//! over a private socket, and Ryter asks the person. The password goes from
//! the person to sudo and nowhere else: never to the model, the session's
//! log, or disk.
//!
//! The socket answers only while a root command the person approved is
//! running ("armed"), and only to a caller holding this session's token.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Name the askpass link is called by; `main` checks `argv[0]` for it.
pub const HELPER_NAME: &str = "ryter-askpass";
/// How long a password the person chose to keep answers without asking.
pub const REMEMBER_FOR: Duration = Duration::from_secs(300);
/// sudo asking again this soon after an answer, within the same command,
/// means the answer was wrong: it is sudo's second try. A later command is
/// a new question, which a kept password answers.
const WRONG_WITHIN: Duration = Duration::from_secs(15);
const SOCK_VAR: &str = "RYTER_ASKPASS_SOCK";
const TOKEN_VAR: &str = "RYTER_ASKPASS_TOKEN";

/// What the person typed.
pub struct Typed {
    /// The password, as typed.
    pub password: String,
    /// Keep it in memory for [`REMEMBER_FOR`].
    pub remember: bool,
}

/// Where a password comes from.
pub trait PasswordSource: Send + Sync {
    /// Ask for the password sudo wants. `again`: sudo is asking right
    /// after an answer, so that one was not accepted. `None` refuses.
    fn password(&self, prompt: &str, again: bool) -> Option<Typed>;
}

#[derive(Default)]
struct Memory {
    /// A password the person chose to keep, and when it was typed.
    kept: Option<(String, Instant)>,
    /// When sudo was last given an answer, and for which root command
    /// ([`Shared::command`]).
    answered: Option<(usize, Instant)>,
}

struct Shared {
    source: Arc<dyn PasswordSource>,
    token: String,
    armed: AtomicUsize,
    /// Counts the root commands run: one more each time the socket is armed.
    command: AtomicUsize,
    closed: AtomicBool,
    memory: Mutex<Memory>,
}

impl Shared {
    /// The answer for sudo at `now`: the kept password, or what the person
    /// types.
    fn answer(&self, prompt: &str, now: Instant) -> Option<String> {
        let mut memory = self.memory.lock().ok()?;
        // Asked again right after an answer: it was wrong. Giving the kept
        // one a second time would spend another of the account's tries.
        let command = self.command.load(Ordering::SeqCst);
        let again = memory.answered.is_some_and(|(of, at)| {
            of == command && now.saturating_duration_since(at) < WRONG_WITHIN
        });
        if again {
            memory.kept = None;
        }
        if memory
            .kept
            .as_ref()
            .is_some_and(|(_, at)| now.saturating_duration_since(*at) >= REMEMBER_FOR)
        {
            memory.kept = None;
        }
        let password = match &memory.kept {
            Some((password, _)) => password.clone(),
            None => {
                let typed = self.source.password(prompt, again)?;
                if typed.remember {
                    memory.kept = Some((typed.password.clone(), now));
                }
                typed.password
            }
        };
        memory.answered = Some((command, now));
        Some(password)
    }
}

/// A running askpass endpoint.
pub struct Askpass {
    bin_dir: PathBuf,
    sock: PathBuf,
    shared: Arc<Shared>,
}

impl std::fmt::Debug for Askpass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Askpass")
            .field("sock", &self.sock)
            .finish_non_exhaustive()
    }
}

/// While alive, the socket will ask for a password.
pub struct Armed(Arc<Shared>);

impl Drop for Armed {
    fn drop(&mut self) {
        self.0.armed.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Serialize, Deserialize)]
struct Ask {
    token: String,
    prompt: String,
}

impl Askpass {
    /// Set up `bin/` under `home` (the helper link and the sudo wrapper)
    /// and start listening.
    pub fn start(home: &Path, source: Arc<dyn PasswordSource>) -> Result<Self> {
        let bin_dir = home.join("bin");
        let sudo =
            find_sudo(&bin_dir).ok_or_else(|| Error::Config("sudo isn't installed".into()))?;
        // The socket goes where the system keeps a user's own, and clears
        // at logout; without one, under the home.
        let run_dir = std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|d| !d.is_empty())
            .map(|d| PathBuf::from(d).join("ryter"))
            .unwrap_or_else(|| home.join("run"));
        Self::start_in(home, &run_dir, source, &sudo)
    }

    /// [`Self::start`] with everything under `home`, the socket included,
    /// and the sudo the wrapper runs named by the caller.
    pub fn start_with(home: &Path, source: Arc<dyn PasswordSource>, sudo: &Path) -> Result<Self> {
        Self::start_in(home, &home.join("run"), source, sudo)
    }

    fn start_in(
        home: &Path,
        run_dir: &Path,
        source: Arc<dyn PasswordSource>,
        sudo: &Path,
    ) -> Result<Self> {
        let io = |e: std::io::Error| Error::Io(e.to_string());
        let bin_dir = home.join("bin");
        fs::create_dir_all(&bin_dir).map_err(io)?;
        set_mode(&bin_dir, 0o700)?;
        let exe = std::env::current_exe().map_err(io)?;
        let link = bin_dir.join(HELPER_NAME);
        if fs::read_link(&link).ok().as_deref() != Some(exe.as_path()) {
            let _ = fs::remove_file(&link);
            std::os::unix::fs::symlink(&exe, &link).map_err(io)?;
        }
        let wrapper = bin_dir.join("sudo");
        let script = format!(
            "#!/bin/sh\n# Written by Ryter: a command it runs has no terminal to type a password on,\n# so sudo asks Ryter's askpass.\nexec '{}' -A \"$@\"\n",
            sudo.display().to_string().replace('\'', "'\\''")
        );
        if fs::read_to_string(&wrapper).ok().as_deref() != Some(script.as_str()) {
            fs::write(&wrapper, script).map_err(io)?;
        }
        set_mode(&wrapper, 0o700)?;

        fs::create_dir_all(run_dir).map_err(io)?;
        set_mode(run_dir, 0o700)?;
        clean_stale(run_dir);
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let sock = run_dir.join(format!("askpass-{}-{n}.sock", std::process::id()));
        let _ = fs::remove_file(&sock);
        let listener = UnixListener::bind(&sock)
            .map_err(|e| Error::Io(format!("the askpass socket {}: {e}", sock.display())))?;
        set_mode(&sock, 0o600)?;

        let shared = Arc::new(Shared {
            source,
            token: random_token()?,
            armed: AtomicUsize::new(0),
            command: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
            memory: Mutex::new(Memory::default()),
        });
        let listening = shared.clone();
        std::thread::Builder::new()
            .name("ryter-askpass".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    if listening.closed.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let shared = listening.clone();
                    let _ = std::thread::Builder::new()
                        .name("ryter-askpass-reply".into())
                        .spawn(move || reply(&stream, &shared));
                }
            })
            .map_err(io)?;
        Ok(Self {
            bin_dir,
            sock,
            shared,
        })
    }

    /// Allow password requests until the guard drops.
    pub fn arm(&self) -> Armed {
        self.shared.command.fetch_add(1, Ordering::SeqCst);
        self.shared.armed.fetch_add(1, Ordering::SeqCst);
        Armed(self.shared.clone())
    }

    /// Environment for a command that may run sudo, given the `PATH` it
    /// would otherwise have.
    pub fn env(&self, path: &str) -> Vec<(String, String)> {
        vec![
            ("PATH".into(), format!("{}:{path}", self.bin_dir.display())),
            (
                "SUDO_ASKPASS".into(),
                self.bin_dir.join(HELPER_NAME).display().to_string(),
            ),
            (SOCK_VAR.into(), self.sock.display().to_string()),
            (TOKEN_VAR.into(), self.shared.token.clone()),
        ]
    }

    /// Forget a kept password now.
    pub fn forget(&self) {
        if let Ok(mut memory) = self.shared.memory.lock() {
            *memory = Memory::default();
        }
    }
}

impl Drop for Askpass {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::SeqCst);
        // Wake the listener so it sees the flag, then take the socket away.
        let _ = UnixStream::connect(&self.sock);
        let _ = fs::remove_file(&self.sock);
    }
}

fn reply(stream: &UnixStream, shared: &Shared) {
    let mut line = String::new();
    if BufReader::new(stream).read_line(&mut line).is_err() {
        return;
    }
    let answer = match serde_json::from_str::<Ask>(&line) {
        Ok(ask) if ask.token == shared.token && shared.armed.load(Ordering::SeqCst) > 0 => {
            shared.answer(&ask.prompt, Instant::now())
        }
        _ => None,
    };
    let text = match answer {
        Some(password) => format!("OK {password}\n"),
        None => "NO\n".into(),
    };
    let mut out = stream;
    let _ = out.write_all(text.as_bytes());
}

/// Remove sockets left by Ryter processes that are gone.
fn clean_stale(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let pid = name
            .strip_prefix("askpass-")
            .and_then(|rest| rest.split('-').next())
            .and_then(|pid| pid.parse::<i32>().ok())
            .and_then(rustix::process::Pid::from_raw);
        if let Some(pid) = pid {
            if rustix::process::test_kill_process(pid) == Err(rustix::io::Errno::SRCH) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

/// The helper side: sudo ran `ryter-askpass "<prompt>"`. Prints the
/// password and returns 0, or returns 1.
pub fn helper_main() -> i32 {
    let prompt = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Password:".into());
    let (Ok(sock), Ok(token)) = (std::env::var(SOCK_VAR), std::env::var(TOKEN_VAR)) else {
        eprintln!("{HELPER_NAME}: not started by Ryter");
        return 1;
    };
    let Ok(mut stream) = UnixStream::connect(&sock) else {
        eprintln!("{HELPER_NAME}: Ryter isn't running");
        return 1;
    };
    let ask = serde_json::to_string(&Ask { token, prompt }).unwrap_or_default();
    if writeln!(stream, "{ask}").is_err() {
        return 1;
    }
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return 1;
    }
    match line.strip_prefix("OK ") {
        Some(password) => {
            let mut out = std::io::stdout();
            let _ = out.write_all(password.trim_end_matches('\n').as_bytes());
            let _ = out.write_all(b"\n");
            0
        }
        None => 1,
    }
}

fn find_sudo(exclude: &Path) -> Option<PathBuf> {
    let path = std::env::var("PATH").unwrap_or_default();
    path.split(':')
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .filter(|dir| dir != exclude)
        .map(|dir| dir.join("sudo"))
        .find(|p| p.is_file())
        .or_else(|| Some(PathBuf::from("/usr/bin/sudo")).filter(|p| p.is_file()))
}

fn random_token() -> Result<String> {
    use std::io::Read;
    let mut buf = [0u8; 24];
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .map_err(|e| Error::Io(e.to_string()))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|e| Error::Io(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Answers with `0`, counting how often it was asked.
    struct Fixed(Option<(&'static str, bool)>, AtomicUsize);

    impl Fixed {
        fn new(answer: Option<(&'static str, bool)>) -> Arc<Self> {
            Arc::new(Self(answer, AtomicUsize::new(0)))
        }
        fn asked(&self) -> usize {
            self.1.load(Ordering::SeqCst)
        }
    }

    impl PasswordSource for Fixed {
        fn password(&self, _prompt: &str, _again: bool) -> Option<Typed> {
            self.1.fetch_add(1, Ordering::SeqCst);
            self.0.map(|(password, remember)| Typed {
                password: password.into(),
                remember,
            })
        }
    }

    fn start(home: &Path, source: Arc<Fixed>) -> Askpass {
        Askpass::start_with(home, source, Path::new("/nowhere/sudo")).unwrap()
    }

    fn ask(ap: &Askpass, token: &str) -> String {
        let mut s = UnixStream::connect(&ap.sock).unwrap();
        let ask = Ask {
            token: token.into(),
            prompt: "pw".into(),
        };
        writeln!(s, "{}", serde_json::to_string(&ask).unwrap()).unwrap();
        let mut line = String::new();
        BufReader::new(&s).read_line(&mut line).unwrap();
        line
    }

    #[test]
    fn answers_only_when_armed_and_with_the_token() {
        let home = tempfile::tempdir().unwrap();
        let ap = start(home.path(), Fixed::new(Some(("hunter2", false))));
        let token = ap.shared.token.clone();
        assert_eq!(ask(&ap, &token), "NO\n", "not armed");
        let guard = ap.arm();
        assert_eq!(ask(&ap, "wrong"), "NO\n", "bad token");
        assert_eq!(ask(&ap, &token), "OK hunter2\n");
        drop(guard);
        assert_eq!(ask(&ap, &token), "NO\n", "disarmed");
    }

    #[test]
    fn a_refusal_is_a_no() {
        let home = tempfile::tempdir().unwrap();
        let ap = start(home.path(), Fixed::new(None));
        let _armed = ap.arm();
        assert_eq!(ask(&ap, &ap.shared.token.clone()), "NO\n");
    }

    #[test]
    fn the_socket_the_folder_and_the_wrapper_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let ap = start(home.path(), Fixed::new(None));
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&ap.sock), 0o600);
        assert_eq!(mode(&home.path().join("bin")), 0o700);
        let wrapper = fs::read_to_string(home.path().join("bin/sudo")).unwrap();
        assert!(
            wrapper.contains("exec '/nowhere/sudo' -A \"$@\""),
            "{wrapper}"
        );
        let env = ap.env("/usr/bin");
        let bin = home.path().join("bin").display().to_string();
        assert!(env.contains(&("PATH".into(), format!("{bin}:/usr/bin"))));
        assert!(env.contains(&("SUDO_ASKPASS".into(), format!("{bin}/{HELPER_NAME}"))));
    }

    #[test]
    fn the_socket_goes_with_the_askpass() {
        let home = tempfile::tempdir().unwrap();
        let ap = start(home.path(), Fixed::new(None));
        let sock = ap.sock.clone();
        assert!(sock.exists());
        drop(ap);
        assert!(!sock.exists());
    }

    fn shared(source: Arc<Fixed>) -> Shared {
        Shared {
            source,
            token: String::new(),
            armed: AtomicUsize::new(1),
            command: AtomicUsize::new(1),
            closed: AtomicBool::new(false),
            memory: Mutex::new(Memory::default()),
        }
    }

    #[test]
    fn a_kept_password_answers_until_it_is_asked_for_again_at_once() {
        let source = Fixed::new(Some(("hunter2", true)));
        let shared = shared(source.clone());
        let start = Instant::now();
        assert_eq!(shared.answer("pw", start).as_deref(), Some("hunter2"));
        assert_eq!(source.asked(), 1);
        // sudo back within seconds: the answer was wrong, so the person is
        // asked, not given the same one again.
        let soon = start + Duration::from_secs(2);
        assert_eq!(shared.answer("pw", soon).as_deref(), Some("hunter2"));
        assert_eq!(source.asked(), 2);
        // Later in the same command: answered from memory.
        let later = soon + WRONG_WITHIN;
        assert_eq!(shared.answer("pw", later).as_deref(), Some("hunter2"));
        assert_eq!(source.asked(), 2);
        // The next root command, a moment on: a new question, not a second
        // try. Read as one, a quick `sudo ldconfig` after an install said
        // "sudo did not accept that one" and asked for the password again.
        shared.command.fetch_add(1, Ordering::SeqCst);
        let next = later + Duration::from_secs(1);
        assert_eq!(shared.answer("pw", next).as_deref(), Some("hunter2"));
        assert_eq!(source.asked(), 2);
        // Past its time: asked again.
        let stale = soon + REMEMBER_FOR;
        assert_eq!(shared.answer("pw", stale).as_deref(), Some("hunter2"));
        assert_eq!(source.asked(), 3);
    }

    #[test]
    fn a_password_is_kept_only_when_the_person_says_so() {
        let source = Fixed::new(Some(("hunter2", false)));
        let shared = shared(source.clone());
        let start = Instant::now();
        shared.answer("pw", start);
        shared.answer("pw", start + WRONG_WITHIN);
        assert_eq!(source.asked(), 2);
    }
}
