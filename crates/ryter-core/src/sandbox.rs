//! Landlock profiles. Off by default; a requested profile fail-closes if the kernel cannot enforce it.
//!
//! What a sandboxed tool thread can reach, besides the project:
//!
//! - **System folders,** to read and run: `/usr`, `/bin`, `/etc`, and the
//!   places package managers install to (`/opt`, `/nix`, `/snap`, Homebrew).
//! - **A few devices,** to read and write: `/dev/null` and its kin. `git`
//!   opens `/dev/null` for writing before it does anything, and every
//!   `> /dev/null` in a script does the same.
//! - **The user's tools,** to read and run: toolchains installed under the
//!   home folder (`~/.cargo/bin`, `~/.rustup`, node managers) and whatever
//!   else is on `PATH` there. Without them most projects can't build.
//! - **The tools' download caches,** to write: a build that fetches a
//!   dependency writes it there.
//! - **Part of Ryter's own folder:** its scratch folder, logs, sessions,
//!   pages and crew worktrees to write; skills and the rules file to read.
//!
//! Never the rest of the home folder, `~/.ssh`, the tools' saved logins
//! (`~/.cargo/credentials.toml`, `~/.npmrc`), Ryter's keys, or `/tmp`.
//! Landlock has no "all but this" rule, so each is a list of what is
//! granted, never a parent with exceptions.
//!
//! The sandbox is the filesystem only. It doesn't limit the network, and a
//! command that can reach the Docker socket can reach the whole machine.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use crate::error::{Error, Result};

/// Filesystem sandbox profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxProfile {
    /// No Landlock. Default.
    Off,
    /// Workspace + `~/.ryter` writable; the rest of the tree is unreachable except a small read set.
    Workspace,
    /// Like [`Self::Workspace`] but the project tree is read-only.
    ReadOnly,
}

impl SandboxProfile {
    /// Config / flag spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Workspace => "workspace",
            Self::ReadOnly => "read-only",
        }
    }
}

impl std::fmt::Display for SandboxProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SandboxProfile {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "false" => Ok(Self::Off),
            "workspace" | "ws" => Ok(Self::Workspace),
            "read-only" | "readonly" | "ro" => Ok(Self::ReadOnly),
            other => Err(Error::Config(format!(
                "unknown sandbox {other:?} (off|workspace|read-only)"
            ))),
        }
    }
}

/// Probe whether this kernel can enforce Landlock. Never applies a ruleset.
pub fn probe() -> String {
    #[cfg(not(target_os = "linux"))]
    {
        "not linux".into()
    }
    #[cfg(target_os = "linux")]
    {
        probe_linux()
    }
}

