//! Updating Ryter itself from its GitHub releases (`ryter update`, and the
//! check at launch).
//!
//! A release carries `SHA256SUMS` and `SHA256SUMS.sig`, an ed25519 signature
//! over the sums made by the release workflow with a key only it holds.
//! Ryter installs a release only when that signature matches the public key
//! built into it and the download matches its sum. A checksum from the same
//! release alone would only catch a corrupt download, not a release someone
//! tampered with. The new binary must then run and report its version
//! before it replaces this one, the way `install.sh` replaces it, so a
//! running Ryter keeps working until it restarts.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

use crate::config::UpdateMode;
use crate::error::{Error, Result};

/// The latest published release. Drafts are never listed, so publishing a
/// release is what offers it to everyone.
const LATEST: &str = "https://api.github.com/repos/zypher-systems/ryter/releases/latest";
/// Release files: `<DOWNLOADS>/<tag>/<file>`.
const DOWNLOADS: &str = "https://github.com/zypher-systems/ryter/releases/download";

/// The public half of the release signing key, as the release workflow
/// checks every signature against it.
const RELEASE_KEY_PEM: &str = include_str!("../../../release/ryter-release.pub.pem");

/// The key a release must be signed with, base64 (32 bytes, ed25519): the
/// release key, or `RYTER_UPDATE_PUBKEY` at build time (a fork, a test).
fn built_in_key() -> String {
    match option_env!("RYTER_UPDATE_PUBKEY") {
        Some(key) => key.to_string(),
        None => key_from_pem(RELEASE_KEY_PEM).unwrap_or_default(),
    }
}

/// The raw key in an ed25519 `PUBLIC KEY` PEM (what `openssl pkey -pubout`
/// writes), base64.
fn key_from_pem(pem: &str) -> Option<String> {
    // SubjectPublicKeyInfo for ed25519: this prefix, then the 32-byte key.
    const PREFIX: [u8; 12] = [
        0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
    ];
    let body: String = pem
        .lines()
        .skip_while(|l| !l.starts_with("-----BEGIN PUBLIC KEY-----"))
        .skip(1)
        .take_while(|l| !l.starts_with("-----END"))
        .collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(body.trim())
        .ok()?;
    let key = der.strip_prefix(&PREFIX[..])?;
    (key.len() == 32).then(|| base64::engine::general_purpose::STANDARD.encode(key))
}

/// Set by the release workflow on the binaries it publishes. Only those
/// replace themselves; a cargo build (installed or in a checkout, wherever
/// its target folder is) updates the way it was built.
const RELEASE_BUILD: bool = option_env!("RYTER_RELEASE_BUILD").is_some();

/// Whether this binary came from a release, and so may replace itself.
pub fn is_release_build() -> bool {
    RELEASE_BUILD
}

/// What to say when a newer release is out and this build can't take it:
/// one built with cargo updates the way it was built.
pub fn built_with_cargo(avail: &Available, current: Version) -> String {
    format!(
        "Ryter {} is out (you have {current}). This Ryter was built with cargo, so update it \
         the way you built it, or install the release: {INSTALL}. What's new: {}",
        avail.version, avail.notes
    )
}

/// The launch check runs at most this often.
const CHECK_EVERY_SECS: u64 = 24 * 60 * 60;

/// The install script, for when Ryter can't replace itself.
const INSTALL: &str =
    "curl -fsSL https://raw.githubusercontent.com/zypher-systems/ryter/main/install.sh | sh";

/// The install script, putting Ryter in `dir`.
fn install_into(dir: &Path) -> String {
    format!(
        "curl -fsSL https://raw.githubusercontent.com/zypher-systems/ryter/main/install.sh | \
         RYTER_INSTALL_DIR='{}' sh",
        dir.display()
    )
}

/// `MAJOR.MINOR.PATCH`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u64, pub u64, pub u64);

impl Version {
    /// `v0.9.1` or `0.9.1`. Anything else (a pre-release tag) is `None`.
    pub fn parse(s: &str) -> Option<Self> {
        let mut parts = s.trim().trim_start_matches('v').split('.');
        let v = Version(
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
        );
        parts.next().is_none().then_some(v)
    }

    /// This build's version.
    pub fn current() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).expect("the crate's version is MAJOR.MINOR.PATCH")
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// Where releases come from, and the key they must be signed with.
#[derive(Debug, Clone)]
pub struct Source {
    /// The latest release, as GitHub's API describes it.
    pub latest: String,
    /// Release files: `<downloads>/<tag>/<file>`.
    pub downloads: String,
    /// The signing key, base64.
    pub key: String,
}

