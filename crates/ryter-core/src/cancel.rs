//! Cooperative cancel plus Unix process-group kill.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

/// Shared stop flag for a turn. Cheap to clone (`Arc`).
#[derive(Debug)]
pub struct Cancel {
    flag: AtomicBool,
    notify: Notify,
    groups: Mutex<Vec<u32>>,
}

impl Cancel {
    /// Fresh, not cancelled.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            flag: AtomicBool::new(false),
            notify: Notify::new(),
            groups: Mutex::new(Vec::new()),
        })
    }

    /// True after [`cancel`](Self::cancel) and before [`reset`](Self::reset).
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Clear the flag and forget registered process groups. Call when a new turn starts.
    pub fn reset(&self) {
        self.flag.store(false, Ordering::SeqCst);
        if let Ok(mut g) = self.groups.lock() {
            g.clear();
        }
    }

    /// Set the flag, wake waiters, and `kill -KILL` registered process groups.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
        let groups = self.groups.lock().map(|g| g.clone()).unwrap_or_default();
        for pgid in groups {
            kill_group(pgid);
        }
    }

    /// Track a Unix process group so [`cancel`](Self::cancel) can kill it.
    pub fn register_pgid(&self, pgid: u32) {
        if let Ok(mut g) = self.groups.lock() {
            if !g.contains(&pgid) {
                g.push(pgid);
            }
        }
        if self.is_cancelled() {
            kill_group(pgid);
        }
    }

    /// Drop a process group after the child has exited.
    pub fn unregister_pgid(&self, pgid: u32) {
        if let Ok(mut g) = self.groups.lock() {
            g.retain(|p| *p != pgid);
        }
    }

    /// Wait until [`cancel`](Self::cancel) is called.
    pub async fn cancelled(&self) {
        loop {
            let notified = self.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

fn kill_group(pgid: u32) {
    if pgid == 0 {
        return;
    }
    let _ = std::process::Command::new("kill")
        .args(["-KILL", &format!("-{pgid}")])
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn cancel_is_sticky_until_reset() {
        let c = Cancel::new();
        assert!(!c.is_cancelled());
        c.cancel();
        assert!(c.is_cancelled());
        c.reset();
        assert!(!c.is_cancelled());
    }

    #[tokio::test]
    async fn cancelled_wakes() {
        let c = Cancel::new();
        let w = c.clone();
        let h = tokio::spawn(async move {
            w.cancelled().await;
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        c.cancel();
        tokio::time::timeout(Duration::from_secs(1), h)
            .await
            .unwrap()
            .unwrap();
    }
}