thread_local! {
    static ACTIVE: std::cell::Cell<SandboxProfile> = const { std::cell::Cell::new(SandboxProfile::Off) };
    static SCRATCH: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// The profile [`apply`] put on this thread, `Off` when none. Landlock binds
/// the thread and everything it starts, so a browser opened from here would
/// run inside the sandbox.
pub fn active() -> SandboxProfile {
    ACTIVE.with(|a| a.get())
}

/// Tokio runtime: current-thread when sandboxed so Landlock covers tool calls.
pub fn runtime(profile: SandboxProfile) -> std::io::Result<tokio::runtime::Runtime> {
    if profile == SandboxProfile::Off {
        tokio::runtime::Runtime::new()
    } else {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
    }
}

/// The user's own machine, as far as a profile needs it: where their home
/// folder is, and their `PATH`. Read from the environment by [`apply`];
/// given by hand in tests, which must not depend on the machine.
#[derive(Debug, Clone, Default)]
pub struct Machine {
    /// The user's home folder, when there is one.
    pub user_home: Option<PathBuf>,
    /// `PATH`, as the shell has it.
    pub path: String,
}

impl Machine {
    /// This process's.
    pub fn here() -> Self {
        Self {
            user_home: dirs::home_dir(),
            path: std::env::var("PATH").unwrap_or_default(),
        }
    }
}

/// Under the home folder, what a sandboxed command may read and run, when
/// it is there: where toolchains install themselves. Each is a folder of
/// programs and their settings. None holds a saved login.
const TOOL_HOMES: &[&str] = &[
    ".cargo/bin",
    ".cargo/env",
    ".cargo/config",
    ".cargo/config.toml",
    ".rustup",
    ".nvm",
    ".volta",
    ".fnm",
    ".local/share/fnm",
    ".asdf",
    ".local/share/mise",
    ".config/mise",
    ".pyenv",
    ".rbenv",
    ".sdkman",
    ".deno",
    ".bun",
    "go/bin",
    ".local/bin",
    ".local/lib",
    ".local/pipx",
    ".local/share/pipx",
    ".local/share/uv",
    ".tool-versions",
    // `git` needs to know who is committing. The settings files by name:
    // `~/.config/git/credentials` is where git can keep saved logins.
    ".gitconfig",
    ".config/git/config",
    ".config/git/ignore",
    ".config/git/attributes",
];

/// Under the home folder, what a sandboxed command may also write, when it
/// is there: where package managers keep what they download. A build that
/// needs a new dependency fails without these.
const TOOL_CACHES: &[&str] = &[
    ".cargo/registry",
    ".cargo/git",
    ".cargo/.package-cache",
    ".cargo/.package-cache-mutate",
    ".cargo/.global-cache",
    ".npm",
    ".cache/pip",
    ".cache/uv",
    ".cache/go-build",
    "go/pkg",
    ".cache/yarn",
    ".cache/pnpm",
    ".local/share/pnpm",
    ".bun/install/cache",
    ".cache/deno",
    ".cache/node",
    ".cache/typescript",
    ".gradle/caches",
    ".m2/repository",
    ".cache/sccache",
    ".cache/pre-commit",
];

/// System folders to read and run from, beyond the standard ones.
const SYSTEM_READ: &[&str] = &[
    "/usr",
    "/bin",
    "/sbin",
    "/lib",
    "/lib64",
    "/etc",
    "/proc",
    "/sys",
    "/dev",
    "/opt",
    "/nix",
    "/snap",
    "/var/lib/snapd",
    "/home/linuxbrew/.linuxbrew",
];

/// Devices to read and write. `git` opens `/dev/null` for writing as it
/// starts, so with `/dev` read-only it could not run at all.
const DEVICES: &[&str] = &[
    "/dev/null",
    "/dev/zero",
    "/dev/full",
    "/dev/random",
    "/dev/urandom",
    // New terminals, for tools and test suites that open one. Not
    // `/dev/tty`: that is the user's own terminal, the one Ryter is drawn on.
    "/dev/ptmx",
    "/dev/pts",
];

/// The user's tools: what a sandboxed command may read and run under their
/// home folder (`.0`), and what it may also write there (`.1`). Only what
/// exists. Folders on `PATH` under the home folder are read too, each as
/// itself: never its parent, which may hold anything.
pub fn tool_reach(machine: &Machine) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let Some(home) = machine.user_home.as_deref() else {
        return (Vec::new(), Vec::new());
    };
    // What is there, and is itself and not a link. A grant on a link is a
    // grant on what it points at: `~/.cache/pip` linked to `~/.ssh` would
    // open the keys. A linked tool folder is left out.
    let real = |p: &Path| {
        p.symlink_metadata()
            .is_ok_and(|m| !m.file_type().is_symlink())
    };
    let under = |list: &[&str]| -> Vec<PathBuf> {
        list.iter()
            .map(|rel| home.join(rel))
            .filter(|p| real(p))
            .collect()
    };
    let mut read = under(TOOL_HOMES);
    for dir in std::env::split_paths(&machine.path) {
        // The home folder itself on `PATH` would be the whole of it.
        if dir.starts_with(home)
            && dir != home
            && real(&dir)
            && dir.is_dir()
            && !read.contains(&dir)
        {
            read.push(dir);
        }
    }
    (read, under(TOOL_CACHES))
}