impl Source {
    /// GitHub, or a mirror at `RYTER_UPDATE_URL` (`<url>/latest`, and files
    /// at `<url>/download/<tag>/<file>`). Either way a release must be
    /// signed with the key built into this binary.
    pub fn new() -> Self {
        match std::env::var("RYTER_UPDATE_URL") {
            Ok(url) if !url.trim().is_empty() => {
                let url = url.trim().trim_end_matches('/');
                Self {
                    latest: format!("{url}/latest"),
                    downloads: format!("{url}/download"),
                    key: built_in_key(),
                }
            }
            _ => Self {
                latest: LATEST.into(),
                downloads: DOWNLOADS.into(),
                key: built_in_key(),
            },
        }
    }
}

impl Default for Source {
    fn default() -> Self {
        Self::new()
    }
}

/// A release newer than this build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Available {
    /// Its version.
    pub version: Version,
    /// Its tag (`v0.9.1`).
    pub tag: String,
    /// Its page, with the release notes.
    pub notes: String,
}

/// The latest release, if it is newer than `current`.
pub fn check(src: &Source, current: Version) -> Result<Option<Available>> {
    let client = client(Duration::from_secs(15))?;
    let body = get(&client, &src.latest, 1 << 20).map_err(|e| {
        let why = match e {
            Error::Io(m) => m,
            e => e.to_string(),
        };
        Error::Io(format!("couldn't check for a newer release: {why}"))
    })?;
    let release: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| Error::Io(format!("the release list didn't parse: {e}")))?;
    if release["draft"] == true || release["prerelease"] == true {
        return Ok(None);
    }
    let tag = release["tag_name"].as_str().unwrap_or_default();
    let Some(version) = Version::parse(tag) else {
        return Ok(None);
    };
    Ok((version > current).then(|| Available {
        version,
        tag: tag.to_string(),
        notes: release["html_url"].as_str().unwrap_or_default().to_string(),
    }))
}

/// This binary, when Ryter may replace it: one the release workflow built,
/// not a development build or one `cargo install` manages.
pub fn installed_binary() -> Result<PathBuf> {
    let exe = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .map_err(|e| Error::Io(format!("can't find this binary: {e}")))?;
    let cargo_bin = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".cargo")))
        .map(|c| c.join("bin"));
    replaceable(&exe, cargo_bin.as_deref(), RELEASE_BUILD)?;
    Ok(exe)
}

/// Why `exe` isn't Ryter's to replace, if it isn't.
fn replaceable(exe: &Path, cargo_bin: Option<&Path>, release_build: bool) -> Result<()> {
    if !release_build {
        return Err(Error::Config(format!(
            "this Ryter at {} was built with cargo, not taken from a release; update it the \
             way you built it, or install the release: {INSTALL}",
            exe.display()
        )));
    }
    if is_dev_build(exe) {
        return Err(Error::Config(format!(
            "{} is a development build; build it again with cargo instead",
            exe.display()
        )));
    }
    let cargo_bin = cargo_bin.map(|b| b.canonicalize().unwrap_or_else(|_| b.to_path_buf()));
    if cargo_bin.is_some_and(|b| exe.starts_with(b)) {
        return Err(Error::Config(
            "this Ryter was installed with cargo; update it the same way: cargo install --git \
             https://github.com/zypher-systems/ryter ryter-cli"
                .into(),
        ));
    }
    Ok(())
}

/// A binary cargo built in a checkout: `…/target/[<triple>/]{debug,release}/`.
fn is_dev_build(exe: &Path) -> bool {
    let parts: Vec<String> = exe
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    parts.iter().enumerate().any(|(i, p)| {
        p == "target"
            && parts[i + 1..]
                .iter()
                .take(2)
                .any(|q| q == "debug" || q == "release")
    })
}

/// Download `avail`, check it, and put it in place of `exe`.
pub fn install(src: &Source, avail: &Available, exe: &Path) -> Result<()> {
    install_within(src, avail, exe, PROBE_LIMIT)
}

