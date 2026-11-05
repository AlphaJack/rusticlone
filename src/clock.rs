//! The current time, and waits that end early on an interrupt or a [`Wakes::wake`]. Tests use a clock that
//! moves only when waited on.

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
    #[expect(clippy::disallowed_methods, reason = "reads the system clock")]
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }

    #[expect(clippy::disallowed_methods, reason = "reads the system clock")]
    fn wait(&self, for_: Duration, stop: &dyn Fn() -> bool) {
        WAKES.sleep(for_, stop);
    }
}

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Stops every rustic call and run, on SIGINT or SIGTERM.
pub(crate) fn interrupt() {
    INTERRUPTED.store(true, Ordering::Relaxed);
    WAKES.wake();
}

pub(crate) fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::Relaxed)
}

thread_local! {
    /// The earliest time at which what was read as of now changes, which [`next_change`] collects.
    static NEXT_CHANGE: std::cell::Cell<Option<Timestamp>> = const { std::cell::Cell::new(None) };
}

/// What was read as of now changes at `at`: an age turns over, a threshold passes, a schedule fires.
pub(crate) fn changes_at(at: Timestamp) {
    NEXT_CHANGE.with(|next| next.set(Some(next.get().map_or(at, |earliest| earliest.min(at)))));
}

/// Runs `f`, and returns with what it returns the earliest time at which what it read as of now changes: `None`
/// if nothing does.
#[cfg(any(test, feature = "server"))]
pub(crate) fn next_change<T>(f: impl FnOnce() -> T) -> (T, Option<Timestamp>) {
    let outer = NEXT_CHANGE.take();
    let done = f();
    (done, NEXT_CHANGE.replace(outer))
}

/// Counts wakes, so that a wait that read the count before checking its condition misses none.
pub(crate) struct Wakes {
    count: Mutex<u64>,
    woken: Condvar,
}

/// This process's: a run was cancelled, the process is stopping, a lock was freed, or another process changed
/// the state.
pub(crate) static WAKES: Wakes = Wakes::new();

impl Wakes {
    const fn new() -> Self {
        Self { count: Mutex::new(0), woken: Condvar::new() }
    }

    /// Makes every wait on these wakes check its condition again.
    pub(crate) fn wake(&self) {
        *self.count.lock() += 1;
        self.woken.notify_all();
    }

    /// Read before checking a condition, and passed to [`Self::wait`] so that no wake is missed.
    pub(crate) fn seen(&self) -> u64 {
        *self.count.lock()
    }

    /// Waits until a wake comes after `seen` wakes.
    pub(crate) fn wait(&self, seen: u64) {
        let mut count = self.count.lock();
        self.woken.wait_while(&mut count, |n| *n == seen);
    }

    /// Waits for `for_`, or until `stop` returns true when woken.
    pub(crate) fn sleep(&self, for_: Duration, stop: &dyn Fn() -> bool) {
        let (mut count, until) = (self.count.lock(), Instant::now() + for_);
        while !stop() && !self.woken.wait_until(&mut count, until).timed_out() {}
    }
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

#[cfg(test)]
mod tests {
    use std::{sync::Arc, thread};

    use super::*;

    /// A wait ends at the first wake after the count it read, so no wake between the read and the wait is missed.
    #[test]
    fn a_wait_ends_at_the_next_wake() {
        let wakes = Arc::new(Wakes::new());
        let seen = wakes.seen();
        let (done, ended) = std::sync::mpsc::channel();
        let waiting = Arc::clone(&wakes);
        thread::spawn(move || {
            waiting.wait(seen);
            done.send(()).unwrap();
        });
        assert!(ended.recv_timeout(Duration::from_millis(100)).is_err(), "ended before a wake");
        wakes.wake();
        assert_eq!(wakes.seen(), seen + 1);
        ended.recv().unwrap();
    }

    /// A sleep lasts its time, and ends early once woken with its condition met.
    #[test]
    #[expect(clippy::disallowed_methods, reason = "reads the system clock")]
    fn a_sleep_lasts_its_time_unless_stopped() {
        let (wakes, stop) = (Arc::new(Wakes::new()), Arc::new(AtomicBool::new(false)));
        let start = Instant::now();
        wakes.sleep(Duration::from_millis(50), &|| false);
        assert!(start.elapsed() >= Duration::from_millis(50));
        System.wait(Duration::from_millis(50), &|| false);
        assert!(start.elapsed() >= Duration::from_millis(100));
        let sleeping = {
            let (wakes, stop) = (Arc::clone(&wakes), Arc::clone(&stop));
            thread::spawn(move || wakes.sleep(Duration::from_secs(3600), &|| stop.load(Ordering::Relaxed)))
        };
        stop.store(true, Ordering::Relaxed);
        wakes.wake();
        sleeping.join().unwrap();
    }
}