/// Apply `profile` to the calling thread. [`SandboxProfile::Off`] is a no-op.
///
/// A non-off profile on a kernel without Landlock returns an error (fail closed).
/// Call this on the thread that will run tools; pair it with a current-thread
/// tokio runtime so work-stealing threads are not left unrestricted.
pub fn apply(profile: SandboxProfile, workspace: &Path, home: &Path) -> Result<()> {
    apply_on(profile, workspace, home, &Machine::here())
}

/// [`apply`], on a machine described by hand.
pub fn apply_on(
    profile: SandboxProfile,
    workspace: &Path,
    home: &Path,
    machine: &Machine,
) -> Result<()> {
    if profile == SandboxProfile::Off {
        return Ok(());
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (workspace, home, machine);
        Err(Error::Config(format!(
            "sandbox {profile} requested but Landlock is Linux-only"
        )))
    }
    #[cfg(target_os = "linux")]
    {
        apply_linux(profile, workspace, home, machine)?;
        ACTIVE.with(|a| a.set(profile));
        SCRATCH.with(|s| *s.borrow_mut() = Some(canonicalize_or(home).join("tmp")));
        Ok(())
    }
}

/// Where a command run from a sandbox should keep its temporary files, or
/// `None` outside one. `/tmp` isn't granted: other projects, and other
/// programs' sockets, live there. `mktemp` and every tool that asks for a
/// temporary file failed until `TMPDIR` pointed here.
///
/// The thread that applied the profile knows its scratch folder. A thread
/// it started is in the same sandbox without knowing, so it is recognised
/// by what the sandbox does: the temporary folder can't be listed.
pub fn scratch() -> Option<PathBuf> {
    if let Some(dir) = SCRATCH.with(|s| s.borrow().clone()) {
        return Some(dir);
    }
    if cfg!(target_os = "linux") && std::fs::read_dir(std::env::temp_dir()).is_err() {
        return Some(crate::config::home_dir().join("tmp"));
    }
    None
}

#[cfg(target_os = "linux")]
fn probe_linux() -> String {
    use landlock::{ABI, Access, AccessFs, CompatLevel, Compatible, Ruleset, RulesetAttr};
    match Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::from_all(ABI::V1))
    {
        Ok(_) => "available".into(),
        Err(e) => format!("unavailable ({e})"),
    }
}