/// [`install`], giving the new binary `limit` to report its version.
fn install_within(src: &Source, avail: &Available, exe: &Path, limit: Duration) -> Result<()> {
    let target = target().ok_or_else(|| {
        Error::Config(format!(
            "there's no prebuilt Ryter for {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
    })?;
    let key = public_key(&src.key)?;
    let asset = format!("ryter-{target}.tar.gz");
    let client = client(Duration::from_secs(300))?;
    let base = format!("{}/{}", src.downloads, avail.tag);
    let sums = get(&client, &format!("{base}/SHA256SUMS"), 64 << 10)?;
    // Releases before 0.9.0 aren't signed.
    let sig = get(&client, &format!("{base}/SHA256SUMS.sig"), 4 << 10).map_err(|_| {
        Error::Config(format!(
            "{} isn't signed (it has no SHA256SUMS.sig); nothing was installed",
            avail.tag
        ))
    })?;
    verify(&key, &sums, &sig)?;
    let want = sum_for(&sums, &asset)?;
    let tarball = get(&client, &format!("{base}/{asset}"), 200 << 20)?;
    if hex(&Sha256::digest(&tarball)) != want {
        return Err(Error::Config(format!(
            "{asset} doesn't match its checksum; nothing was installed"
        )));
    }
    let binary = unpack(&tarball, &format!("ryter-{target}/ryter"))?;
    replace_within(exe, &binary, avail.version, limit)
}

/// When it last checked, and whether the launch check is due.
fn due(home: &Path, now: u64) -> bool {
    let last = std::fs::read_to_string(home.join("update.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["checked"].as_u64());
    // Never checked is due, whatever the clock says.
    last.is_none_or(|last| now.saturating_sub(last) >= CHECK_EVERY_SECS)
}

fn record(home: &Path, now: u64) {
    let _ = std::fs::create_dir_all(home);
    let _ = std::fs::write(
        home.join("update.json"),
        serde_json::json!({ "checked": now }).to_string(),
    );
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The launch check: at most once a day, by `mode`. Returns what to tell the
/// user, if anything. A check that fails (offline, say) says nothing and is
/// tried again next launch. A development build never checks.
pub fn on_launch(home: &Path, mode: UpdateMode) -> Option<String> {
    if std::env::current_exe().is_ok_and(|exe| is_dev_build(&exe)) {
        return None;
    }
    let src = Source::new();
    let current = Version::current();
    launch(
        home,
        mode,
        &src,
        current,
        installed_binary,
        now(),
        RELEASE_BUILD,
    )
}

/// [`on_launch`], with what it depends on passed in.
fn launch(
    home: &Path,
    mode: UpdateMode,
    src: &Source,
    current: Version,
    exe: impl FnOnce() -> Result<PathBuf>,
    now: u64,
    release_build: bool,
) -> Option<String> {
    if mode == UpdateMode::Off || !due(home, now) {
        return None;
    }
    let found = check(src, current).ok()?;
    record(home, now);
    let avail = found?;
    Some(match mode {
        // A cargo build is told it's out, and how to update it.
        _ if !release_build => built_with_cargo(&avail, current),
        UpdateMode::Install => match exe().and_then(|exe| install(src, &avail, &exe)) {
            Ok(()) => format!(
                "Ryter {} is installed. Restart Ryter to use it. What's new: {}",
                avail.version, avail.notes
            ),
            Err(e) => format!(
                "Ryter {} is out (you have {current}), but it couldn't be installed: {e}",
                avail.version
            ),
        },
        _ => format!(
            "Ryter {} is out (you have {current}). `ryter update` installs it. What's new: {}",
            avail.version, avail.notes
        ),
    })
}

/// The release asset's target for this machine.
fn target() -> Option<&'static str> {
    match (std::env::consts::ARCH, std::env::consts::OS) {
        ("x86_64", "linux") => Some("x86_64-unknown-linux-musl"),
        ("aarch64", "linux") => Some("aarch64-unknown-linux-musl"),
        ("x86_64", "macos") => Some("x86_64-apple-darwin"),
        ("aarch64", "macos") => Some("aarch64-apple-darwin"),
        _ => None,
    }
}

fn client(timeout: Duration) -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(format!("ryter/{}", Version::current()))
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout)
        .build()
        .map_err(|e| Error::Io(e.to_string()))
}

/// `url`'s body, refusing more than `max` bytes.
fn get(client: &reqwest::blocking::Client, url: &str, max: u64) -> Result<Vec<u8>> {
    let resp = client
        .get(url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|e| Error::Io(format!("{url}: {e}")))?;
    let mut body = Vec::new();
    resp.take(max + 1)
        .read_to_end(&mut body)
        .map_err(|e| Error::Io(format!("{url}: {e}")))?;
    if body.len() as u64 > max {
        return Err(Error::Io(format!("{url} is larger than expected")));
    }
    Ok(body)
}

fn public_key(b64: &str) -> Result<VerifyingKey> {
    let no_key = || {
        Error::Config(
            "this build has no release key to check updates with; install with the script: \
             curl -fsSL https://raw.githubusercontent.com/zypher-systems/ryter/main/install.sh | sh"
                .into(),
        )
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|_| no_key())?;
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| no_key())?;
    VerifyingKey::from_bytes(&bytes).map_err(|_| no_key())
}

/// `sums` signed with `key`; `sig` is the signature in base64.
fn verify(key: &VerifyingKey, sums: &[u8], sig: &[u8]) -> Result<()> {
    let refused = || {
        Error::Config(
            "the release's signature doesn't match Ryter's release key; nothing was installed"
                .into(),
        )
    };
    let sig = std::str::from_utf8(sig).map_err(|_| refused())?;
    let sig = base64::engine::general_purpose::STANDARD
        .decode(sig.trim())
        .map_err(|_| refused())?;
    let sig = Signature::from_slice(&sig).map_err(|_| refused())?;
    key.verify_strict(sums, &sig).map_err(|_| refused())
}

/// `asset`'s sum in `SHA256SUMS` (`<hex>  <name>` lines).
fn sum_for(sums: &[u8], asset: &str) -> Result<String> {
    String::from_utf8_lossy(sums)
        .lines()
        .find_map(|line| {
            let mut words = line.split_whitespace();
            let sum = words.next()?;
            let name = words.next()?.trim_start_matches('*');
            (name == asset).then(|| sum.to_ascii_lowercase())
        })
        .ok_or_else(|| Error::Config(format!("{asset} isn't in the release's SHA256SUMS")))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The file at `path` in the gzipped tarball.
fn unpack(tarball: &[u8], path: &str) -> Result<Vec<u8>> {
    let bad = |e: std::io::Error| Error::Io(format!("the download didn't unpack: {e}"));
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(tarball));
    for entry in archive.entries().map_err(bad)? {
        let mut entry = entry.map_err(bad)?;
        if entry.header().entry_type().is_file() && entry.path().map_err(bad)? == Path::new(path) {
            let mut out = Vec::new();
            entry
                .by_ref()
                .take(200 << 20)
                .read_to_end(&mut out)
                .map_err(bad)?;
            return Ok(out);
        }
    }
    Err(Error::Config(format!("the download has no {path}")))
}

/// How long a new binary has to report its version.
const PROBE_LIMIT: Duration = Duration::from_secs(10);

/// How much of its output is read.
const PROBE_OUTPUT: usize = 4 << 10;

/// Put `binary` in place of `exe`, once it runs and reports `version`
/// within `limit`. The swap is a rename in the same folder, so a running
/// Ryter isn't disturbed.
fn replace_within(exe: &Path, binary: &[u8], version: Version, limit: Duration) -> Result<()> {
    let dir = exe
        .parent()
        .ok_or_else(|| Error::Config(format!("{} has no folder", exe.display())))?;
    clear_leftovers(dir);
    let tmp = dir.join(format!(".ryter.update-{}", std::process::id()));
    if let Err(e) = std::fs::write(&tmp, binary) {
        let _ = std::fs::remove_file(&tmp);
        return Err(Error::Config(match e.kind() {
            std::io::ErrorKind::PermissionDenied => format!(
                "Ryter can't write to {}. Update it as the account that installed it: {}",
                dir.display(),
                install_into(dir)
            ),
            _ => format!("couldn't write the update to {}: {e}", dir.display()),
        }));
    }
    let checked = (|| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| Error::Io(e.to_string()))?;
        }
        let said = probe(&tmp, limit)?;
        if said.split_whitespace().nth(1) != Some(&version.to_string()) {
            return Err(Error::Config(format!(
                "the new binary reported {:?}, not ryter {version}; nothing was installed",
                said.trim()
            )));
        }
        std::fs::rename(&tmp, exe)
            .map_err(|e| Error::Io(format!("couldn't put the update in place: {e}")))
    })();
    if checked.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    checked
}

