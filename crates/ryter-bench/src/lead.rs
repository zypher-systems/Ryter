//! The leader a task's build hat runs under: a process of ours between the
//! runner and the hat, whose only children are the hat and whatever the
//! hat's tree leaves behind.
//!
//! On Linux the leader is a child subreaper (`PR_SET_CHILD_SUBREAPER`): a
//! process under it whose parent exits is reparented to the leader, not to
//! `init`. So the hat's tree, walked by parent pid from the leader, is whole
//! at every moment, whatever a process does with its session (`setsid`), its
//! group, or its environment; nothing can leave it. When the hat ends, or the
//! runner asks for the end (the leader's stdin closes, which also happens
//! when the runner itself dies), the leader stops every process under
//! itself, kills it, reaps it, and exits with the hat's exit code. On the
//! other platforms there is no subreaper: the leader sees the tree while it
//! stands, and a process whose parent has exited is lost to it; the docs say
//! so.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The leader's exit code when processes under it could not be ended; the
/// runner does not score such a task.
pub const SURVIVORS: i32 = 121;

/// How long the hat has to end after `TERM` before its tree is killed.
const GRACE: Duration = Duration::from_secs(10);
/// How long the tree under the leader has to empty once killed.
const EMPTY_BY: Duration = Duration::from_secs(10);

/// Run `command` as the hat under this process and return the exit code to
/// leave with: the hat's own, 128 plus its signal when a signal ended it,
/// 127 when it could not start, [`SURVIVORS`] when the tree could not be
/// emptied after it. `control` is read on a thread; when it ends (EOF or a
/// byte), the hat is ended: `TERM` to its group, `GRACE`, then the tree.
pub fn lead(command: &[String], control: impl Read + Send + 'static) -> i32 {
    #[cfg(target_os = "linux")]
    if let Err(e) = rustix::process::set_child_subreaper(Some(rustix::process::getpid())) {
        eprintln!("ryter bench-lead: no subreaper, the tree is walked instead: {e}");
    }
    let Some((program, args)) = command.split_first() else {
        eprintln!("ryter bench-lead: no command");
        return 127;
    };
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut hat = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("could not start: {e}");
            return 127;
        }
    };
    let end = Arc::new(AtomicBool::new(false));
    {
        let end = end.clone();
        let mut control = control;
        std::thread::spawn(move || {
            let _ = control.read(&mut [0u8; 1]);
            end.store(true, Ordering::SeqCst);
        });
    }
    let mut code = loop {
        match hat.try_wait() {
            Ok(Some(status)) => break exit_code(&status),
            Ok(None) => {}
            Err(_) => break 1,
        }
        if end.load(Ordering::SeqCst) {
            #[cfg(unix)]
            {
                let group = rustix::process::Pid::from_child(&hat);
                let _ = rustix::process::kill_process_group(group, rustix::process::Signal::TERM);
            }
            let deadline = Instant::now() + GRACE;
            let status = loop {
                if let Ok(Some(status)) = hat.try_wait() {
                    break Some(status);
                }
                if Instant::now() >= deadline {
                    break None;
                }
                std::thread::sleep(Duration::from_millis(50));
            };
            break status.as_ref().map_or(128 + 9, exit_code);
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    // However the hat ended, nothing of its tree lives on past here.
    if !end_tree(std::process::id()) {
        eprintln!("ryter bench-lead: processes the hat started could not be ended");
        code = SURVIVORS;
    }
    let _ = hat.kill();
    let _ = hat.wait();
    code
}

fn exit_code(status: &std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    1
}

/// Stop, kill and reap everything under `root` until nothing is left, or
/// `EMPTY_BY` has passed; whether the tree is empty. Stopped first so that
/// nothing forks between the listing and the kill; a zombie is reaped when
/// it is ours, and becomes ours when its parent in the tree dies.
#[cfg(unix)]
fn end_tree(root: u32) -> bool {
    use rustix::process::{Pid, Signal, WaitOptions, kill_process, waitpid};
    let deadline = Instant::now() + EMPTY_BY;
    let mut stopped: Vec<u32> = Vec::new();
    for _ in 0..5 {
        let fresh: Vec<u32> = descendants(root)
            .into_iter()
            .filter(|p| !stopped.contains(p))
            .collect();
        if fresh.is_empty() {
            break;
        }
        for pid in &fresh {
            if let Some(pid) = Pid::from_raw(*pid as i32) {
                let _ = kill_process(pid, Signal::STOP);
            }
        }
        stopped.extend(fresh);
    }
    loop {
        let found = descendants(root);
        if found.is_empty() {
            return true;
        }
        for pid in &found {
            if let Some(pid) = Pid::from_raw(*pid as i32) {
                let _ = kill_process(pid, Signal::KILL);
            }
        }
        for pid in &found {
            if let Some(pid) = Pid::from_raw(*pid as i32) {
                let _ = waitpid(Some(pid), WaitOptions::NOHANG);
            }
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(not(unix))]
fn end_tree(_root: u32) -> bool {
    true
}

/// Every process under `root`, by parent pid: read from `/proc` on Linux
/// (no process is started for it, so the listing is of the tree alone),
/// from `ps` elsewhere. A zombie is listed; it leaves when it is reaped.
#[cfg(unix)]
pub(crate) fn descendants(root: u32) -> Vec<u32> {
    let pairs = parents();
    let mut found = vec![root];
    let mut i = 0;
    while i < found.len() {
        let parent = found[i];
        for (pid, ppid) in &pairs {
            if *ppid == parent && !found.contains(pid) {
                found.push(*pid);
            }
        }
        i += 1;
    }
    found.remove(0);
    found
}

/// `(pid, parent pid)` for every process.
#[cfg(target_os = "linux")]
fn parents() -> Vec<(u32, u32)> {
    let mut pairs = Vec::new();
    for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        // `pid (comm) state ppid …`; the comm may hold spaces and parens.
        let Some(rest) = stat.rsplit_once(')').map(|(_, r)| r) else {
            continue;
        };
        if let Some(ppid) = rest.split_whitespace().nth(1).and_then(|p| p.parse().ok()) {
            pairs.push((pid, ppid));
        }
    }
    pairs
}

#[cfg(all(unix, not(target_os = "linux")))]
fn parents() -> Vec<(u32, u32)> {
    parents_by_ps()
}

/// `(pid, parent pid)` for every process but the `ps` that listed them:
/// it is a child of this process while it runs, and listing it would keep
/// a leader's tree from ever looking empty.
#[cfg(all(unix, any(test, not(target_os = "linux"))))]
fn parents_by_ps() -> Vec<(u32, u32)> {
    let (ps, pairs) = ps_listing();
    pairs.into_iter().filter(|(pid, _)| *pid != ps).collect()
}

/// The `ps` process's own pid, and every `(pid, parent pid)` it printed.
#[cfg(all(unix, any(test, not(target_os = "linux"))))]
fn ps_listing() -> (u32, Vec<(u32, u32)>) {
    let Ok(child) = Command::new("ps")
        .args(["-eo", "pid=,ppid="])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return (0, Vec::new());
    };
    let ps = child.id();
    let Ok(out) = child.wait_with_output() else {
        return (ps, Vec::new());
    };
    let pairs = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
        })
        .collect();
    (ps, pairs)
}