#[cfg(target_os = "linux")]
fn apply_linux(
    profile: SandboxProfile,
    workspace: &Path,
    home: &Path,
    machine: &Machine,
) -> Result<()> {
    use landlock::{
        ABI, Access, AccessFs, CompatLevel, Compatible, Ruleset, RulesetAttr, RulesetCreatedAttr,
        RulesetStatus, path_beneath_rules,
    };

    let abi = ABI::V1;
    // Moving or linking a file from one folder to another is its own right
    // (`Refer`), added to Landlock after its first version. A rule set that
    // doesn't name it refuses every such move, as "Invalid cross-device
    // link": `rustc` puts a library's metadata in place that way, and so
    // does every package manager. So it is named, and granted with the rest
    // wherever a command may write. A kernel without it keeps the first
    // version's behaviour; the sandbox still holds there.
    let created = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::from_all(abi))
        .map_err(|e| {
            Error::Config(format!(
                "sandbox {profile} requested but Landlock is unavailable: {e}"
            ))
        })?
        .set_compatibility(CompatLevel::BestEffort)
        .handle_access(AccessFs::Refer)
        .map_err(|e| {
            Error::Config(format!(
                "sandbox {profile} requested but Landlock is unavailable: {e}"
            ))
        })?
        .create()
        .map_err(|e| {
            Error::Config(format!(
                "sandbox {profile} requested but Landlock is unavailable: {e}"
            ))
        })?;

    let ws = canonicalize_or(workspace);
    let home = canonicalize_or(home);
    // Everything a folder that may be written allows, moves included.
    let all = AccessFs::from_all(abi) | AccessFs::Refer;
    let ws_access = match profile {
        SandboxProfile::ReadOnly => AccessFs::from_read(abi),
        SandboxProfile::Workspace | SandboxProfile::Off => all,
    };
    // Do not allow `/tmp` itself: TempDir and other projects live there.
    // Scratch is `~/.ryter/tmp` (or `$RYTER_HOME/tmp`), and `TMPDIR` points
    // commands at it ([`scratch`]).
    //
    // Granting all of `~/.ryter` used to hand tools read/write on
    // `~/.ryter/keys/<connection>`, so even the `read-only` profile let a
    // builder's bash read every API key. Landlock has no negative rules, so the
    // writable set is enumerated instead. Keys are resolved before `apply` runs,
    // so nothing here needs them.
    let (tools, caches) = tool_reach(machine);
    let mut read = existing(SYSTEM_READ);
    read.extend(readable_set(&home));
    read.extend(tools);
    let mut write = writable_set(&home);
    write.extend(caches);
    write.extend(existing(DEVICES));
    // A rule on a file can carry only the rights a file has: the rest are
    // for folders, and asking for them on a file is refused.
    let (read_dirs, read_files) = by_kind(read);
    let (write_dirs, write_files) = by_kind(write);
    let file_read = AccessFs::ReadFile | AccessFs::Execute;
    let file_write = file_read | AccessFs::WriteFile;
    let rules = |e: landlock::RulesetError| Error::Config(format!("sandbox rules: {e}"));
    let status = created
        .set_compatibility(CompatLevel::BestEffort)
        .add_rules(path_beneath_rules(&read_dirs, AccessFs::from_read(abi)))
        .map_err(rules)?
        .add_rules(path_beneath_rules(&read_files, file_read))
        .map_err(rules)?
        .add_rules(path_beneath_rules(&write_dirs, all))
        .map_err(rules)?
        .add_rules(path_beneath_rules(&write_files, file_write))
        .map_err(rules)?
        .add_rules(path_beneath_rules(&[ws], ws_access))
        .map_err(rules)?
        .restrict_self()
        .map_err(|e| Error::Config(format!("sandbox restrict: {e}")))?;

    if status.ruleset == RulesetStatus::NotEnforced {
        return Err(Error::Config(format!(
            "sandbox {profile} requested but Landlock was not enforced"
        )));
    }
    // The keys are in this process: in its memory, and in its environment
    // when they came from there. `/proc` is readable in the sandbox, and a
    // command could read both from `/proc/<this process>`. A process that
    // isn't dumpable keeps those from every other process of the same user.
    rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)
        .map_err(|e| {
            Error::Config(format!(
                "sandbox {profile}: could not close this process to others: {e}"
            ))
        })?;
    Ok(())
}

/// Folders and files apart.
#[cfg(target_os = "linux")]
fn by_kind(paths: Vec<PathBuf>) -> (Vec<PathBuf>, Vec<PathBuf>) {
    paths.into_iter().partition(|p| p.is_dir())
}

/// Directories a sandboxed thread may write, created if missing.
///
/// Enumerated rather than granting `~/.ryter` wholesale: Landlock has no
/// negative rules, so any grant on the parent would re-expose `keys/`.
#[cfg(target_os = "linux")]
fn writable_set(home: &Path) -> Vec<std::path::PathBuf> {
    // `worktrees` is where a crew's builders work. Without it, crew mode
    // could not build anything under a sandbox.
    made(home, &["tmp", "logs", "sessions", "pages", "worktrees"])
}

/// What a sandboxed thread may read in `~/.ryter`, beyond what it writes:
/// the user's skills, which the model loads, and their rules file, which
/// goes in every prompt. Read only: a rules file the sandboxed shell could
/// write would change without the user being asked.
#[cfg(target_os = "linux")]
fn readable_set(home: &Path) -> Vec<std::path::PathBuf> {
    let mut set = made(home, &["skills"]);
    // The file itself, and only a real one. A link there would be followed,
    // and one pointing at `keys/<connection>` would hand the key to the
    // sandbox. `home` is resolved; the name is joined on and not resolved.
    let rules = canonicalize_or(home).join(crate::rules::FILE);
    if rules
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_file())
    {
        set.push(rules);
    }
    set
}

