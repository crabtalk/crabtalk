//! A bound on how long a guest may run.
//!
//! rvtime's interrupt check covers every way guest code can fail to terminate:
//! a guest can only run forever by looping, and unbounded recursion exhausts
//! the native stack and traps. What it needs is someone to ask, from a thread
//! that is not the one blocked inside the guest.
//!
//! One thread serves every invocation. A thread per call would cost more to
//! spawn than a whole invocation costs to run — the boundary is ~17µs — so
//! deadlines are registered in a shared list and the watchdog sleeps until the
//! earliest one, or indefinitely when there are none.

use rvtime::Interrupt;
use std::{
    sync::{
        Condvar, LazyLock, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

/// How long a guest may run before it is asked to stop.
///
/// A guest blocked in a host call cannot notice an interrupt until that call
/// returns, so an embedder's system harness has to time out well inside this.
/// The bound exists to stop non-termination, not to enforce latency, and a
/// harness doing slow but finite work should finish rather than be killed.
const TIMEOUT: Duration = Duration::from_secs(60);

/// One guest's deadline: the ticket that withdraws it, when it expires, and
/// the handle that stops it.
struct Entry {
    ticket: u64,
    at: Instant,
    interrupt: Interrupt,
}

/// Deadlines the watchdog is waiting on, keyed by a monotonic ticket so a
/// finished invocation removes its own rather than someone else's.
static PENDING: LazyLock<(Mutex<Vec<Entry>>, Condvar)> =
    LazyLock::new(|| (Mutex::new(Vec::new()), Condvar::new()));

/// Hands out tickets. Wrapping after 2^64 invocations is not a scenario.
static NEXT: AtomicU64 = AtomicU64::new(0);

/// Registers a deadline and withdraws it when dropped.
///
/// Withdrawal is the point: the overwhelming majority of invocations finish
/// long before the deadline, and a guard that forgot to deregister would leave
/// the watchdog interrupting a store that had already been dropped.
pub struct Deadline(u64);

impl Deadline {
    /// Ask for `interrupt` to be tripped if this guard is still alive in
    /// [`TIMEOUT`].
    pub fn set(interrupt: Interrupt) -> Self {
        let ticket = NEXT.fetch_add(1, Ordering::Relaxed);
        let (pending, wake) = &*PENDING;
        pending.lock().expect("watchdog deadlines").push(Entry {
            ticket,
            at: Instant::now() + TIMEOUT,
            interrupt,
        });
        // The watchdog may be parked with no deadline to wait on, or waiting
        // on one later than this.
        wake.notify_one();

        start();
        Self(ticket)
    }
}

impl Drop for Deadline {
    fn drop(&mut self) {
        let (pending, _) = &*PENDING;
        pending
            .lock()
            .expect("watchdog deadlines")
            .retain(|entry| entry.ticket != self.0);
    }
}

/// Start the watchdog once, on the first invocation that needs it. An embedder
/// that never runs a guest never gets the thread.
fn start() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        thread::Builder::new()
            .name("berm-watchdog".to_owned())
            .spawn(watch)
            .expect("spawn the berm watchdog");
    });
}

fn watch() {
    let (pending, wake) = &*PENDING;
    let mut deadlines = pending.lock().expect("watchdog deadlines");
    loop {
        let now = Instant::now();
        // Tripping an interrupt does not remove the deadline: the guest stops
        // at its next backward edge rather than immediately, and its own guard
        // is what withdraws the entry.
        let mut earliest: Option<Instant> = None;
        for entry in deadlines.iter() {
            if entry.at <= now {
                entry.interrupt.interrupt();
            } else {
                earliest = Some(earliest.map_or(entry.at, |e: Instant| e.min(entry.at)));
            }
        }

        deadlines = match earliest {
            Some(at) => {
                wake.wait_timeout(deadlines, at.saturating_duration_since(now))
                    .expect("watchdog deadlines")
                    .0
            }
            None => wake.wait(deadlines).expect("watchdog deadlines"),
        };
    }
}