/// What `binary --version` prints, within `limit`.
///
/// Nothing it does can hold the update up. It gets no input, so it can't
/// read the terminal. Its output is read up to [`PROBE_OUTPUT`] by a reader
/// that is never waited on: a flood ends when that reader stops, and a
/// child left holding the output open doesn't block. Past `limit`, it and
/// everything it started (its own process group) are killed and reaped.
/// `Command::output` had no deadline: a signed binary that hung on
/// `--version` hung `ryter update`, and the update file stayed behind.
fn probe(binary: &Path, limit: Duration) -> Result<String> {
    use std::process::Stdio;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    let mut cmd = std::process::Command::new(binary);
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Config(format!("the new binary didn't run: {e}")))?;
    let said = Arc::new(Mutex::new(Vec::new()));
    let ended = Arc::new(AtomicBool::new(false));
    if let Some(mut out) = child.stdout.take() {
        let (said, ended) = (said.clone(), ended.clone());
        std::thread::spawn(move || {
            let mut chunk = [0u8; 1024];
            while let Ok(n @ 1..) = out.read(&mut chunk) {
                let Ok(mut said) = said.lock() else { break };
                said.extend_from_slice(&chunk[..n]);
                if said.len() >= PROBE_OUTPUT {
                    break;
                }
            }
            ended.store(true, Ordering::Release);
        });
    }
    let until = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                end_group(&mut child);
                return Err(Error::Config(format!(
                    "the new binary didn't report its version within {} s; nothing was installed",
                    limit.as_secs_f32()
                )));
            }
        }
    };
    // Its output is in the pipe by now; a child of its own may hold the
    // pipe open, so don't wait for the end for long.
    let grace = Instant::now() + Duration::from_millis(500);
    while !ended.load(Ordering::Acquire) && Instant::now() < grace {
        std::thread::sleep(Duration::from_millis(10));
    }
    if !status.success() {
        return Err(Error::Config(format!(
            "the new binary failed ({status}); nothing was installed"
        )));
    }
    let said = said.lock().map(|s| s.clone()).unwrap_or_default();
    Ok(String::from_utf8_lossy(&said[..said.len().min(PROBE_OUTPUT)]).into_owned())
}

