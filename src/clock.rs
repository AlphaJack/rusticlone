//! The current time, and waits that end early on an interrupt or a [`wake`]. Tests use a clock that
//! moves only when waited on.

#![expect(clippy::disallowed_methods, reason = "reads the system clock")]

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use jiff::Timestamp;
use parking_lot::{Condvar, Mutex};

pub(crate) trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
    /// Waits for `for_`, or until `stop` returns true.
    fn wait(&self, for_: Duration, stop: &dyn Fn() -> bool);
}

pub(crate) struct System;

impl Clock for System {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }

    fn wait(&self, for_: Duration, stop: &dyn Fn() -> bool) {
        sleep(for_, stop);
    }
}

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Stops every rustic call and run, on SIGINT or SIGTERM.
pub(crate) fn interrupt() {
    INTERRUPTED.store(true, Ordering::Relaxed);
    wake();
}

pub(crate) fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::Relaxed)
}

static WAKES: Mutex<u64> = Mutex::new(0);
static WOKEN: Condvar = Condvar::new();

/// Makes every [`wait`] in this process check its condition again: a run was cancelled, the process
/// is stopping, a lock was freed, or another process changed the state.
pub(crate) fn wake() {
    *WAKES.lock() += 1;
    WOKEN.notify_all();
}

/// Read before checking a condition, and passed to [`wait`] so that no wake is missed.
pub(crate) fn wakes() -> u64 {
    *WAKES.lock()
}

/// Waits until [`wake`] is called after `seen` wakes.
pub(crate) fn wait(seen: u64) {
    let mut wakes = WAKES.lock();
    WOKEN.wait_while(&mut wakes, |n| *n == seen);
}

/// Waits for `for_`, or until `stop` returns true when woken.
pub(crate) fn sleep(for_: Duration, stop: &dyn Fn() -> bool) {
    let (mut wakes, until) = (WAKES.lock(), Instant::now() + for_);
    while !stop() && !WOKEN.wait_until(&mut wakes, until).timed_out() {}
}

/// Moves only when waited on.
#[cfg(test)]
pub(crate) struct Fixed(pub Mutex<Timestamp>);

#[cfg(test)]
impl Fixed {
    pub(crate) fn at(time: &str) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self(Mutex::new(time.parse().unwrap())))
    }
}

#[cfg(test)]
impl Clock for Fixed {
    fn now(&self) -> Timestamp {
        *self.0.lock()
    }

    fn wait(&self, for_: Duration, _: &dyn Fn() -> bool) {
        let mut now = self.0.lock();
        *now = now.checked_add(for_).unwrap();
    }
}
