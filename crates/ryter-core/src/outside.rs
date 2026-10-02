//! Ryter's own record-keeping, done outside the sandbox.
//!
//! A sandbox profile shuts `~/.ryter` to the thread that runs the model's
//! commands, except for a few folders. That is what keeps a command from
//! reading a key there, or from writing "the user approved this" where
//! Ryter would believe it. But Ryter itself keeps records there too: which
//! run file the user approved, which product it left running. Granting the
//! sandbox those files would hand them to every command.
//!
//! So the sandboxed thread doesn't touch them. It hands the work to a
//! thread that was started before the sandbox was applied, and so isn't in
//! it. Only Ryter's own code can reach that thread: a command is another
//! process.

use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};

type Job = Box<dyn FnOnce() + Send>;

static CLERK: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();

thread_local! {
    /// This thread is the one that works outside the sandbox.
    static IS_CLERK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Start the thread that works outside the sandbox. Called before a
/// sandbox is applied; a thread started after one is inside it.
pub(crate) fn start() {
    CLERK.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        let _ = std::thread::Builder::new()
            .name("ryter-records".into())
            .spawn(move || {
                IS_CLERK.with(|c| c.set(true));
                for job in rx {
                    job();
                }
            });
        Mutex::new(tx)
    });
}

/// Run `job` where the sandbox doesn't reach, and wait for what it returns.
///
/// Once a sandbox has been applied anywhere in the process, every thread
/// hands its records over: a thread started by a sandboxed one is in the
/// sandbox too, and doesn't know it. Before that, the job runs in place.
/// A job that panics does so in the caller, and the thread that ran it
/// carries on.
pub(crate) fn run<T: Send + 'static>(job: impl FnOnce() -> T + Send + 'static) -> T {
    let Some(clerk) = CLERK.get().filter(|_| !IS_CLERK.with(|c| c.get())) else {
        return job();
    };
    let (tx, rx) = channel();
    // The job is kept where it can be taken back: if the thread is gone,
    // it runs here, and fails as the sandbox says it must.
    let job = std::sync::Arc::new(Mutex::new(Some(job)));
    let theirs = job.clone();
    let sent = clerk.lock().is_ok_and(|c| {
        c.send(Box::new(move || {
            if let Some(job) = theirs.lock().ok().and_then(|mut j| j.take()) {
                let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                let _ = tx.send(out);
            }
        }))
        .is_ok()
    });
    if sent {
        match rx.recv() {
            Ok(Ok(out)) => return out,
            Ok(Err(panic)) => std::panic::resume_unwind(panic),
            Err(_) => {}
        }
    }
    let here = job.lock().ok().and_then(|mut j| j.take());
    match here {
        Some(job) => job(),
        // It was taken there and no answer came back: that thread ended
        // mid-job, which only a panic outside the job does.
        None => panic!("Ryter's record-keeping thread stopped"),
    }
}
