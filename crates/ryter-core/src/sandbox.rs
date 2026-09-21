//! Landlock profiles. Off by default; a requested profile fail-closes if the kernel cannot enforce it.

use std::path::Path;
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

/// Apply `profile` to the calling thread. [`SandboxProfile::Off`] is a no-op.
///
/// A non-off profile on a kernel without Landlock returns an error (fail closed).
/// Call this on the thread that will run tools; pair it with a current-thread
/// tokio runtime so work-stealing threads are not left unrestricted.
pub fn apply(profile: SandboxProfile, workspace: &Path, home: &Path) -> Result<()> {
    if profile == SandboxProfile::Off {
        return Ok(());
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (workspace, home);
        return Err(Error::Config(format!(
            "sandbox {profile} requested but Landlock is Linux-only"
        )));
    }
    #[cfg(target_os = "linux")]
    {
        apply_linux(profile, workspace, home)
    }
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
fn apply_linux(profile: SandboxProfile, workspace: &Path, home: &Path) -> Result<()> {
    use landlock::{
        ABI, Access, AccessFs, CompatLevel, Compatible, Ruleset, RulesetAttr, RulesetCreatedAttr,
        RulesetStatus, path_beneath_rules,
    };

    let abi = ABI::V1;
    let created = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::from_all(abi))
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
    let scratch = home.join("tmp");
    let _ = std::fs::create_dir_all(&scratch);
    let ws_access = match profile {
        SandboxProfile::ReadOnly => AccessFs::from_read(abi),
        SandboxProfile::Workspace | SandboxProfile::Off => AccessFs::from_all(abi),
    };

    let ro = existing(&[
        "/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc", "/proc", "/dev",
    ]);
    // Do not allow `/tmp` itself: TempDir and other projects live there.
    // Scratch is `~/.ryter/tmp` (or `$RYTER_HOME/tmp`).
    //
    // Granting all of `~/.ryter` used to hand tools read/write on
    // `~/.ryter/keys/<connection>`, so even the `read-only` profile let a
    // builder's bash read every API key. Landlock has no negative rules, so the
    // writable set is enumerated instead. Keys are resolved before `apply` runs,
    // so nothing here needs them.
    let rw = writable_set(&home);
    let _ = &scratch;
    let status = created
        .set_compatibility(CompatLevel::BestEffort)
        .add_rules(path_beneath_rules(&ro, AccessFs::from_read(abi)))
        .map_err(|e| Error::Config(format!("sandbox rules: {e}")))?
        .add_rules(path_beneath_rules(&rw, AccessFs::from_all(abi)))
        .map_err(|e| Error::Config(format!("sandbox rules: {e}")))?
        .add_rules(path_beneath_rules(&[ws], ws_access))
        .map_err(|e| Error::Config(format!("sandbox rules: {e}")))?
        .restrict_self()
        .map_err(|e| Error::Config(format!("sandbox restrict: {e}")))?;

    if status.ruleset == RulesetStatus::NotEnforced {
        return Err(Error::Config(format!(
            "sandbox {profile} requested but Landlock was not enforced"
        )));
    }
    Ok(())
}

/// Directories a sandboxed thread may write, created if missing.
///
/// Enumerated rather than granting `~/.ryter` wholesale: Landlock has no
/// negative rules, so any grant on the parent would re-expose `keys/`.
#[cfg(target_os = "linux")]
fn writable_set(home: &Path) -> Vec<std::path::PathBuf> {
    ["tmp", "logs", "sessions"]
        .iter()
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
}