/// Kill a probe that ran out of time, and everything it started, then reap
/// it. The group is killed while its leader is unreaped, so its id can't
/// have passed to another process.
fn end_group(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let group = rustix::process::Pid::from_child(child);
        let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Update files an interrupted update left in `dir` (Ryter quit mid-way),
/// once they are an hour old: a newer one may be another Ryter's, at work.
fn clear_leftovers(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > Duration::from_secs(60 * 60));
        if stale
            && entry
                .file_name()
                .to_string_lossy()
                .starts_with(".ryter.update-")
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use std::io::Write;
    use tempfile::TempDir;

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    /// A release's tarball holding `ryter-<target>/ryter`: a script that
    /// reports `version`.
    fn tarball(target: &str, version: &str) -> Vec<u8> {
        tarball_of(target, &format!("#!/bin/sh\necho 'ryter {version}'\n"))
    }

    /// A release's tarball holding `ryter-<target>/ryter` as `script`.
    fn tarball_of(target: &str, script: &str) -> Vec<u8> {
        let mut tar = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(script.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(
            &mut header,
            format!("ryter-{target}/ryter"),
            script.as_bytes(),
        )
        .unwrap();
        let tar = tar.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&tar).unwrap();
        gz.finish().unwrap()
    }

    /// A release server on a local port: `/latest`, and files under
    /// `/download/<tag>/`. Returns its base URL.
    fn serve(files: Vec<(String, Vec<u8>)>) -> String {
        use std::io::{BufRead, BufReader};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                let _ = reader.read_line(&mut line);
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                        break;
                    }
                }
                let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                let mut stream = stream;
                match files.iter().find(|(p, _)| *p == path) {
                    Some((_, body)) => {
                        let _ = write!(
                            stream,
                            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(body);
                    }
                    None => {
                        let _ = stream.write_all(
                            b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                        );
                    }
                }
            }
        });
        url
    }

    /// A release's files: (path, body).
    type Files = Vec<(String, Vec<u8>)>;

    /// A change to a release, to see it refused: what the error must say.
    type Tamper = (&'static str, Box<dyn Fn(&mut Files)>);

    /// A signed release of `version` for this machine, served locally.
    fn release(version: &str, tamper: impl Fn(&mut Files)) -> Source {
        let target = target().expect("a supported test machine");
        let tag = format!("v{version}");
        let asset = format!("ryter-{target}.tar.gz");
        let tgz = tarball(target, version);
        let sums = format!("{}  {asset}\n", hex(&Sha256::digest(&tgz)));
        let sig = b64(&signing_key().sign(sums.as_bytes()).to_bytes());
        let latest = serde_json::json!({
            "tag_name": tag,
            "html_url": format!("https://example.test/releases/{tag}"),
            "draft": false,
            "prerelease": false,
        });
        let mut files = vec![
            ("/latest".to_string(), latest.to_string().into_bytes()),
            (format!("/download/{tag}/SHA256SUMS"), sums.into_bytes()),
            (format!("/download/{tag}/SHA256SUMS.sig"), sig.into_bytes()),
            (format!("/download/{tag}/{asset}"), tgz),
        ];
        tamper(&mut files);
        let url = serve(files);
        Source {
            latest: format!("{url}/latest"),
            downloads: format!("{url}/download"),
            key: b64(signing_key().verifying_key().as_bytes()),
        }
    }

    /// An installed `ryter` 0.1.0 in a folder of its own.
    fn installed() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let exe = dir.path().join("ryter");
        std::fs::write(&exe, "#!/bin/sh\necho 'ryter 0.1.0'\n").unwrap();
        (dir, exe)
    }

    /// The PEM `openssl pkey -pubout` writes, made here from a known key.
    fn pem_of(key: &VerifyingKey) -> String {
        let mut der = vec![
            0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
        ];
        der.extend_from_slice(key.as_bytes());
        format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
            b64(&der)
        )
    }

    /// The key file's PEM gives the raw key; a placeholder gives none.
    #[test]
    fn the_release_key_is_read_from_its_pem() {
        let key = signing_key().verifying_key();
        assert_eq!(key_from_pem(&pem_of(&key)), Some(b64(key.as_bytes())));
        assert_eq!(key_from_pem("# not yet\n"), None);
        assert_eq!(
            key_from_pem("-----BEGIN PUBLIC KEY-----\nAAAA\n-----END PUBLIC KEY-----\n"),
            None
        );
    }

    /// The committed release key parses, so a release build can check
    /// updates: a placeholder or a damaged file fails here, not in a user's
    /// `ryter update`.
    #[test]
    fn the_release_key_is_in_place() {
        let key = key_from_pem(RELEASE_KEY_PEM).expect("release/ryter-release.pub.pem");
        assert!(public_key(&key).is_ok());
    }

    /// What the release workflow does, `openssl pkeyutl -sign -rawin`, is
    /// what Ryter verifies. Opt in: `RYTER_OPENSSL=1`, with openssl 3 on
    /// `PATH`.
    #[test]
    #[ignore = "set RYTER_OPENSSL=1; needs openssl 3"]
    fn openssl_signatures_verify() {
        if std::env::var("RYTER_OPENSSL").as_deref() != Ok("1") {
            return;
        }
        let dir = TempDir::new().unwrap();
        let run = |args: &[&str]| {
            let ok = std::process::Command::new("openssl")
                .args(args)
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success();
            assert!(ok, "openssl {args:?}");
        };
        run(&["genpkey", "-algorithm", "ed25519", "-out", "k.pem"]);
        run(&["pkey", "-in", "k.pem", "-pubout", "-out", "k.pub.pem"]);
        std::fs::write(dir.path().join("SHA256SUMS"), "abc  ryter-x.tar.gz\n").unwrap();
        run(&[
            "pkeyutl",
            "-sign",
            "-inkey",
            "k.pem",
            "-rawin",
            "-in",
            "SHA256SUMS",
            "-out",
            "sig",
        ]);
        let pem = std::fs::read_to_string(dir.path().join("k.pub.pem")).unwrap();
        let key = public_key(&key_from_pem(&pem).expect("openssl's PEM")).unwrap();
        let sums = std::fs::read(dir.path().join("SHA256SUMS")).unwrap();
        let sig = b64(&std::fs::read(dir.path().join("sig")).unwrap());
        verify(&key, &sums, sig.as_bytes()).unwrap();
        assert!(verify(&key, b"other", sig.as_bytes()).is_err());
    }

    #[test]
    fn versions_compare_as_numbers() {
        assert_eq!(Version::parse("v0.10.0"), Some(Version(0, 10, 0)));
        assert!(Version::parse("v0.10.0") > Version::parse("0.9.9"));
        assert_eq!(Version::parse("v1.0.0-rc1"), None);
        assert_eq!(Version::parse("1.2"), None);
        assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
    }

    /// The whole path: a newer signed release is found, checked, unpacked,
    /// run, and swapped in. An older or equal one isn't offered.
    #[test]
    fn a_signed_release_is_installed() {
        let src = release("9.9.9", |_| {});
        let avail = check(&src, Version(0, 1, 0)).unwrap().expect("newer");
        assert_eq!(avail.version, Version(9, 9, 9));
        assert_eq!(avail.notes, "https://example.test/releases/v9.9.9");
        assert_eq!(check(&src, Version(9, 9, 9)).unwrap(), None);
        let (dir, exe) = installed();
        install(&src, &avail, &exe).unwrap();
        let out = std::process::Command::new(&exe)
            .arg("--version")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ryter 9.9.9");
        let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().collect();
        assert_eq!(left.len(), 1, "no update file left behind");
    }

    /// Nothing is installed unless the signature, the sum, and the binary
    /// itself all check out. The installed binary is untouched each time.
    #[test]
    fn a_release_that_doesnt_check_out_is_refused() {
        let tag = "/download/v9.9.9";
        let cases: Vec<Tamper> = vec![
            (
                "signature",
                Box::new(move |f| {
                    let other = SigningKey::from_bytes(&[9u8; 32]);
                    let sums = f
                        .iter()
                        .find(|(p, _)| p.ends_with("SHA256SUMS"))
                        .unwrap()
                        .1
                        .clone();
                    let forged = b64(&other.sign(&sums).to_bytes());
                    f.iter_mut().find(|(p, _)| p.ends_with(".sig")).unwrap().1 =
                        forged.into_bytes();
                }),
            ),
            (
                "isn't signed",
                Box::new(move |f| f.retain(|(p, _)| !p.ends_with(".sig"))),
            ),
            (
                "checksum",
                Box::new(move |f| {
                    let target = target().unwrap();
                    f.iter_mut()
                        .find(|(p, _)| *p == format!("{tag}/ryter-{target}.tar.gz"))
                        .unwrap()
                        .1 = tarball(target, "9.9.9").into_iter().chain([0u8]).collect();
                }),
            ),
            (
                "reported",
                Box::new(move |f| {
                    // Signed and summed, but the binary is the wrong version.
                    let target = target().unwrap();
                    let tgz = tarball(target, "6.6.6");
                    let sums = format!("{}  ryter-{target}.tar.gz\n", hex(&Sha256::digest(&tgz)));
                    let sig = b64(&signing_key().sign(sums.as_bytes()).to_bytes());
                    for (p, body) in f.iter_mut() {
                        if p.ends_with(".tar.gz") {
                            *body = tgz.clone();
                        } else if p.ends_with("SHA256SUMS") {
                            *body = sums.clone().into_bytes();
                        } else if p.ends_with(".sig") {
                            *body = sig.clone().into_bytes();
                        }
                    }
                }),
            ),
        ];
        for (why, tamper) in cases {
            let src = release("9.9.9", tamper);
            let avail = check(&src, Version(0, 1, 0)).unwrap().unwrap();
            let (dir, exe) = installed();
            let err = install(&src, &avail, &exe).unwrap_err().to_string();
            assert!(err.contains(why), "{why}: {err}");
            assert_eq!(
                std::fs::read_to_string(&exe).unwrap(),
                "#!/bin/sh\necho 'ryter 0.1.0'\n",
                "{why}: the installed binary is untouched"
            );
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "{why}");
        }
        // A build without a release key installs nothing.
        let mut src = release("9.9.9", |_| {});
        src.key = String::new();
        let avail = check(&src, Version(0, 1, 0)).unwrap().unwrap();
        let (_dir, exe) = installed();
        assert!(
            install(&src, &avail, &exe)
                .unwrap_err()
                .to_string()
                .contains("no release key")
        );
    }

    /// Drafts and pre-releases are never offered.
    #[test]
    fn drafts_and_prereleases_are_not_offered() {
        for (key, tag) in [
            ("draft", "v9.9.9"),
            ("prerelease", "v9.9.9"),
            ("none", "v9.9.9-rc1"),
        ] {
            let src = release("9.9.9", move |f| {
                let mut latest = serde_json::json!({"tag_name": tag, "html_url": "x"});
                if key != "none" {
                    latest[key] = serde_json::json!(true);
                }
                f[0].1 = latest.to_string().into_bytes();
            });
            assert_eq!(check(&src, Version(0, 1, 0)).unwrap(), None, "{key}");
        }
    }

    /// A development build and a cargo install aren't Ryter's to replace.
    #[test]
    fn only_an_installed_release_is_replaced() {
        for dev in [
            "/home/me/ryter/target/release/ryter",
            "/home/me/ryter/target/debug/ryter",
            "/home/me/ryter/target/x86_64-unknown-linux-musl/release/ryter",
        ] {
            let err = replaceable(Path::new(dev), None, true)
                .unwrap_err()
                .to_string();
            assert!(err.contains("development build"), "{dev}: {err}");
        }
        let cargo = Path::new("/home/me/.cargo/bin");
        let err = replaceable(&cargo.join("ryter"), Some(cargo), true)
            .unwrap_err()
            .to_string();
        assert!(err.contains("cargo install"), "{err}");
        let installed = Path::new("/home/me/.local/bin/ryter");
        assert!(replaceable(installed, Some(cargo), true).is_ok());
        // Only a binary the release workflow built replaces itself: a cargo
        // build in a target folder of any name doesn't. One in
        // `target-updtest/` was replaced in a live test.
        let err = replaceable(Path::new("/x/target-updtest/release/ryter"), None, false)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("built with cargo") && err.contains("install.sh"),
            "{err}"
        );
        assert!(replaceable(installed, Some(cargo), false).is_err());
    }

    /// A folder Ryter can't write to says how to update instead.
    #[cfg(unix)]
    #[test]
    fn a_folder_it_cant_write_says_how_to_update() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, exe) = installed();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
        let probe = dir.path().join("probe");
        if std::fs::write(&probe, "x").is_ok() {
            // Running as root: permissions don't apply, so there's nothing to see.
            let _ = std::fs::remove_file(probe);
        } else {
            let err = replace_within(&exe, b"#!/bin/sh\n", Version(9, 9, 9), PROBE_LIMIT)
                .unwrap_err()
                .to_string();
            assert!(
                err.contains("can't write to") && err.contains("install.sh"),
                "{err}"
            );
        }
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A signed 9.9.9 release whose binary is `script`.
    fn signed_script(script: &'static str) -> Source {
        release("9.9.9", move |f| {
            let target = target().unwrap();
            let tgz = tarball_of(target, script);
            let sums = format!("{}  ryter-{target}.tar.gz\n", hex(&Sha256::digest(&tgz)));
            let sig = b64(&signing_key().sign(sums.as_bytes()).to_bytes());
            for (p, body) in f.iter_mut() {
                if p.ends_with(".tar.gz") {
                    *body = tgz.clone();
                } else if p.ends_with("SHA256SUMS") {
                    *body = sums.clone().into_bytes();
                } else if p.ends_with(".sig") {
                    *body = sig.clone().into_bytes();
                }
            }
        })
    }

    /// A signed binary that hangs, or floods its output, when asked its
    /// version can't hold the update up: a timely error, the installed
    /// binary untouched, and no update file left behind. One whose child
    /// holds its output open after it answers installs at once. The probe
    /// used to wait for ever (a reviewer's signed `sleep 30`).
    #[test]
    fn a_binary_that_hangs_cant_hold_the_update_up() {
        let limit = Duration::from_secs(1);
        for (why, script) in [
            ("hangs", "#!/bin/sh\nsleep 30 & sleep 30\n"),
            (
                "floods",
                "#!/bin/sh\nwhile :; do echo 'ryter 9.9.9 and more'; done\n",
            ),
        ] {
            let src = signed_script(script);
            let avail = check(&src, Version(0, 1, 0)).unwrap().unwrap();
            let (dir, exe) = installed();
            let started = Instant::now();
            let err = install_within(&src, &avail, &exe, limit)
                .unwrap_err()
                .to_string();
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "{why}: took {:?}",
                started.elapsed()
            );
            assert!(err.contains("nothing was installed"), "{why}: {err}");
            assert_eq!(
                std::fs::read_to_string(&exe).unwrap(),
                "#!/bin/sh\necho 'ryter 0.1.0'\n",
                "{why}: the installed binary is untouched"
            );
            let left: Vec<_> = std::fs::read_dir(dir.path())
                .unwrap()
                .flatten()
                .map(|e| e.file_name())
                .collect();
            assert_eq!(left.len(), 1, "{why}: {left:?}");
        }
        let src = signed_script("#!/bin/sh\nsleep 3 &\necho 'ryter 9.9.9'\n");
        let avail = check(&src, Version(0, 1, 0)).unwrap().unwrap();
        let (_dir, exe) = installed();
        let started = Instant::now();
        install_within(&src, &avail, &exe, limit).unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "took {:?}",
            started.elapsed()
        );
        assert!(std::fs::read_to_string(&exe).unwrap().contains("sleep 3"));
    }

    /// An update file an interrupted update left behind goes once it's an
    /// hour old; a newer one may be another Ryter's, at work.
    #[test]
    fn leftovers_of_an_interrupted_update_are_cleared() {
        let (dir, exe) = installed();
        let old = dir.path().join(".ryter.update-1");
        let new = dir.path().join(".ryter.update-2");
        std::fs::write(&old, "x").unwrap();
        std::fs::write(&new, "x").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - Duration::from_secs(2 * 60 * 60))
            .unwrap();
        clear_leftovers(dir.path());
        assert!(!old.exists() && new.exists() && exe.exists());
    }

    /// At launch: install mode installs and says to restart, notify mode
    /// only says it's out, a failed install says why, and it all happens at
    /// most once a day. Offline, it says nothing and tries next launch.
    #[test]
    fn the_launch_check_installs_or_notifies() {
        let src = release("9.9.9", |_| {});
        let old = Version(0, 1, 0);
        let (_dir, exe) = installed();
        let at = |p: &Path| {
            let p = p.to_path_buf();
            move || Ok(p)
        };

        let home = TempDir::new().unwrap();
        let said = launch(
            home.path(),
            UpdateMode::Notify,
            &src,
            old,
            at(&exe),
            100,
            true,
        )
        .unwrap();
        assert!(
            said.contains("9.9.9 is out (you have 0.1.0). `ryter update` installs it"),
            "{said}"
        );
        assert!(
            std::fs::read_to_string(&exe).unwrap().contains("0.1.0"),
            "notify installs nothing"
        );
        assert_eq!(
            launch(
                home.path(),
                UpdateMode::Install,
                &src,
                old,
                at(&exe),
                200,
                true
            ),
            None
        );

        let home = TempDir::new().unwrap();
        let said = launch(
            home.path(),
            UpdateMode::Install,
            &src,
            old,
            at(&exe),
            100,
            true,
        )
        .unwrap();
        assert!(
            said.starts_with("Ryter 9.9.9 is installed. Restart Ryter to use it."),
            "{said}"
        );
        let out = std::process::Command::new(&exe)
            .arg("--version")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ryter 9.9.9");

        let home = TempDir::new().unwrap();
        let refused = || Err(Error::Config("built with cargo".into()));
        let said = launch(
            home.path(),
            UpdateMode::Install,
            &src,
            old,
            refused,
            100,
            true,
        )
        .unwrap();
        assert!(
            said.contains("couldn't be installed") && said.contains("built with cargo"),
            "{said}"
        );

        // A cargo build copied anywhere is told how to update, not that
        // `ryter update` installs it, and nothing is downloaded or replaced.
        let home = TempDir::new().unwrap();
        let before = std::fs::read(&exe).unwrap();
        let said = launch(
            home.path(),
            UpdateMode::Install,
            &src,
            old,
            at(&exe),
            100,
            false,
        )
        .unwrap();
        assert!(
            said.contains("built with cargo") && !said.contains("`ryter update` installs"),
            "{said}"
        );
        assert_eq!(std::fs::read(&exe).unwrap(), before, "untouched");

        let home = TempDir::new().unwrap();
        let mut offline = src.clone();
        offline.latest = "http://127.0.0.1:1/latest".into();
        assert_eq!(
            launch(
                home.path(),
                UpdateMode::Install,
                &offline,
                old,
                at(&exe),
                100,
                true
            ),
            None
        );
        assert!(due(home.path(), 101), "tried again next launch");
    }

    /// The launch check runs at most once a day.
    #[test]
    fn the_launch_check_is_daily() {
        let home = TempDir::new().unwrap();
        assert!(due(home.path(), 1_000_000));
        record(home.path(), 1_000_000);
        assert!(!due(home.path(), 1_000_000 + 3600));
        assert!(due(home.path(), 1_000_000 + CHECK_EVERY_SECS));
        assert_eq!(on_launch(home.path(), UpdateMode::Off), None);
    }
}