/// Whether `pid` runs: a zombie, which runs no code, does not count.
#[cfg(all(test, unix))]
pub(crate) fn alive(pid: i32) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
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

// `lead` itself is tried through the binary, in `ryter-cli`'s
// `tests/bench_lead.rs`: run in this process it would end every child of
// the test binary, other tests' included.
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// The tree under a process is listed through more than one level,
    /// without starting a process to list it on Linux.
    #[test]
    fn descendants_are_listed_through_the_tree() {
        let mut child = Command::new("sh")
            .args(["-c", "sh -c 'sleep 30; echo' & wait"])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut found = descendants(child.id());
        while Instant::now() < deadline && found.len() < 2 {
            std::thread::sleep(Duration::from_millis(50));
            found = descendants(child.id());
        }
        assert!(found.len() >= 2, "{found:?}");
        let _ = child.kill();
        let _ = child.wait();
        // No parent walk starts from the sleep's grandparent's parent.
        assert!(!descendants(std::process::id()).contains(&child.id()));
    }

    /// `ps` lists itself under the process that ran it; the listing used
    /// for a tree leaves it out, or a leader with nothing under it would
    /// see a new child at every look.
    #[test]
    fn the_ps_listing_leaves_the_ps_itself_out() {
        let me = std::process::id();
        let (ps, raw) = ps_listing();
        assert!(ps != 0, "ps ran");
        assert!(raw.contains(&(ps, me)), "ps listed itself under us");
        assert!(
            !parents_by_ps().iter().any(|(pid, _)| *pid == ps),
            "the ps is left out"
        );
    }
}
