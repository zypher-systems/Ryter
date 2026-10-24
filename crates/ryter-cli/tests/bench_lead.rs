//! `ryter bench-lead`: the process a bench task's build hat runs under. Tried
//! through the binary, since in the test process it would end every child
//! of the test binary.

#![cfg(unix)]

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn alive(pid: i32) -> bool {
    #[cfg(target_os = "linux")]
    {
        // A zombie runs no code.
        fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|s| {
                s.rsplit_once(')')
                    .map(|(_, r)| r.split_whitespace().next() != Some("Z"))
            })
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .is_ok_and(|s| s.success())
    }
}

fn pid_in(path: &Path) -> i32 {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && fs::metadata(path).is_ok_and(|m| m.len() == 0) {
        std::thread::sleep(Duration::from_millis(50));
    }
    fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .parse()
        .expect("a pid was written")
}

fn leader(script: &str) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_ryter"))
        .args(["bench-lead", "sh", "-c", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the leader")
}

/// A process the hat detached into a session of its own, its parent gone
/// before the hat ended, with nothing of the hat's left in its environment:
/// reparented to the leader, found there, and ended before the leader exits.
#[cfg(target_os = "linux")]
#[test]
fn a_detached_process_whose_parent_is_gone_is_ended_with_the_hat() {
    if !Command::new("setsid")
        .arg("true")
        .status()
        .is_ok_and(|s| s.success())
    {
        eprintln!("no setsid here; nothing to detach with");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("pid");
    fs::write(&marker, "").unwrap();
    // The hat backgrounds a detached sleeper with a scrubbed environment,
    // waits until it has written its pid, and exits clean.
    let script = format!(
        "env -i PATH=\"$PATH\" setsid sh -c 'echo $$ > {m}; exec sleep 300' </dev/null >/dev/null 2>&1 & \
         while [ ! -s {m} ]; do sleep 0.05; done; echo done",
        m = marker.display()
    );
    let mut child = leader(&script);
    let stdin = child.stdin.take();
    let out = child.wait_with_output().unwrap();
    drop(stdin);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "done");
    let pid = pid_in(&marker);
    assert!(!alive(pid), "the detached sleep {pid} outlived the leader");
}

/// The runner closing the leader's stdin ends the hat and what it runs,
/// and the leader exits well within the grace, with a non-zero code.
#[test]
fn closing_the_control_ends_the_hat_and_its_tree() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("pid");
    fs::write(&marker, "").unwrap();
    let script = format!("sleep 300 & echo $! > {}; wait", marker.display());
    let mut child = leader(&script);
    let pid = pid_in(&marker);
    assert!(alive(pid));
    let started = Instant::now();
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "{:?}",
        started.elapsed()
    );
    assert!(!out.status.success(), "{out:?}");
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && alive(pid) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!alive(pid), "the sleep {pid} outlived the leader");
}

/// The hat's exit code is the leader's; a hat that cannot start is 127.
#[test]
fn the_hats_exit_code_comes_through() {
    let mut child = leader("exit 7");
    let stdin = child.stdin.take();
    let status = child.wait().unwrap();
    drop(stdin);
    assert_eq!(status.code(), Some(7));
    let mut child = Command::new(env!("CARGO_BIN_EXE_ryter"))
        .args(["bench-lead", "/nonexistent/ryter-bench-test", "--json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take();
    let out = child.wait_with_output().unwrap();
    drop(stdin);
    assert_eq!(out.status.code(), Some(127));
    assert!(String::from_utf8_lossy(&out.stderr).contains("could not start"));
}