#[cfg(target_os = "linux")]
fn made(home: &Path, subs: &[&str]) -> Vec<std::path::PathBuf> {
    subs.iter()
        .map(|sub| {
            let p = home.join(sub);
            let _ = std::fs::create_dir_all(&p);
            canonicalize_or(&p)
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn canonicalize_or(p: &Path) -> std::path::PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

#[cfg(target_os = "linux")]
fn existing(paths: &[&str]) -> Vec<std::path::PathBuf> {
    paths
        .iter()
        .filter(|p| Path::new(p).exists())
        .map(std::path::PathBuf::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The writable set must not include the plaintext key store.
    #[cfg(target_os = "linux")]
    #[test]
    fn sandbox_does_not_grant_the_key_store() {
        use tempfile::TempDir;
        let home = TempDir::new().unwrap();
        let ws = TempDir::new().unwrap();
        std::fs::create_dir_all(home.path().join("keys")).unwrap();
        std::fs::write(home.path().join("keys/spacexai"), "xai-secret").unwrap();
        // Landlock is applied to the calling thread and cannot be undone, so
        // this asserts the rule set rather than applying it.
        let rw = writable_set(home.path());
        assert!(
            !rw.iter().any(|p| p.ends_with("keys")),
            "keys must never be writable: {rw:?}"
        );
        assert!(
            !rw.iter().any(|p| p == home.path()),
            "granting all of ~/.ryter re-exposes keys: {rw:?}"
        );
        assert!(rw.iter().any(|p| p.ends_with("sessions")), "{rw:?}");
        assert!(rw.iter().any(|p| p.ends_with("tmp")), "{rw:?}");
        assert!(rw.iter().any(|p| p.ends_with("pages")), "{rw:?}");
        let ro = readable_set(home.path());
        assert!(
            ro.iter().all(|p| p.ends_with("skills")),
            "only skills is read beyond the writable set: {ro:?}"
        );
        // With a rules file, that one file too, and never the keys.
        crate::rules::save(home.path(), "- a rule").unwrap();
        let ro = readable_set(home.path());
        assert!(
            ro.iter().any(|p| p.ends_with("RYTER.md")) && !ro.iter().any(|p| p.ends_with("keys")),
            "{ro:?}"
        );
        // A rules file that is a link is never followed: one pointing at a
        // key would put the key in the sandbox's reach.
        std::fs::create_dir_all(home.path().join("keys")).unwrap();
        std::fs::write(home.path().join("keys/openrouter"), "sk-secret").unwrap();
        std::fs::remove_file(crate::rules::path(home.path())).unwrap();
        std::os::unix::fs::symlink(
            home.path().join("keys/openrouter"),
            crate::rules::path(home.path()),
        )
        .unwrap();
        let ro = readable_set(home.path());
        assert!(
            ro.iter().all(|p| p.ends_with("skills")),
            "a linked rules file adds nothing: {ro:?}"
        );
        let _ = ws;
    }

    #[test]
    fn parse_profiles() {
        assert_eq!(
            SandboxProfile::from_str("off").unwrap(),
            SandboxProfile::Off
        );
        assert_eq!(
            SandboxProfile::from_str("workspace").unwrap(),
            SandboxProfile::Workspace
        );
        assert_eq!(
            SandboxProfile::from_str("read-only").unwrap(),
            SandboxProfile::ReadOnly
        );
        assert!(SandboxProfile::from_str("landlock").is_err());
    }

    #[test]
    fn off_is_noop() {
        apply(SandboxProfile::Off, Path::new("/tmp"), Path::new("/tmp")).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn workspace_denies_paths_outside_on_this_thread() {
        use tempfile::TempDir;
        let ws = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        std::fs::write(ws.path().join("in.txt"), "inside").unwrap();
        std::fs::write(outside.path().join("secret.txt"), "nope").unwrap();
        let ws_p = ws.path().to_path_buf();
        let home_p = home.path().to_path_buf();
        let in_p = ws.path().join("in.txt");
        let secret = outside.path().join("secret.txt");
        let handle = std::thread::spawn(move || {
            if let Err(e) = apply(SandboxProfile::Workspace, &ws_p, &home_p) {
                // Kernel without Landlock: fail-closed is the product; skip the deny check.
                eprintln!("sandbox apply skipped: {e}");
                return;
            }
            assert_eq!(std::fs::read_to_string(&in_p).unwrap(), "inside");
            assert!(
                std::fs::read_to_string(&secret).is_err(),
                "outside path should be denied"
            );
        });
        handle.join().expect("sandbox thread");
        // Sibling thread is unrestricted.
        assert_eq!(
            std::fs::read_to_string(outside.path().join("secret.txt")).unwrap(),
            "nope"
        );
    }

    /// Pages and skills work under the sandbox, and keys stay shut: the
    /// sandbox used to stop `show_page` writing and `load_skill` reading
    /// the user's own skills.
    #[cfg(target_os = "linux")]
    #[test]
    fn pages_and_skills_work_under_the_sandbox() {
        use tempfile::TempDir;
        let ws = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        std::fs::create_dir_all(home.path().join("skills/mine")).unwrap();
        std::fs::write(home.path().join("skills/mine/SKILL.md"), "body").unwrap();
        std::fs::create_dir_all(home.path().join("keys")).unwrap();
        std::fs::write(home.path().join("keys/spacexai"), "xai-secret").unwrap();
        crate::rules::save(home.path(), "- a rule").unwrap();
        let (ws_p, home_p) = (ws.path().to_path_buf(), home.path().to_path_buf());
        let handle = std::thread::spawn(move || {
            if let Err(e) = apply(SandboxProfile::ReadOnly, &ws_p, &home_p) {
                eprintln!("sandbox apply skipped: {e}");
                return;
            }
            assert_eq!(active(), SandboxProfile::ReadOnly);
            // The user's rules are read for every prompt, and can't be
            // written from in here, by Ryter or by a shell command.
            assert_eq!(crate::rules::load(&home_p).as_deref(), Some("- a rule"));
            assert!(crate::rules::save(&home_p, "- another").is_err());
            assert!(std::fs::write(crate::rules::path(&home_p), "x").is_err());
            let page = home_p.join("pages/s1/report.html");
            std::fs::create_dir_all(page.parent().unwrap()).unwrap();
            std::fs::write(&page, "<p>x</p>").unwrap();
            assert_eq!(
                std::fs::read_to_string(home_p.join("skills/mine/SKILL.md")).unwrap(),
                "body"
            );
            assert!(std::fs::write(home_p.join("skills/mine/x"), "y").is_err());
            assert!(std::fs::read_to_string(home_p.join("keys/spacexai")).is_err());
        });
        handle.join().expect("sandbox thread");
        assert_eq!(active(), SandboxProfile::Off, "only the sandboxed thread");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn read_only_blocks_writes_in_workspace() {
        use tempfile::TempDir;
        let ws = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        std::fs::write(ws.path().join("a.txt"), "x").unwrap();
        let ws_p = ws.path().to_path_buf();
        let home_p = home.path().to_path_buf();
        let target = ws.path().join("nope.txt");
        let handle = std::thread::spawn(move || {
            if let Err(e) = apply(SandboxProfile::ReadOnly, &ws_p, &home_p) {
                eprintln!("sandbox apply skipped: {e}");
                return;
            }
            assert!(std::fs::write(&target, "y").is_err());
        });
        handle.join().expect("sandbox thread");
    }

    /// A real toolchain works in the sandbox, and what sits beside it stays
    /// shut. Before this, on a real machine under `workspace`: `git` could
    /// not start (`/dev/null` wasn't writable), `cargo` and `rustc` under
    /// `~/.cargo` were refused, nothing could make a temporary file, and a
    /// crew's builders could not write in their worktrees.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_real_toolchain_works_in_the_sandbox() {
        use std::os::unix::fs::PermissionsExt;
        use tempfile::TempDir;
        let ws = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        let user = TempDir::new().unwrap();
        let u = user.path().to_path_buf();
        let file = |rel: &str, text: &str| {
            let p = u.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, text).unwrap();
            p
        };
        let tool = |rel: &str| {
            let p = file(rel, "#!/bin/sh\necho ran\n");
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };
        // A toolchain, a tool on PATH, a cache, and what must stay shut.
        let cargo = tool(".cargo/bin/cargo");
        let mine = tool("bin/mytool");
        file(".cargo/registry/index/x", "cached");
        file(".cargo/credentials.toml", "token = \"secret\"");
        file(".npmrc", "//registry/:_authToken=secret");
        file(".ssh/id_ed25519", "private key");
        file("notes/diary.txt", "private");
        file(".gitconfig", "[user]\nname = Me\n");
        file(".config/git/config", "[core]\n");
        file(".config/git/credentials", "https://me:token@host");
        // A cache planted as a link to the keys must not open them.
        std::fs::create_dir_all(u.join(".cache")).unwrap();
        std::os::unix::fs::symlink(u.join(".ssh"), u.join(".cache/pip")).unwrap();
        std::fs::create_dir_all(home.path().join("keys")).unwrap();
        std::fs::write(home.path().join("keys/openrouter"), "sk-secret").unwrap();
        let machine = Machine {
            user_home: Some(u.clone()),
            path: format!("/usr/bin:{}:{}", u.join("bin").display(), u.display()),
        };
        let (ws_p, home_p) = (ws.path().to_path_buf(), home.path().to_path_buf());
        std::thread::spawn(move || {
            if let Err(e) = apply_on(SandboxProfile::Workspace, &ws_p, &home_p, &machine) {
                eprintln!("sandbox apply skipped: {e}");
                return;
            }
            let run = |prog: &Path| {
                std::process::Command::new(prog)
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            };
            let read = |rel: &str| std::fs::read_to_string(u.join(rel));
            // The tools run.
            assert_eq!(run(&cargo).unwrap(), "ran");
            assert_eq!(run(&mine).unwrap(), "ran");
            assert!(read(".gitconfig").is_ok());
            assert!(read(".config/git/config").is_ok());
            // Their cache is written; the programs themselves are not.
            std::fs::write(u.join(".cargo/registry/index/y"), "new").unwrap();
            assert!(std::fs::write(u.join(".cargo/bin/cargo"), "swapped").is_err());
            assert!(std::fs::write(u.join("bin/mytool"), "swapped").is_err());
            // Saved logins beside them, and the rest of the home folder, stay shut:
            // the home folder on PATH did not open it.
            for shut in [
                ".cargo/credentials.toml",
                ".npmrc",
                ".ssh/id_ed25519",
                ".cache/pip/id_ed25519",
                ".config/git/credentials",
                "notes/diary.txt",
            ] {
                assert!(read(shut).is_err(), "{shut} is readable");
            }
            assert!(std::fs::read_to_string(home_p.join("keys/openrouter")).is_err());
            // `/dev/null` takes writes, as `git` and every `> /dev/null` need.
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/null")
                .unwrap();
            let sh = std::process::Command::new("sh")
                .args(["-c", "echo x > /dev/null && echo fine"])
                .output()
                .unwrap();
            assert_eq!(String::from_utf8_lossy(&sh.stdout).trim(), "fine");
            // Ryter's own memory and environment, where the keys are, are
            // shut to the commands it runs.
            let own = std::process::Command::new("sh")
                .args([
                    "-c",
                    "cat /proc/$PPID/environ >/dev/null 2>&1 && echo open || echo shut",
                ])
                .output()
                .unwrap();
            assert_eq!(String::from_utf8_lossy(&own.stdout).trim(), "shut");
            // A file moves between two folders of the project, as `rustc`
            // moves a library's metadata into place. Under a rule set that
            // doesn't name that right, it failed: "Invalid cross-device link".
            std::fs::create_dir_all(ws_p.join("target/tmp")).unwrap();
            std::fs::create_dir_all(ws_p.join("target/deps")).unwrap();
            std::fs::write(ws_p.join("target/tmp/lib.rmeta"), "meta").unwrap();
            std::fs::rename(
                ws_p.join("target/tmp/lib.rmeta"),
                ws_p.join("target/deps/lib.rmeta"),
            )
            .unwrap();
            std::fs::hard_link(
                ws_p.join("target/deps/lib.rmeta"),
                ws_p.join("target/tmp/again.rmeta"),
            )
            .unwrap();
            // Out of the scratch folder into the project too.
            std::fs::create_dir_all(home_p.join("tmp")).unwrap();
            std::fs::write(home_p.join("tmp/made"), "x").unwrap();
            std::fs::rename(home_p.join("tmp/made"), ws_p.join("made")).unwrap();
            // But nothing moves into a folder that is only read.
            assert!(std::fs::rename(ws_p.join("made"), u.join("bin/made")).is_err());
            // A crew's worktrees are written, and so is the project.
            let wt = home_p.join("worktrees/sess/t1");
            std::fs::create_dir_all(&wt).unwrap();
            std::fs::write(wt.join("a.txt"), "built").unwrap();
            std::fs::write(ws_p.join("a.txt"), "edited").unwrap();
            // Temporary files go to Ryter's scratch folder, through the shell tool.
            assert_eq!(scratch().as_deref(), Some(home_p.join("tmp").as_path()));
            let cancel = crate::cancel::Cancel::new();
            let made = crate::tools::shell::run_command(
                "f=$(mktemp) && echo x > \"$f\" && dirname \"$f\"",
                &ws_p,
                std::time::Duration::from_secs(20),
                &cancel,
            )
            .unwrap();
            match made {
                crate::tools::shell::Run::Ok(out) => {
                    assert!(
                        out.contains(&home_p.join("tmp").display().to_string()),
                        "{out}"
                    );
                }
                other => panic!("mktemp failed in the sandbox: {other:?}"),
            }
            assert!(std::fs::read_dir("/tmp").is_err(), "/tmp is open");
        })
        .join()
        .expect("sandbox thread");
        assert_eq!(scratch(), None, "only inside a sandbox");
    }

    /// The tools a profile reaches are the ones that exist, each as itself.
    #[test]
    fn the_tools_reached_are_the_ones_that_are_there() {
        use tempfile::TempDir;
        let user = TempDir::new().unwrap();
        let u = user.path();
        for dir in [".cargo/bin", ".cargo/registry", ".npm", "bin", "notes"] {
            std::fs::create_dir_all(u.join(dir)).unwrap();
        }
        // Links are left out: a cache, a toolchain, and a folder on PATH.
        #[cfg(unix)]
        for (link, to) in [
            (".cache/uv", "notes"),
            (".bun", "notes"),
            ("linked", "notes"),
        ] {
            std::fs::create_dir_all(u.join(link).parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(u.join(to), u.join(link)).unwrap();
        }
        let machine = Machine {
            user_home: Some(u.to_path_buf()),
            path: format!(
                "/usr/bin:{}:{}:{}:{}",
                u.join("bin").display(),
                u.display(),
                u.join("missing").display(),
                u.join("linked").display()
            ),
        };
        let (read, write) = tool_reach(&machine);
        assert_eq!(read, [u.join(".cargo/bin"), u.join("bin")]);
        assert_eq!(write, [u.join(".cargo/registry"), u.join(".npm")]);
        // Nobody's home folder: nothing of it.
        let nobody = Machine {
            user_home: None,
            path: "/usr/bin".into(),
        };
        assert_eq!(tool_reach(&nobody), (Vec::new(), Vec::new()));
    }
}
